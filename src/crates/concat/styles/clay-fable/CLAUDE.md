# Style: clay fable

A channel of short vertical videos (9:16) for children and families: fables,
bedtime stories, small lessons, as stop-motion clay animation on miniature
sets.

## Characters

Each character is made once as a clay figure and saved in
`characters/<name>/` (front and three-quarter). Every later picture of them
passes those with `-i`, so they stay the same figure.

## Stop motion

- A pose change is an edit of the previous picture, not a new one: pass the
  last pose with `-i` and change only the arms, head or expression. The set
  and the figure's place stay put, so cuts do not jump.
- Key actions (a step, a wave, a jump) as a motion sequence (see the
  brief): key poses edited from the figure's reference, in-betweens
  between them, joined at 12 pictures a second into one clip.
- Between actions, hold a pose and add a tiny breathing move (scale 100 to
  101 percent) so it never looks frozen.

## Episodes

- 45 to 90 seconds. Once upon a time, a problem, three tries, what they
  learn. The moral as the last line, spoken and on screen.
- Gentle pacing; nothing scary.

## The look

- Plasticine figures with visible thumbprints, rounded shapes, felt and
  cardboard sets, soft studio light, shallow depth of field, slightly
  imperfect.
- Text: chunky rounded letters in warm cream with a soft shadow.
- Picture prompts start with: `stop-motion claymation, plasticine figure
  with visible thumbprints, miniature handmade set, soft studio lighting,
  shallow depth of field,`

## Sound and words

Thai unless the user writes in English. Simple words a 5-year-old knows.
A warm narrating voice: a voice file the user puts in `assets/`; without
one, tell the story in short on-screen lines. Light music box or ukulele.

## Layers on the timeline

Set, figures, props, text, captions.
