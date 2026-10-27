# Style: music, paper-collage stop motion

A music channel's vertical shorts (9:16): one song section (usually the
first hook, 20 to 40 seconds) as paper-collage stop motion. The artist's
character sits in one scene and changes pose on the beat; props, birds,
hearts and notes are paper cut-outs; the lyrics appear as cut-out letters.

## Inputs

- The song file and its lyrics (`.txt`, `[Verse]` and `[Hook]` tags as from
  Suno) in `assets/`. The words on screen come from the lyrics file; the
  timing comes from the song (`captions_add` on the song, then fix the words
  to match the file).
- The character in `characters/<name>/`.

## Poses on the beat

- First make one base pose in the scene (`generated/me_base.png`) with a
  TRANSPARENT background. Every other pose is an edit of it: pass it with
  `-i` and change only the body, so the bench or chair and the figure's
  place stay exactly where they are.
- Four to six poses that fit the lyric of each line: listening, nodding,
  singing along, leaning back, shy, looking at the phone.
- Change pose every 2 beats; in the hook every beat. One section of the song
  is one activity: do not change place or clothes mid-section.
- Stop motion runs at 12 pictures a second (2 frames each at 24 fps): props
  re-scatter every 2 beats, hearts pulse on the beat (scale).

## The hook

On the hook's first line, a paper wipe (a torn paper picture sliding across)
changes the background to a burst of sun rays turning on the beat, and the
song title drops in syllable by syllable.

## The look

- Torn paper edges with a thin white border on every cut-out, flat colours,
  paper grain, a soft drop shadow, slightly wobbly placement.
- Lyrics: one line at a time, letters cut from different papers and
  magazines (ransom-note style), lower middle of the frame.
- Picture prompts start with: `paper collage cut-out, torn paper edges,
  white paper border, flat colours, paper grain texture,`

## Layers on the timeline

Background (and sun rays), scene parts (far, lawn, tree), character poses,
props, lyrics, title.
