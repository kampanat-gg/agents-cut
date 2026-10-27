# Agent sidebar: plan

The goal: chat with an agent in the left sidebar, and it edits the video open
in the window, calling other tools (Codex, image generation, voice, captions)
when it needs to. Prompting and editing in one app.

```
┌────────────┬──────────────────────────────────────┐
│            │            preview                   │
│  agent     │          ◁   ▶   ▷                   │
│  chat      ├──────────────────────────────────────┤
│            │  media · audio · text · effects ...  │
│            ├────┬─────────────────────────────────┤
│            │ V1 │                                 │
│ ┌────────┐ │ V2 │          timeline               │
│ │prompt ⏎│ │ A1 │                                 │
│ └────────┘ │    │                                 │
└────────────┴────┴─────────────────────────────────┘
```

## What already exists

- **`concat-mcp`**: an MCP server with about 40 editing tools (projects,
  clips, trims, titles, captions, keyframes, effects, templates, export,
  `preview_frame` so the agent can look at the result). Run it with
  `concat --mcp`. See [MCP.md](MCP.md).
- **`concat-api` / `concat-server`**: the same commands as JSON-RPC, served
  from the running window (Settings > Automation).
- **`concat-speech`**: local Whisper captions and TTS, already wired to the
  `captions_add` tool.

Most of the agent's tool layer is already built, live mode included. The new
work is the sidebar and the process that runs the agent.

## Live mode (done)

An agent connected through the window (`concat --mcp window`) edits the
project you are looking at. The API's server hands every call on that
project to the window (`concat_api::LiveWindow`, implemented in
`crates/concat/src/studio/live.rs`), which runs it on its event loop through
the same paths a click takes:

- every agent edit is one undo step in the window, and Cmd-Z works on it;
- the preview and timeline update as the agent edits, and the autosave
  writes it;
- there is one session, so you and the agent never save over each other.

Frames and exports read a copy of the edit (`Session::detached`), so a
render never holds up the window. A call waits at most 30 seconds for the
window (a modal dialog holds the event loop) and then fails with a reason.

## Running the agent: Claude Code CLI (done)

The sidebar (`ui/agent.slint`, `src/agent.rs`) runs the user's installed
`claude`, one process per turn, with the request on stdin:

```sh
claude -p --output-format stream-json --verbose \
  --strict-mcp-config --mcp-config <config>/agent-mcp.json \
  --tools "" --allowedTools mcp__concat \
  --append-system-prompt "<where the project is, how to work>" \
  [--resume <session id>]
```

- **No API key in the app**: it uses the user's own Claude login.
- **Only the editor's tools**: `--tools ""` takes away the shell and file
  tools, and `--strict-mcp-config` the user's other MCP servers, so the
  agent can do nothing but edit (and nothing needs an approval yet).
- **The server**: `agent-mcp.json` (mode 600, it holds the token) points
  `concat --mcp window` at the Automation page's server when it is on, or at
  one the sidebar starts on a free loopback port for itself.
- **Streaming**: each JSON line becomes a reply bubble or a tool row, which
  turns green or red when the tool's result comes back.
- **History**: the `session_id` of the first turn is resumed by the next;
  New chat forgets it. Stop kills the turn's process.
- **Fallback**: if `claude` is not found (the PATH, `~/.local/bin`,
  `~/.claude/local`, Homebrew; or `CONCAT_CLAUDE`), the sidebar says to
  install it.

Still to do here: Allow / Deny for tools beyond the editor's, through
`--permission-prompt-tool`, once those tools are added (step 4).

## Other tools, via the generated `--mcp-config`

| Need | MCP server |
| --- | --- |
| Edit this video | the app itself (`concat --mcp window`) |
| Code, scripts, batch jobs | Codex: `codex mcp-server` |
| Image or video generation | any image/video MCP the user adds (fal, Replicate, Gemini Imagen/Veo, ...) |
| Voice | local TTS (built in) or an ElevenLabs-style MCP |
| Subtitles | built in (`captions_add`, local Whisper) |

A settings page lists these servers with on/off switches and writes the JSON.
Generated files land in the project's media folder, and the agent imports
them with `media_import`.

**Google Antigravity** is an IDE with its own agent, not an MCP server, so it
cannot be plugged in. Its models (Gemini, Imagen, Veo) can be, through an MCP
server that wraps the Gemini API.

## Video styles

A style is a small file: a system-prompt fragment (pacing, caption look,
transitions, music mood), default parameters and an optional template id.
Picking a style in the sidebar passes `--append-system-prompt-file` for the
next run. Examples: TikTok fast-cut, Podcast clip, Tutorial, Cinematic.

## Build order

1. ~~Live mode: the agent edits the open project through the window's undo
   stack.~~ Done.
2. ~~Sidebar pane: chat list, input box, tool cards.~~ Done; approvals come
   with step 4.
3. ~~Agent process: spawn, stream, resume, cancel.~~ Done.
4. MCP settings page: Codex, image/video generation, voice.
5. Style presets.

## Notes

- To give the app to other people, use the Claude Agent SDK with an API key
  rather than a personal Claude login. Check Anthropic's terms first.
- The code is AGPL-3.0: a distributed fork must publish its source. The name
  "Concat" and its logo are not covered by the licence. See
  [TRADEMARK.md](../TRADEMARK.md): a modified build needs its own name and
  icon (for example "Agents Cut").
