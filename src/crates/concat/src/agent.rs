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
//!
//! Tool calls are not rows of their own: the calls between two things the
//! agent says are one block of steps, each named in words ([`describe`])
//! and timed, under a header that says what is happening now and for how
//! long, folding to a one-line summary once the agent speaks again.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use slint::{ComponentHandle, Model, VecModel};

use crate::host::Shell;
use crate::i18n::{t, tf};
use crate::studio::Studio;
use crate::ui::{Agent, AgentRow, AgentStarter, AgentStep, App};

/// The tools the agent may call: the editor's, and of the machine's only
/// `codex exec`, for pictures, and reading inside the project's folder,
/// which Claude Code allows there without asking and refuses elsewhere.
const ALLOWED: &str = "mcp__concat";
/// The one shell command allowed, by Claude Code's permission rule; any
/// other, or one chained after it, is refused without asking.
const CODEX_RULE: &str = "Bash(codex exec:*)";

/// Claude Code's own tools the agent has, past the shell: reading and
/// searching, writing (inside the project, by the rules in `start`), and
/// the web.
const TOOLS: &str = "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch";

/// The MCP servers of the user's own Claude Code that give voices - an
/// ElevenLabs connector on claude.ai, or one added by hand - by the name
/// their tools carry. Read once in the background (`claude mcp list` is a
/// few seconds of health checks), and empty until then.
static VOICE_SERVERS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Starts reading which voice servers the user's Claude Code has.
pub fn discover_servers() {
    let Some(claude) = find("claude") else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("agent-servers".to_owned())
        .spawn(move || {
            let listed = Command::new(&claude)
                .args(["mcp", "list"])
                .env("PATH", search_path(&claude))
                .stdin(Stdio::null())
                .output();
            let Ok(listed) = listed else {
                return;
            };
            let found = voice_servers_in(&String::from_utf8_lossy(&listed.stdout));
            log::info!("agent: voice servers {found:?}");
            if let Ok(mut servers) = VOICE_SERVERS.lock() {
                *servers = found;
            }
        });
}

fn voice_servers() -> Vec<String> {
    VOICE_SERVERS
        .lock()
        .map(|servers| servers.clone())
        .unwrap_or_default()
}

/// The voice servers in `claude mcp list`'s output, named as their tools
/// are: "claude.ai ElevenLabs (2): https://... - ✔ Connected" is
/// `claude_ai_ElevenLabs_2`. One that needs signing in is left out.
fn voice_servers_in(listing: &str) -> Vec<String> {
    listing
        .lines()
        .filter(|line| line.to_lowercase().contains("elevenlabs"))
        .filter(|line| !line.contains("Needs authentication"))
        .filter_map(|line| line.split_once(": ").map(|(name, _)| name))
        .map(|name| {
            let mut tool = String::new();
            for c in name.chars() {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    tool.push(c);
                } else if !tool.ends_with('_') {
                    tool.push('_');
                }
            }
            tool.trim_matches('_').to_owned()
        })
        .collect()
}

/// How long a turn may say nothing before the sidebar says it is waiting:
/// a first start can sit behind a system prompt (a privacy question about
/// the folder, a Keychain question) that nobody sees.
const QUIET: std::time::Duration = std::time::Duration::from_secs(20);

/// The models the sidebar offers: `claude --model`'s alias, and the name
/// the chip shows.
pub const MODELS: [(&str, &str); 4] = [
    ("fable", "Fable 5.1"),
    ("opus", "Opus 5.5"),
    ("sonnet", "Sonnet 5.5"),
    ("haiku", "Haiku 5.5"),
];

/// The efforts it offers, as `claude --effort` takes them; none is the
/// default.
pub const EFFORTS: [&str; 6] = ["", "low", "medium", "high", "xhigh", "max"];

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
        input: Value,
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

/// One tool call, as its block shows it.
struct Step {
    label: String,
    detail: String,
    /// Who does it; see [`brand_of`].
    brand: &'static str,
    /// 0 running, 1 done, 2 failed: [`AgentStep::state`].
    state: i32,
    began: Instant,
    took: Option<Duration>,
}

/// The tool calls between two things the agent said.
struct Group {
    /// The row it is drawn in.
    row: usize,
    steps: Vec<Step>,
    began: Instant,
    /// Steps are still being added: the agent has not spoken since.
    live: bool,
    open: bool,
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
    /// Every block of steps, in the order they appear.
    groups: Vec<Group>,
    /// Where each running tool call is drawn: its block and its step.
    tools: HashMap<String, (usize, usize)>,
    /// Redraws the open block's clock once a second while a turn runs.
    tick: slint::Timer,
    /// A server of the agent's own, while the Automation page has none
    /// running.
    server: Option<concat_server::Server>,
    shown: bool,
    /// Indexes into [`MODELS`] and [`EFFORTS`].
    model: usize,
    /// The model was picked here rather than read from Claude Code's own
    /// settings, so a turn names it; one read there is left to Claude
    /// Code, which knows it more exactly - `opus[1m]` is more than `opus`.
    picked: bool,
    effort: usize,
    /// Words for the box, from files attached, and a count that tells the
    /// box there is more.
    attached: String,
    attached_token: i32,
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
            groups: Vec::new(),
            tools: HashMap::new(),
            tick: slint::Timer::default(),
            server: None,
            shown: true,
            model: 1,
            picked: false,
            effort: 0,
            attached: String::new(),
            attached_token: 0,
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
        let style = crate::styles::of_project(folder);
        let starters: Vec<AgentStarter> = style
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

    /// Takes the model and effort the preferences remember; with no model
    /// remembered, the one Claude Code itself is set to.
    pub fn remember(&mut self, model: Option<&str>, effort: Option<&str>) {
        let effort = effort.unwrap_or_default();
        let remembered =
            model.and_then(|model| MODELS.iter().position(|(alias, _)| *alias == model));
        self.picked = remembered.is_some();
        self.model = remembered.unwrap_or_else(own_model);
        self.effort = EFFORTS
            .iter()
            .position(|level| *level == effort)
            .unwrap_or(0);
    }

    /// Picks a model by its index in [`MODELS`]; its alias, for the
    /// preferences.
    pub fn choose_model(&mut self, index: usize) -> Option<String> {
        self.model = index.min(MODELS.len() - 1);
        self.picked = true;
        Some(MODELS[self.model].0.to_owned())
    }

    /// Picks an effort by its index in [`EFFORTS`]; as for the model.
    pub fn choose_effort(&mut self, index: usize) -> Option<String> {
        self.effort = index.min(EFFORTS.len() - 1);
        Some(EFFORTS[self.effort].to_owned()).filter(|level| !level.is_empty())
    }

    /// Puts `words` in the box, after whatever is typed there, and opens
    /// the sidebar if it is folded away.
    pub fn mention(&mut self, words: &str) {
        self.attached = format!("{words} ");
        self.attached_token += 1;
        self.shown = true;
    }

    /// Writes what the sidebar shows that is not a row.
    pub fn publish(&self, app: &App) {
        let agent = app.global::<Agent>();
        agent.set_busy(self.busy());
        agent.set_shown(self.shown);
        agent.set_model(self.model as i32);
        agent.set_effort(self.effort as i32);
        if agent.get_attached_token() != self.attached_token {
            agent.set_attached_text(self.attached.as_str().into());
            agent.set_attached_token(self.attached_token);
        }
    }

    /// The labels of the model and effort lists, in the interface's
    /// language, handed to the sidebar once.
    pub fn choices() -> (Vec<slint::SharedString>, Vec<slint::SharedString>) {
        let models = MODELS.iter().map(|(_, label)| (*label).into()).collect();
        let efforts = EFFORTS
            .iter()
            .map(|level| {
                match *level {
                    "low" => t("agent.effort.low"),
                    "medium" => t("agent.effort.medium"),
                    "high" => t("agent.effort.high"),
                    "xhigh" => t("agent.effort.xhigh"),
                    "max" => t("agent.effort.max"),
                    _ => t("agent.effort.default"),
                }
                .into()
            })
            .collect();
        (models, efforts)
    }

    fn push(&mut self, role: &str, text: &str, detail: &str, state: i32) -> usize {
        self.rows.push(AgentRow {
            role: role.into(),
            text: text.into(),
            detail: detail.into(),
            state,
            steps: Default::default(),
            brand: Default::default(),
            open: false,
            elapsed: Default::default(),
        });
        self.rows.row_count() - 1
    }

    /// The block steps go into now: the last one while the agent has not
    /// spoken since, else a new one.
    fn live_group(&mut self) -> usize {
        if let Some(last) = self.groups.len().checked_sub(1)
            && self.groups[last].live
        {
            return last;
        }
        let row = self.push("steps", "", "", 0);
        self.groups.push(Group {
            row,
            steps: Vec::new(),
            began: Instant::now(),
            live: true,
            open: true,
        });
        self.groups.len() - 1
    }

    /// Folds the live block to its summary: the agent has spoken, or the
    /// turn is over. Calls still running then are failed ones.
    fn close_group(&mut self, fail_running: bool) {
        if let Some(index) = self.groups.iter().rposition(|group| group.live) {
            let group = &mut self.groups[index];
            group.live = false;
            group.open = false;
            if fail_running {
                for step in group.steps.iter_mut().filter(|step| step.state == 0) {
                    step.state = 2;
                    step.took = Some(step.began.elapsed());
                }
            }
            self.draw_group(index);
        }
    }

    /// Writes block `index` into its row.
    fn draw_group(&mut self, index: usize) {
        let Some(group) = self.groups.get(index) else {
            return;
        };
        let running = group.steps.iter().rposition(|step| step.state == 0);
        let failed = group.steps.iter().any(|step| step.state == 2);
        let span = if group.live {
            group.began.elapsed()
        } else {
            group
                .steps
                .iter()
                .map(|step| (step.began - group.began) + step.took.unwrap_or_default())
                .max()
                .unwrap_or_default()
        };
        let brand = running
            .filter(|_| group.live)
            .map(|step| group.steps[step].brand)
            .unwrap_or_default();
        let title = match running {
            Some(step) if group.live => group.steps[step].label.clone(),
            _ if group.live => group
                .steps
                .last()
                .map(|step| step.label.clone())
                .unwrap_or_default(),
            _ if group.steps.len() == 1 => t("agent.stepDone"),
            _ => tf("agent.stepsDone", &[&group.steps.len()]),
        };
        let last = group.steps.len().saturating_sub(1);
        let steps: Vec<AgentStep> = group
            .steps
            .iter()
            .enumerate()
            .map(|(at, step)| AgentStep {
                label: step.label.as_str().into(),
                detail: step.detail.as_str().into(),
                brand: step.brand.into(),
                state: step.state,
                time: step.took.map(seconds).unwrap_or_default().into(),
                last: at == last,
            })
            .collect();
        let row = AgentRow {
            role: "steps".into(),
            text: title.into(),
            detail: Default::default(),
            state: if group.live && self.turn.is_some() {
                0
            } else if failed {
                2
            } else {
                1
            },
            steps: slint::ModelRc::from(Rc::new(VecModel::from(steps))),
            brand: brand.into(),
            open: group.open,
            elapsed: duration(span).into(),
        };
        let at = group.row;
        if at < self.rows.row_count() {
            self.rows.set_row_data(at, row);
        }
    }

    /// Opens or folds the block drawn in row `row`.
    pub fn toggle(&mut self, row: usize) {
        if let Some(index) = self.groups.iter().position(|group| group.row == row) {
            self.groups[index].open = !self.groups[index].open;
            self.draw_group(index);
        }
    }

    /// Redraws the live block, for its clock.
    fn refresh(&mut self) {
        if let Some(index) = self.groups.iter().rposition(|group| group.live) {
            self.draw_group(index);
        }
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
        self.tick.stop();
        self.tools.clear();
        self.close_group(true);
        true
    }

    /// Forgets the conversation: the next request starts a new session.
    pub fn new_chat(&mut self) {
        self.stop();
        self.rows.set_vec(Vec::new());
        self.groups.clear();
        self.session = None;
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
                    self.close_group(false);
                    self.push("assistant", text, "", 0);
                }
            }
            Said::Tool { id, name, input } => {
                let (label, detail) = describe(&name, &input);
                let index = self.live_group();
                let group = &mut self.groups[index];
                group.steps.push(Step {
                    label,
                    detail,
                    brand: brand_of(&name),
                    state: 0,
                    began: Instant::now(),
                    took: None,
                });
                self.tools.insert(id, (index, group.steps.len() - 1));
                self.draw_group(index);
            }
            Said::Outcome { id, failed, text } => {
                if let Some((index, at)) = self.tools.remove(&id)
                    && let Some(step) = self
                        .groups
                        .get_mut(index)
                        .and_then(|group| group.steps.get_mut(at))
                {
                    step.state = if failed { 2 } else { 1 };
                    step.took = Some(step.began.elapsed());
                    if failed {
                        step.detail = clip(&text);
                    }
                    self.draw_group(index);
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
                self.tick.stop();
                self.tools.clear();
                self.close_group(true);
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
    // Not strict: the user's own servers and claude.ai connectors load as
    // well, which is how ElevenLabs reaches the agent with no key to paste.
    // Only the ones allowed below can run; a `-p` turn refuses the rest
    // without asking.
    command
        .args(["-p", "--output-format", "stream-json", "--verbose"])
        .arg("--mcp-config")
        .arg(&config);
    // Reading anywhere, writing only inside the project's folder - a
    // script, captions, notes for the channel - and the web for research.
    let inside = format!("/{}/**", project.trim_end_matches('/'));
    let mut allowed: Vec<String> = vec![
        ALLOWED.to_owned(),
        "Read".to_owned(),
        "Glob".to_owned(),
        "Grep".to_owned(),
        "WebSearch".to_owned(),
        "WebFetch".to_owned(),
        format!("Edit({inside})"),
        format!("Write({inside})"),
    ];
    allowed.extend(
        voice_servers()
            .into_iter()
            .map(|server| format!("mcp__{server}")),
    );
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
        allowed.push(CODEX_RULE.to_owned());
        command.args(["--tools", &format!("Bash,{TOOLS}")]);
    } else {
        command.args(["--tools", TOOLS]);
    }
    command.arg("--allowedTools").args(&allowed);
    command.arg("--append-system-prompt").arg(brief);
    let (model, effort) = (MODELS[studio.agent.model].0, EFFORTS[studio.agent.effort]);
    if studio.agent.picked {
        command.args(["--model", model]);
    }
    if !effort.is_empty() {
        command.args(["--effort", effort]);
    }
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
    chat.tick
        .start(slint::TimerMode::Repeated, Duration::from_secs(1), || {
            Shell::with(|shell, _| shell.studio.borrow_mut().agent.refresh());
        });
    log::info!("agent: turn {epoch} started");

    std::thread::Builder::new()
        .name("agent".to_owned())
        .spawn(move || listen(epoch, child, stdout, stderr))
        .map_err(|error| tf("agent.couldNotStart", &[&error]))?;
    Ok(())
}

/// Files the user attached: copied into the project's `assets/` (a file
/// already inside the project is used where it is) and named in the box
/// as `@assets/<name>`, which the brief tells the agent how to read.
/// The model Claude Code runs when not told one, as an index into
/// [`MODELS`]: `ANTHROPIC_MODEL`, else the `model` of its settings file -
/// "opus[1m]", "claude-sonnet-5-5" - by the family it names; Opus when
/// neither says.
fn own_model() -> usize {
    let settings = || {
        let folder = std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))?;
        let text = std::fs::read_to_string(folder.join("settings.json")).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        value.get("model")?.as_str().map(str::to_owned)
    };
    let named = std::env::var("ANTHROPIC_MODEL").ok().or_else(settings);
    family(named.as_deref().unwrap_or_default())
}

/// The model a name means, by the family in it.
fn family(name: &str) -> usize {
    let name = name.to_lowercase();
    MODELS
        .iter()
        .position(|(alias, _)| name.contains(alias))
        .unwrap_or(1)
}

pub fn attach(studio: &mut Studio, files: Vec<PathBuf>) {
    let Some(project) = studio
        .session
        .as_ref()
        .map(|session| PathBuf::from(session.path()))
    else {
        return;
    };
    let assets = project.join("assets");
    let mut named = Vec::new();
    for file in files {
        let inside = file.canonicalize().ok().and_then(|file| {
            let project = project.canonicalize().ok()?;
            file.strip_prefix(&project).ok().map(Path::to_path_buf)
        });
        let relative = match inside {
            Some(relative) => relative,
            None => {
                let Some(name) = file.file_name() else {
                    continue;
                };
                let _ = std::fs::create_dir_all(&assets);
                let mut target = assets.join(name);
                let mut n = 2;
                while target.exists() {
                    let stem = Path::new(name)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy();
                    let ext = Path::new(name)
                        .extension()
                        .map(|ext| format!(".{}", ext.to_string_lossy()))
                        .unwrap_or_default();
                    target = assets.join(format!("{stem} {n}{ext}"));
                    n += 1;
                }
                if let Err(error) = std::fs::copy(&file, &target) {
                    studio.notify(&tf("agent.couldNotAttach", &[&error]), true);
                    continue;
                }
                target
                    .strip_prefix(&project)
                    .map(Path::to_path_buf)
                    .unwrap_or(target)
            }
        };
        named.push(format!("@{}", relative.to_string_lossy()));
    }
    if !named.is_empty() {
        studio.agent.attached = format!("{} ", named.join(" "));
        studio.agent.attached_token += 1;
    }
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
                Some("tool_use") => Some(Said::Tool {
                    id: block["id"].as_str()?.to_owned(),
                    name: block["name"].as_str()?.to_owned(),
                    input: block["input"].clone(),
                }),
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

/// A tool call in words, for its step: what it does, and under it what
/// it does that to. The editor's tools by what they are for, a Codex call
/// by the picture it was asked for, a read by the file's name.
/// Who a tool call is the work of, by its name: Codex for the shell
/// (the one command the agent may run is `codex exec`), ElevenLabs for
/// its connector's tools, Claude for its own reading, writing and
/// searching, and no one for the editor's own tools.
pub fn brand_of(name: &str) -> &'static str {
    if name.to_lowercase().contains("elevenlabs") {
        "elevenlabs"
    } else if name == "Bash" {
        "codex"
    } else if name.starts_with("mcp__") {
        ""
    } else {
        "claude"
    }
}

pub fn describe(name: &str, input: &Value) -> (String, String) {
    let tool = name.rsplit("__").next().unwrap_or(name);
    let text = |key: &str| input[key].as_str().unwrap_or_default();
    let file_name = |key: &str| {
        Path::new(text(key))
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    match tool {
        "Bash" => {
            let command = text("command");
            let asked = quoted(command).unwrap_or(command);
            if asked.contains("ffmpeg") || asked.to_lowercase().contains("join") {
                (t("agent.step.join"), String::new())
            } else {
                // The picture itself: what follows "Generate an image:",
                // up to where the prompt turns to saving it.
                let picture = asked
                    .split_once("Generate an image:")
                    .map_or(asked, |(_, rest)| rest);
                let picture = picture
                    .split_once(" Save it as")
                    .map_or(picture, |(before, _)| before);
                (t("agent.step.draw"), clip(picture))
            }
        }
        "Read" => (
            tf("agent.step.look", &[&file_name("file_path")]),
            String::new(),
        ),
        "Glob" | "Grep" => (t("agent.step.browse"), String::new()),
        "Write" | "Edit" => (
            tf("agent.step.write", &[&file_name("file_path")]),
            String::new(),
        ),
        "WebSearch" => (t("agent.step.search"), clip(text("query"))),
        "WebFetch" => (t("agent.step.fetch"), clip(text("url"))),
        "media_fetch" => (
            tf(
                "agent.step.import",
                &[&Path::new(
                    input["name"]
                        .as_str()
                        .or_else(|| text("url").rsplit('/').next())
                        .unwrap_or_default(),
                )
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()],
            ),
            String::new(),
        ),
        // ElevenLabs' creative tools: what kind of sound is being made.
        "creative_generate_speech" => (t("agent.step.voice"), clip(text("prompt"))),
        "creative_generate_in_flow" => match text("node_type") {
            "music" => (t("agent.step.music"), clip(text("prompt"))),
            "sfx" => (t("agent.step.sfx"), clip(text("prompt"))),
            "speech-to-text" => (t("agent.step.transcribe"), String::new()),
            "tts" => (t("agent.step.voice"), clip(text("prompt"))),
            _ => (t("agent.step.elevenlabs"), clip(text("prompt"))),
        },
        "creative_get_flow_run_status" | "creative_get_flow" => {
            (t("agent.step.waiting"), String::new())
        }
        "creative_list_voices" => (t("agent.step.voices"), String::new()),
        "project_open" => (t("agent.step.open"), String::new()),
        "project_get" | "timeline_read" => (t("agent.step.read"), String::new()),
        "media_import" => (
            tf("agent.step.import", &[&file_name("file")]),
            String::new(),
        ),
        "clip_add" | "clips_add_sequence" => (t("agent.step.place"), String::new()),
        "clip_trim" | "clip_speed" => (t("agent.step.trim"), String::new()),
        "clip_transform" | "clip_update" | "clip_frame" | "clip_mask" | "clips_move" => {
            (t("agent.step.adjust"), String::new())
        }
        "keyframe_set" | "keyframe_clear" | "effect_key_set" => {
            (t("agent.step.animate"), String::new())
        }
        "title_add" | "title_style_words" => (t("agent.step.title"), clip(text("text"))),
        "captions_add" => (t("agent.step.captions"), String::new()),
        "preview_frame" => {
            let at = input["time"].as_f64().unwrap_or_default();
            (tf("agent.step.check", &[&seconds_of(at)]), String::new())
        }
        "edit_undo" | "edit_redo" => (t("agent.step.undo"), String::new()),
        "effects_list" | "catalogue_list" => (t("agent.step.effects"), String::new()),
        "export_start" | "export_wait" | "export_status" => (t("agent.step.export"), String::new()),
        "edit_apply" => describe_edit(&input["command"]),
        other => (other.replace('_', " "), String::new()),
    }
}

/// An `edit_apply` command in words: one edit by what it does, a batch by
/// how many changes and of which kinds.
fn describe_edit(command: &Value) -> (String, String) {
    let ops: Vec<&Value> = match command["op"].as_str() {
        Some("batch") => command["commands"]
            .as_array()
            .map(|commands| commands.iter().collect())
            .unwrap_or_default(),
        _ => vec![command],
    };
    if let [only] = ops[..] {
        match only["op"].as_str() {
            Some("addTimeline" | "renameTimeline") => {
                return (
                    t("agent.step.episode"),
                    clip(only["name"].as_str().unwrap_or_default()),
                );
            }
            Some("addLayerClip") => {
                let effect = only["effectId"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches("concat.")
                    .replace('-', " ");
                return (tf("agent.step.layer", &[&effect]), String::new());
            }
            _ => {}
        }
    }
    // Each kind once, in the order first used, with how many there were.
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for op in &ops {
        let kind = words_of(op["op"].as_str().unwrap_or("edit"));
        match kinds.iter_mut().find(|(seen, _)| *seen == kind) {
            Some((_, count)) => *count += 1,
            None => kinds.push((kind, 1)),
        }
    }
    let detail = kinds
        .iter()
        .map(|(kind, count)| {
            if *count > 1 {
                format!("{kind} ×{count}")
            } else {
                kind.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    (tf("agent.step.edit", &[&ops.len()]), clip(&detail))
}

/// `setClipKey` as "set clip key".
fn words_of(op: &str) -> String {
    let mut words = String::new();
    for character in op.chars() {
        if character.is_uppercase() && !words.is_empty() {
            words.push(' ');
        }
        words.extend(character.to_lowercase());
    }
    words
}

/// A step's time: "5.1s", or "1m 05s" past a minute.
fn seconds(took: Duration) -> String {
    let secs = took.as_secs_f64();
    if secs < 60.0 {
        tf("agent.seconds", &[&format!("{secs:.1}")])
    } else {
        duration(took)
    }
}

/// A block's time, whole seconds: "11s", "2m 14s".
fn duration(span: Duration) -> String {
    let secs = span.as_secs();
    if secs < 60 {
        tf("agent.seconds", &[&secs])
    } else {
        tf(
            "agent.minutes",
            &[&(secs / 60), &format!("{:02}", secs % 60)],
        )
    }
}

/// A time on the timeline in words: "26s", "1m 05s".
fn seconds_of(at: f64) -> String {
    duration(Duration::from_secs_f64(at.max(0.0)))
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
    fn each_step_names_who_does_it() {
        assert_eq!(brand_of("Bash"), "codex");
        assert_eq!(brand_of("Read"), "claude");
        assert_eq!(brand_of("WebSearch"), "claude");
        assert_eq!(brand_of("mcp__concat__clip_add"), "");
        assert_eq!(
            brand_of("mcp__claude_ai_ElevenLabs_2__creative_generate_speech"),
            "elevenlabs"
        );
    }

    #[test]
    fn voice_servers_are_named_as_their_tools_are() {
        let listing = "claude.ai Claude Docs: https://api.anthropic.com/v1/pages/mcp - ✔ Connected\n\
            claude.ai ElevenLabs: https://api.elevenlabs.io/v1/mcp-claude-marketplace - ! Needs authentication\n\
            claude.ai ElevenLabs (2): https://api.elevenlabs.io/v1/mcp - ✔ Connected\n\
            elevenlabs: uvx elevenlabs-mcp - ✔ Connected";
        assert_eq!(
            voice_servers_in(listing),
            ["claude_ai_ElevenLabs_2", "elevenlabs"]
        );
    }

    #[test]
    fn claude_codes_own_model_is_read_by_its_family() {
        assert_eq!(MODELS[family("opus[1m]")].0, "opus");
        assert_eq!(MODELS[family("claude-sonnet-5-5")].0, "sonnet");
        assert_eq!(MODELS[family("Haiku")].0, "haiku");
        assert_eq!(MODELS[family("")].0, "opus");
    }

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
                    input: serde_json::json!({ "path": "/p", "clipId": "c1", "end": 4 }),
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
        assert!(parse(r#"{"type":"rate_limit_event"}"#).is_empty());
        assert!(parse("not json").is_empty());
    }

    #[test]
    fn a_step_is_named_in_words() {
        use serde_json::json;
        assert_eq!(
            describe(
                "Bash",
                &json!({ "command": "codex exec -C '/p' -- \"Generate an image: a red sky over the sea. Save it as generated/sky.png\"" })
            ),
            (
                "Drawing a picture".to_owned(),
                "a red sky over the sea.".to_owned()
            )
        );
        assert_eq!(
            describe(
                "Bash",
                &json!({ "command": "codex exec -C '/p' -- \"Join the frames with ffmpeg\"" })
            )
            .0,
            "Joining the frames into a clip"
        );
        assert_eq!(
            describe(
                "mcp__concat__media_import",
                &json!({ "path": "/p", "file": "/p/generated/kraken.png" })
            )
            .0,
            "Importing kraken.png"
        );
        assert_eq!(
            describe("mcp__concat__preview_frame", &json!({ "time": 26 })).0,
            "Checking the frame at 26s"
        );
        assert_eq!(
            describe(
                "mcp__concat__edit_apply",
                &json!({ "command": { "op": "batch", "commands": [
                    { "op": "setClipKey" }, { "op": "setClipKey" }, { "op": "addTextClip" }
                ] } })
            ),
            (
                "Editing (3 changes)".to_owned(),
                "set clip key ×2, add text clip".to_owned()
            )
        );
        assert_eq!(
            describe(
                "mcp__concat__edit_apply",
                &json!({ "command": { "op": "addLayerClip", "effectId": "concat.rain-drops" } })
            )
            .0,
            "Adding rain drops"
        );
        assert_eq!(
            describe("mcp__concat__something_new", &json!({})).0,
            "something new"
        );
    }

    #[test]
    fn times_read_as_people_say_them() {
        assert_eq!(seconds(Duration::from_millis(5_120)), "5.1s");
        assert_eq!(duration(Duration::from_secs(11)), "11s");
        assert_eq!(duration(Duration::from_secs(134)), "2m 14s");
    }

    #[test]
    fn a_long_detail_is_cut() {
        let long = "x".repeat(DETAIL_CHARS + 20);
        assert_eq!(clip(&long).chars().count(), DETAIL_CHARS + 1);
        assert_eq!(clip("short"), "short");
    }
}
