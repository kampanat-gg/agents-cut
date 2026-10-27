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

Most of the agent's tool layer is already built. The new work is the sidebar,
the process that runs the agent, and live mode (below).

## The missing piece: live mode

Right now an agent edits **its own** project, and the window refuses to let it
open the one on screen (upstream issue #337). A sidebar agent must edit the
project you are looking at, through the window's own undo stack, so that:

- every agent edit is one undo step in the window, and Cmd-Z works on it;
- the preview and timeline update as the agent edits;
- you and the agent never write over each other.

That is the core engineering task: route the API Hub's edit commands for the
open project into the window's edit path, instead of locking it.

## Running the agent: Claude Code CLI

The app spawns the user's installed `claude` and talks to it over stdio:

```sh
claude -p \
  --input-format stream-json --output-format stream-json --verbose \
  --mcp-config <app-generated>.json \
  --append-system-prompt-file <style preset>.md \
  --allowedTools "mcp__concat__*" \
  --permission-prompt-tool mcp__concat__approve
```

- **No API key in the app**: it uses the user's own Claude login.
- **Streaming**: each JSON line (text, tool call, tool result) becomes a chat
  bubble or a tool card in the sidebar.
- **Approvals**: `--permission-prompt-tool` sends "may I run X?" to a tool the
  app serves, which shows Allow / Deny in the sidebar.
- **History**: keep the `session_id`, then `--resume` it when the project
  reopens.
- **Fallback**: if `claude` is not installed, the sidebar says so and links to
  the install page.

## Other tools, via the generated `--mcp-config`

| Need | MCP server |
| --- | --- |
| Edit this video | the app itself (`concat --mcp window`, after live mode) |
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

1. Live mode: the agent edits the open project through the window's undo
   stack.
2. Sidebar pane: chat list, input box, tool cards, approvals.
3. Agent process: spawn, stream, resume, cancel.
4. MCP settings page: Codex, image/video generation, voice.
5. Style presets.

## Notes

- To give the app to other people, use the Claude Agent SDK with an API key
  rather than a personal Claude login. Check Anthropic's terms first.
- The code is AGPL-3.0: a distributed fork must publish its source. The name
  "Concat" and its logo are not covered by the licence. See
  [TRADEMARK.md](../TRADEMARK.md): a modified build needs its own name and
  icon (for example "Agents Cut").
