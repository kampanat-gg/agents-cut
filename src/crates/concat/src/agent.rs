// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The agent beside the editor: a conversation with the user's own Claude
//! Code, which edits the project on screen through the window's API.
//!
//! Each turn is one `claude -p` process, given the request on stdin and
//! read as stream-json on stdout; the next turn resumes the same Claude
//! session by its id, so the conversation carries on. The process reaches
//! the editor through `concat --mcp window`, pointed at the window's API
//! server by an MCP config this writes - the Automation page's server when
//! it is on, or one started here for the agent alone. Every call it makes
//! on the open project runs in the window, through its undo history (live
//! mode, see `studio::live`).
//!
//! The process is read on a thread of its own; what it says crosses to the
//! event loop as [`Said`] and lands in [`Chat::hear`], which turns it into
//! the sidebar's rows. A stopped turn's leftovers are told apart by the
//! turn's number and dropped.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use slint::{ComponentHandle, Model, VecModel};

use crate::host::Shell;
use crate::i18n::{t, tf};
use crate::studio::Studio;
use crate::ui::{Agent, AgentRow, AgentStarter, App};

/// The tools the agent may call: the editor's, and of the machine's only
/// `codex exec`, for pictures, and reading inside the project's folder,
/// which Claude Code allows there without asking and refuses elsewhere.
const ALLOWED: &str = "mcp__concat";
/// The one shell command allowed, by Claude Code's permission rule; any
/// other, or one chained after it, is refused without asking.
const CODEX_RULE: &str = "Bash(codex exec:*)";

/// How long a turn may say nothing before the sidebar says it is waiting:
/// a first start can sit behind a system prompt (a privacy question about
/// the folder, a Keychain question) that nobody sees.
const QUIET: std::time::Duration = std::time::Duration::from_secs(20);

/// How much of a tool's input or a failure is shown under its row.
const DETAIL_CHARS: usize = 160;

/// What the agent is told about where it is and how to work, before the
/// user's words: `{path}` is the project's folder.
const BRIEF: &str = include_str!("../agent/brief.md");

/// Added to [`BRIEF`] when Codex is installed: how to make pictures, in
/// layers, cut out, and with a character kept the same. `{model}` is a
/// `-m` for Codex, or nothing.
const CODEX_BRIEF: &str = include_str!("../agent/codex.md");

/// One thing the process said, as the sidebar cares about it.
#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    /// The Claude session this turn runs in; the next turn resumes it.
    Session(String),
    /// Words to the user.
    Text(String),
    /// A tool call starting: its id, its name and what it was given.
    Tool {
        id: String,
        name: String,
        input: String,
    },
    /// A tool call's end.
    Outcome {
        id: String,
        failed: bool,
        text: String,
    },
    /// The turn's last word: an error, or none.
    Finished(Option<String>),
    /// The process has exited; whether cleanly, and what it wrote to
    /// stderr.
    Exited { ok: bool, stderr: String },
}

/// The turn running now.
struct Turn {
    child: Arc<Mutex<Child>>,
    /// The process said how the turn ended.
    finished: bool,
    /// The process has said anything at all.
    heard: bool,
}

/// The conversation, and the process behind it.
pub struct Chat {
    rows: Rc<VecModel<AgentRow>>,
    /// The open project's style's buttons, empty without a style.
    starters: Rc<VecModel<AgentStarter>>,
    /// The Claude session to resume, once the first turn has named one.
    session: Option<String>,
    turn: Option<Turn>,
    /// Counts turns, so a stopped one's late words are known as such.
    epoch: u64,
    /// The row each running tool call is drawn in, by its id.
    tools: HashMap<String, usize>,
    /// A server of the agent's own, while the Automation page has none
    /// running.
    server: Option<concat_server::Server>,
    shown: bool,
    /// Fires once a turn has been quiet for [`QUIET`].
    watch: slint::Timer,
}

impl Default for Chat {
    fn default() -> Self {
        Chat {
            rows: Rc::new(VecModel::default()),
            starters: Rc::new(VecModel::default()),
            session: None,
            turn: None,
            epoch: 0,
            tools: HashMap::new(),
            server: None,
            shown: true,
            watch: slint::Timer::default(),
        }
    }
}

impl Drop for Chat {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Chat {
    /// The rows, handed to the sidebar once.
    pub fn rows(&self) -> Rc<VecModel<AgentRow>> {
        Rc::clone(&self.rows)
    }

    /// The style buttons, handed to the sidebar once.
    pub fn starters(&self) -> Rc<VecModel<AgentStarter>> {
        Rc::clone(&self.starters)
    }

    /// Shows the buttons of the style the project at `folder` was made
    /// with, in the interface's language; none for a project without one.
    pub fn load_style(&mut self, folder: &Path) {
        let starters: Vec<AgentStarter> = crate::styles::of_project(folder)
            .map(|style| {
                style
                    .starters
                    .iter()
                    .map(|starter| AgentStarter {
                        label: starter.label().into(),
                        prompt: starter.prompt().into(),
                        fill: starter.fill,
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.starters.set_vec(starters);
    }

    pub fn busy(&self) -> bool {
        self.turn.is_some()
    }

    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }

    /// Writes what the sidebar shows that is not a row.
    pub fn publish(&self, app: &App) {
        let agent = app.global::<Agent>();
        agent.set_busy(self.busy());
        agent.set_shown(self.shown);
    }

    fn push(&mut self, role: &str, text: &str, detail: &str, state: i32) -> usize {
        self.rows.push(AgentRow {
            role: role.into(),
            text: text.into(),
            detail: detail.into(),
            state,
        });
        self.rows.row_count() - 1
    }

    /// Stops the running turn, if there is one: its process is killed and
    /// anything it still says is dropped.
    pub fn cancel(&mut self) {
        if self.stop() {
            self.push("note", &t("agent.stopped"), "", 0);
        }
    }

    fn stop(&mut self) -> bool {
        let Some(turn) = self.turn.take() else {
            return false;
        };
        if let Ok(mut child) = turn.child.lock() {
            let _ = child.kill();
        }
        self.epoch += 1;
        for row in std::mem::take(&mut self.tools).into_values() {
            self.settle(row, 2, None);
        }
        true
    }

    /// Forgets the conversation: the next request starts a new session.
    pub fn new_chat(&mut self) {
        self.stop();
        self.rows.set_vec(Vec::new());
        self.session = None;
    }

    fn settle(&mut self, row: usize, state: i32, detail: Option<&str>) {
        if let Some(mut data) = self.rows.row_data(row) {
            data.state = state;
            if let Some(detail) = detail {
                data.detail = detail.into();
            }
            self.rows.set_row_data(row, data);
        }
    }

    /// Says so when the turn `epoch` is still running and has said nothing.
    fn still_quiet(&mut self, epoch: u64) {
        if epoch == self.epoch && self.turn.as_ref().is_some_and(|turn| !turn.heard) {
            self.push("note", &t("agent.slowStart"), "", 0);
        }
    }

    /// Takes one thing the process said, if it is from the turn running.
    pub fn hear(&mut self, epoch: u64, said: Said) {
        if epoch != self.epoch {
            return;
        }
        let Some(turn) = self.turn.as_mut() else {
            return;
        };
        turn.heard = true;
        match said {
            Said::Session(id) => self.session = Some(id),
            Said::Text(text) => {
                let text = text.trim();
                if !text.is_empty() {
                    self.push("assistant", text, "", 0);
                }
            }
            Said::Tool { id, name, input } => {
                let row = self.push("tool", &tool_label(&name), &input, 0);
                self.tools.insert(id, row);
            }
            Said::Outcome { id, failed, text } => {
                if let Some(row) = self.tools.remove(&id) {
                    let detail = failed.then(|| clip(&text));
                    self.settle(row, if failed { 2 } else { 1 }, detail.as_deref());
                }
            }
            Said::Finished(error) => {
                if let Some(turn) = self.turn.as_mut() {
                    turn.finished = true;
                }
                if let Some(error) = error {
                    self.push("error", &tf("agent.failed", &[&error]), "", 0);
                }
            }
            Said::Exited { ok, stderr } => {
                let finished = self.turn.take().is_some_and(|turn| turn.finished);
                self.epoch += 1;
                for row in std::mem::take(&mut self.tools).into_values() {
                    self.settle(row, 2, None);
                }
                if !ok && !finished {
                    let why = last_line(&stderr).unwrap_or("exit");
                    self.push("error", &tf("agent.failed", &[&why]), "", 0);
                }
            }
        }
    }
}

/// Sends `text` to the agent: a turn of the conversation about the
/// project open in `studio`.
pub fn send(studio: &mut Studio, text: String) {
    let text = text.trim().to_owned();
    if text.is_empty() || studio.agent.busy() {
        return;
    }
    studio.agent.push("user", &text, "", 0);
    if let Err(problem) = start(studio, &text) {
        studio.agent.push("error", &problem, "", 0);
    }
}

fn start(studio: &mut Studio, text: &str) -> Result<(), String> {
    let project = studio
        .session
        .as_ref()
        .map(|session| session.path().to_owned())
        .ok_or_else(|| tf("agent.couldNotStart", &[&"no project is open"]))?;
    let claude = find("claude").ok_or_else(|| t("agent.notFound"))?;
    let (address, token) = reach(studio).map_err(|why| tf("agent.noServer", &[&why]))?;
    let config = write_mcp_config(&studio.host.dirs.config, &address, &token)
        .map_err(|why| tf("agent.couldNotStart", &[&why]))?;

    let mut brief = BRIEF.replace("{path}", &project);
    let mut command = Command::new(&claude);
    command
        .args(["-p", "--output-format", "stream-json", "--verbose"])
        .arg("--strict-mcp-config")
        .arg("--mcp-config")
        .arg(&config);
    if find("codex").is_some() {
        // A model named for Codex, where the user's own default is one
        // their sign-in cannot use.
        let model = std::env::var("CONCAT_CODEX_MODEL")
            .ok()
            .filter(|model| !model.trim().is_empty())
            .map(|model| format!(" -m {}", model.trim()))
            .unwrap_or_default();
        brief.push('\n');
        brief.push_str(
            &CODEX_BRIEF
                .replace("{path}", &project)
                .replace("{model}", &model),
        );
        command
            .args(["--tools", "Bash,Read,Glob"])
            .args(["--allowedTools", ALLOWED, CODEX_RULE]);
    } else {
        command
            .args(["--tools", "Read,Glob"])
            .args(["--allowedTools", ALLOWED]);
    }
    command.arg("--append-system-prompt").arg(brief);
    if let Some(session) = &studio.agent.session {
        command.arg("--resume").arg(session);
    }
    command
        .current_dir(&project)
        .env("PATH", search_path(&claude))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| tf("agent.couldNotStart", &[&error]))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|error| tf("agent.couldNotStart", &[&error]))?;
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));

    let chat = &mut studio.agent;
    chat.epoch += 1;
    let epoch = chat.epoch;
    chat.turn = Some(Turn {
        child: Arc::clone(&child),
        finished: false,
        heard: false,
    });
    chat.watch
        .start(slint::TimerMode::SingleShot, QUIET, move || {
            Shell::with(|shell, app| {
                let mut studio = shell.studio.borrow_mut();
                studio.agent.still_quiet(epoch);
                studio.agent.publish(&app);
            });
        });
    log::info!("agent: turn {epoch} started");

    std::thread::Builder::new()
        .name("agent".to_owned())
        .spawn(move || listen(epoch, child, stdout, stderr))
        .map_err(|error| tf("agent.couldNotStart", &[&error]))?;
    Ok(())
}

/// Reads one turn's process to its end, handing each thing it says to
/// the event loop.
fn listen(
    epoch: u64,
    child: Arc<Mutex<Child>>,
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
) {
    // Read beside stdout, so a process with a lot to say on stderr does
    // not stall on a full pipe.
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_string(&mut text);
        }
        text
    });
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            for said in parse(&line) {
                deliver(epoch, said);
            }
        }
    }
    let stderr = errors.join().unwrap_or_default();
    let ok = child
        .lock()
        .ok()
        .and_then(|mut child| child.wait().ok())
        .is_some_and(|status| status.success());
    if !stderr.trim().is_empty() {
        log::warn!("agent: {}", stderr.trim());
    }
    deliver(epoch, Said::Exited { ok, stderr });
}

fn deliver(epoch: u64, said: Said) {
    let _ = slint::invoke_from_event_loop(move || {
        Shell::with(|shell, app| {
            let mut studio = shell.studio.borrow_mut();
            studio.agent.hear(epoch, said);
            studio.agent.publish(&app);
        });
    });
}

/// What one line of stream-json says. Lines of other kinds - rate limits,
/// summaries - say nothing the sidebar shows.
pub fn parse(line: &str) -> Vec<Said> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let content = || {
        value["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    match value["type"].as_str() {
        Some("system") if value["subtype"] == "init" => value["session_id"]
            .as_str()
            .map(|id| vec![Said::Session(id.to_owned())])
            .unwrap_or_default(),
        Some("assistant") => content()
            .iter()
            .filter_map(|block| match block["type"].as_str() {
                Some("text") => Some(Said::Text(block["text"].as_str()?.to_owned())),
                Some("tool_use") => {
                    let name = block["name"].as_str()?;
                    let input = &block["input"];
                    // A shell call is named by its program and shown by
                    // what it asks for, not by the command line.
                    let (name, input) = match input["command"].as_str() {
                        Some(command) if name == "Bash" => (
                            command.split_whitespace().next().unwrap_or(name).to_owned(),
                            clip(
                                input["description"]
                                    .as_str()
                                    .or_else(|| quoted(command))
                                    .unwrap_or(command),
                            ),
                        ),
                        _ => (name.to_owned(), summary(input)),
                    };
                    Some(Said::Tool {
                        id: block["id"].as_str()?.to_owned(),
                        name,
                        input,
                    })
                }
                _ => None,
            })
            .collect(),
        Some("user") => content()
            .iter()
            .filter(|block| block["type"] == "tool_result")
            .filter_map(|block| {
                Some(Said::Outcome {
                    id: block["tool_use_id"].as_str()?.to_owned(),
                    failed: block["is_error"].as_bool().unwrap_or(false),
                    text: result_text(&block["content"]),
                })
            })
            .collect(),
        Some("result") => {
            let failed = value["is_error"].as_bool().unwrap_or(false);
            let error = failed.then(|| {
                value["result"]
                    .as_str()
                    .or(value["subtype"].as_str())
                    .unwrap_or("error")
                    .to_owned()
            });
            let mut said = Vec::new();
            if let Some(id) = value["session_id"].as_str() {
                said.push(Said::Session(id.to_owned()));
            }
            said.push(Said::Finished(error));
            said
        }
        _ => Vec::new(),
    }
}

/// A tool's input as one short line, without the project's folder that
/// every call repeats.
fn summary(input: &Value) -> String {
    let mut input = input.clone();
    if let Some(fields) = input.as_object_mut() {
        fields.remove("path");
        if fields.is_empty() {
            return String::new();
        }
    }
    clip(&input.to_string())
}

/// A tool result's words: a string, or the text blocks of a list.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The first double-quoted part of a command line: what a `codex exec`
/// was asked to do.
fn quoted(command: &str) -> Option<&str> {
    let start = command.find('"')? + 1;
    let length = command[start..].find('"')?;
    Some(&command[start..start + length])
}

/// `mcp__concat__clip_trim` as "clip trim".
fn tool_label(name: &str) -> String {
    name.rsplit("__").next().unwrap_or(name).replace('_', " ")
}

fn clip(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(DETAIL_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

fn last_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).rfind(|line| !line.is_empty())
}

/// The window's API server for the agent to reach, and its token: the
/// Automation page's when it runs, or one of the agent's own, started on
/// a free loopback port the first time it is needed.
fn reach(studio: &mut Studio) -> Result<(String, String), String> {
    if let Some(server) = &studio.host.server
        && let Some(address) = server.json_addr()
    {
        return Ok((address.to_string(), server.token().to_owned()));
    }
    if studio.agent.server.is_none() {
        let exporter = studio.host.exporter.clone();
        let open_projects = studio.host.open_projects.clone();
        let config = concat_server::Config {
            json: Some(std::net::SocketAddr::from(([127, 0, 0, 1], 0))),
            ..concat_server::Config::default()
        };
        let server = concat_server::Server::start(config, move |events| {
            let mut api = concat_api::Api::new(events)?;
            api.share_exporter(exporter);
            api.share_open_projects(open_projects);
            api.go_live(Arc::new(crate::studio::live::Window));
            Ok(api)
        })?;
        studio.agent.server = Some(server);
    }
    let server = studio.agent.server.as_ref().ok_or("no server")?;
    let address = server.json_addr().ok_or("the server has no address")?;
    Ok((address.to_string(), server.token().to_owned()))
}

/// Writes the MCP config that points `concat --mcp window` at the server,
/// readable by the user alone since it holds the token.
fn write_mcp_config(folder: &Path, address: &str, token: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let config = serde_json::json!({
        "mcpServers": {
            "concat": {
                "command": exe,
                "args": ["--mcp", "window"],
                "env": {
                    "CONCAT_API_ADDRESS": address,
                    "CONCAT_API_TOKEN": token,
                },
            },
        },
    });
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let path = folder.join("agent-mcp.json");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&path).map_err(|error| error.to_string())?;
    file.write_all(config.to_string().as_bytes())
        .map_err(|error| error.to_string())?;
    Ok(path)
}

/// The folders a program is looked for in: the PATH this process was
/// given, then where installers put `claude` and `codex`, since an app
/// opened from the Finder is not given the shell's PATH.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
        // nvm's Node versions, newest first: where `npm i -g` put codex.
        let mut versions: Vec<PathBuf> = std::fs::read_dir(home.join(".nvm/versions/node"))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path().join("bin"))
                    .collect()
            })
            .unwrap_or_default();
        versions.sort();
        dirs.extend(versions.into_iter().rev());
    }
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    dirs
}

/// The program `name`, or for `claude` the one `CONCAT_CLAUDE` names.
fn find(name: &str) -> Option<PathBuf> {
    if name == "claude"
        && let Some(named) = std::env::var_os("CONCAT_CLAUDE").map(PathBuf::from)
    {
        return named.is_file().then_some(named);
    }
    search_dirs()
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The PATH the process runs with: its own folder first, for an install
/// that is a script needing what sits beside it.
fn search_path(claude: &Path) -> std::ffi::OsString {
    let mut dirs = search_dirs();
    if let Some(folder) = claude.parent() {
        dirs.insert(0, folder.to_path_buf());
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stream_reads_as_the_sidebar_needs_it() {
        assert_eq!(
            parse(r#"{"type":"system","subtype":"init","session_id":"s1"}"#),
            vec![Said::Session("s1".to_owned())]
        );
        assert_eq!(
            parse(
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."},{"type":"tool_use","id":"t1","name":"mcp__concat__clip_trim","input":{"path":"/p","clipId":"c1","end":4}}]}}"#
            ),
            vec![
                Said::Text("Done.".to_owned()),
                Said::Tool {
                    id: "t1".to_owned(),
                    name: "mcp__concat__clip_trim".to_owned(),
                    input: r#"{"clipId":"c1","end":4}"#.to_owned(),
                },
            ]
        );
        assert_eq!(
            parse(
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"no clip c1"}]}]}}"#
            ),
            vec![Said::Outcome {
                id: "t1".to_owned(),
                failed: true,
                text: "no clip c1".to_owned(),
            }]
        );
        assert_eq!(
            parse(r#"{"session_id":"s1","type":"result","is_error":false,"result":"Done."}"#),
            vec![Said::Session("s1".to_owned()), Said::Finished(None)]
        );
        assert_eq!(
            parse(
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"codex exec -C '/p' \"Generate an image: a red sky\""}}]}}"#
            ),
            vec![Said::Tool {
                id: "t2".to_owned(),
                name: "codex".to_owned(),
                input: "Generate an image: a red sky".to_owned(),
            }]
        );
        assert!(parse(r#"{"type":"rate_limit_event"}"#).is_empty());
        assert!(parse("not json").is_empty());
    }

    #[test]
    fn a_tool_is_named_in_words() {
        assert_eq!(tool_label("mcp__concat__clip_trim"), "clip trim");
        assert_eq!(tool_label("captions_add"), "captions add");
    }

    #[test]
    fn a_long_detail_is_cut() {
        let long = "x".repeat(DETAIL_CHARS + 20);
        assert_eq!(clip(&long).chars().count(), DETAIL_CHARS + 1);
        assert_eq!(clip("short"), "short");
    }
}
