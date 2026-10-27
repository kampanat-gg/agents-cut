# Styles

A style is what a new project starts as: one look and one way of working for
every episode of a channel, so nobody has to write a prompt for it.

## Where they live

Built-in styles are folders in `src/crates/concat/styles/`, compiled into the
app (`src/crates/concat/src/styles.rs`, `BUILT_IN`). Each has two files:

- `style.toml`: the name and a sentence about it (English and Thai), the
  frame's shape (`aspect`, one of the New Project sheet's shapes, such as
  `"9:16"`), and the sidebar's buttons (`[[starters]]`).
- `CLAUDE.md`: the recipe, for the agent: how an episode is built, how to
  write, the look, the order of the lanes, rules for real people.

## The built-in styles

All 9:16, picked from what short-form channels make most and from what the
app can build from pictures, keyframes, titles and captions (no generated
video, nothing made to pass for real people or real news).

| Style | For |
| --- | --- |
| `paper-collage-history` | History: archival-style prints cut out on aged paper |
| `vintage-engraving` | Strange facts of nature and science, 19th-century engravings as paper puppets |
| `isometric-world` | How a city or system works, one isometric diorama with big numbers |
| `map-explainer` | Geography, history and events on calm maps with arrows and lower thirds |
| `true-crime-mystery` | Unsolved mysteries on a dark evidence board |
| `top-list` | Top 5 countdowns, one picture and one fact per item |
| `money-explainer` | Personal finance with flat illustrations and editable numbers |
| `kinetic-quote` | Quotes, lessons and podcast moments as moving words |
| `what-if` | Imagined "what if" worlds as documentary stills, marked as AI |
| `anime-story` | Stories and feelings as hand-painted anime, characters kept the same, shot lists |
| `cute-3d-character` | A 3D mascot or pet that explains and reacts |
| `clay-fable` | Children's fables in stop-motion clay, poses edited from the last |
| `music-collage` | A song's short as paper-collage stop motion on the beat (from the KININ shorts) |
| `lofi-loop` | Seamless cosy loops under a song |

## What choosing one does

New Project lists the styles under STYLE. Picking one sets the frame's shape.
Create then makes the project folder as usual and adds:

- `style.toml` and `CLAUDE.md` (a `CLAUDE.md` already there is kept);
- `generated/`, `characters/` and `assets/`.

The sidebar runs `claude` in the project folder, and Claude Code reads a
`CLAUDE.md` in its working directory on its own, so the recipe reaches the
agent with nothing more. When the project opens, the sidebar shows the
style's buttons under "Start with": a button with `fill = true` puts its
prompt in the box for the user to finish (a topic, a date); the others send
their prompt at once.

Both files belong to the project from then on: edit them to change how that
channel's episodes come out.

## Adding a style

1. Make `src/crates/concat/styles/<id>/` with `style.toml` and `CLAUDE.md`,
   copying `paper-collage-history` as a start.
2. Add it to `BUILT_IN` in `styles.rs`.
3. `cargo test -p concat styles` checks that every style parses, names a
   known frame shape and has buttons and a recipe.

A good recipe names concrete things: episode length and structure, how
often something moves, the background made once and reused, the exact
words that start every picture prompt, the accent colour, the label style,
and the lanes from the bottom up.
