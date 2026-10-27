## Making pictures with Codex

To make a picture, run Codex in Bash, exactly in this form and as the whole
command:

```
codex exec --skip-git-repo-check --sandbox workspace-write --ephemeral{model} -C '{path}' [-i <reference> ...] -- "Generate an image: <what to draw>, <aspect ratio>. Save it as generated/<short-name>.png in the current directory. Reply with only the saved file's path."
```

- The `--` before the prompt is required: `-i` takes every word after it as a
  file until `--`.
- A picture takes one to two minutes. You may run up to three at once in the
  background and wait for them.
- Bash runs nothing but `codex exec`. Then import the file with `media_import`
  and place it with `clip_add`.
- Look at a picture with Read before placing it when the result matters.

### Layers, for scenes that move

Make a scene that will move as layers, not one flat picture:

1. One full-frame background with no subject in it, in the project's aspect
   ratio.
2. Each subject, person or prop as a picture of its own with a transparent
   background (see below).
3. Place the background on the lowest lane and each layer on its own lane above
   it, in depth order.
4. Give each layer a slow move with keyframes, foreground faster than
   background (parallax): drift, push in, a small rotation, a pop in.

### Transparent backgrounds

For a cut-out, sticker, prop or person to place over something else, put in the
prompt: `isolated, TRANSPARENT background (real alpha channel), no backdrop, no
shadow plate`. Codex then saves an RGBA PNG.

### Keeping a character the same (identity lock)

When a picture shows someone who has a folder in `characters/`:

1. Read their notes first and follow them.
2. Choose up to five reference pictures from their folder, the clearest face
   first, and pass each with `-i` before `--`.
3. Say in the prompt: keep the exact face, hair, glasses and build from the
   attached references; change only pose, clothes and setting as asked.

Never invent the face of a real, identifiable person who has no folder in
`characters/`. Ask the user for photos to put there instead.
