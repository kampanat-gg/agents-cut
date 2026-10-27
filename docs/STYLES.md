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
