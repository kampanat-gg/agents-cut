## Making pictures with Codex

To make a picture, run Codex in Bash, exactly in this form and as the whole
command:

```
codex exec --skip-git-repo-check --sandbox workspace-write --ephemeral{model} -C '{path}' [-i <reference> ...] -- "Generate an image: <what to draw>, <aspect ratio>. Save it as episodes/<episode>/images/<short-name>.png in the current directory. Reply with only the saved file's path."
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

### A layer plan for every scene

Split a scene into fine layers, and give each the cheapest way to move that
looks right. Write the plan in one line per layer before drawing anything.

| Layer | How it moves |
| --- | --- |
| Sky, distant land, a room | One still picture, a slow keyframed drift or push |
| Clouds, smoke, mist | A transparent still drifting slowly, or the `concat.drifting-fog` effect |
| Rain, snow, bubbles, dust, light shafts | Code, not pictures: an effect layer (`edit_apply` with `addLayerClip`) of `concat.rain-drops`, `concat.snowfall`, `concat.bubbles`, `concat.falling-dust` or `concat.light-rays`; set its knobs |
| The surface of the sea, a lake, a river | The still sea picture with `concat.water-ripple` on a layer just above it (set its horizon), plus 2 or 3 transparent wave-band strips wider than the frame, sliding opposite ways and bobbing |
| A breaking wave, a splash | A motion sequence |
| A creature, a limb, hair, a character's action | A motion sequence |
| Props, labels, text | Transparent stills with keyframed pops and drifts |

An effect layer treats everything beneath it: put `water-ripple` on the lane
right above the sea, before the creature, and weather layers on top.

### Motion sequences, for movement that must be smooth

Keyframes on a still are enough for drifting, zooming, turning and popping.
For movement inside the picture (a tentacle curling, a wave breaking, a wing
flapping, a walk, hair in the wind), draw it frame by frame as a motion
sequence. Use one only where the motion matters: it costs a picture per
frame.

1. Keep everything that does not move on separate layers: the background is
   one still. Only the moving part is drawn per frame, with a TRANSPARENT
   background.
2. Draw a base frame first: `episodes/<episode>/motion/<name>/base.png`. It is a
   reference, not a frame: frame `0001.png` is drawn as an edit of it like
   every other key, or the first frame jumps against the rest.
3. Draw 3 to 5 key poses, each an edit of the base (pass the base with
   `-i`), all at once in the background. Say in every prompt: keep the
   exact same style, line weight, colours, framing, size and position as the
   base; change only the motion.
4. Draw the in-betweens: for each pair of neighbouring keys, pass the two
   keys and the base with `-i` and ask for the frame one third (or two
   thirds) of the way from the first to the second. Run them four at once.
5. Number the frames `0001.png`, `0002.png`, ... in playing order. For a
   loop, the last frames lead back into `0001`.
6. Have Codex join them into one clip with a transparent background, in the
   same Codex call or a new one, with exactly this command in the sequence
   folder: `ffmpeg -y -framerate 12 -i %04d.png -c:v libvpx-vp9 -pix_fmt
   yuva420p -b:v 0 -crf 30 -auto-alt-ref 0 ../<name>.webm`. 12 pictures a
   second is stop-motion pace; use 8 for a slower, more handmade feel.
7. Import `episodes/<episode>/motion/<name>.webm` and place it over the background.
   Codex draws 2:3, so set both to fill the frame (`frameClip` with fill) or
   they show bars in 9:16. To hold it longer, place it again right after
   itself (it loops cleanly when the last frame leads back into the first).
8. To fix one bad frame, redraw only that frame between its neighbours, then
   join the clip again.

A 12-frame sequence takes about ten to twelve minutes (in-betweens are the
slow part). Start it first and build the rest of the episode while it
draws. Tell the user how many pictures a sequence will take before drawing
one longer than 24 frames.
