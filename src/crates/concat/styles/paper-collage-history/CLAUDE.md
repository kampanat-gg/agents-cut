# Style: paper-collage history

This project is a channel of short vertical videos (9:16) that tell history:
people, events, places from the past. Every episode uses this one look, so a
viewer knows the channel at a glance. Follow this file for every episode.

## Episodes

- Name tabs `EP<number> <short title>`.
- Length 45 to 90 seconds unless the user asks otherwise.
- Structure:
  1. Hook, 0 to 3 s: the most surprising fact or question, as big text over
     the strongest picture.
  2. Context, about 10 s: who, when, where. A date card.
  3. Three to five beats, 8 to 15 s each: one fact or turn per beat, one
     picture or layered scene per beat.
  4. Payoff: what it led to, or why it matters now.
  5. Close, 2 to 3 s: one line that invites a follow or the next episode.
- Nothing on screen stays still for longer than 3 seconds: drift, push in,
  a small rotation, a pop in.

## Writing

- The script and the on-screen words are in Thai unless the user writes in
  English. Plain, spoken, short sentences; one idea a line.
- Before building an episode, write the script as beats and show it to the
  user in a few lines, then build it. Stick to facts; say when something is
  disputed. Name dates in the Thai calendar (พ.ศ.) with the day and month.

## The look

- Background: one aged paper texture, warm beige with soft stains and
  fibres, no objects, full frame 9:16. Make it once as
  `generated/paper-bg.png` and reuse it under every episode.
- Pictures: archival-style black-and-white photographs, printed with a white
  border, slightly rotated (2 to 6 degrees), like prints pinned to paper.
- People and objects: cut-outs with a transparent background and a thin white
  paper edge, placed over the prints.
- Accent colour: one red, #D7322D, for a round dot, a stripe of tape, a
  ribbon, a circle behind a person. No other bright colours.
- Labels: dark text on white paper strips with torn edges, or white text on a
  black bar. A date label reads like `ครั้งที่ 1 · 20 ต.ค. 2501` with a red
  dot before it.
- Big numbers (years, counts, episode numbers) as outlined, heavy numerals.
- Picture prompts for Codex start with: `vintage archival photograph, black
  and white, printed with white border, paper collage style,` and end with
  the aspect ratio. Cut-outs add the transparent-background words from the
  brief.

## Layers on the timeline

From the bottom lane up: paper background, prints and scene pictures,
cut-outs, labels and titles, captions. Keep that order in every episode.

## Real events and archive pictures

Generated pictures are illustrations, not records. Real archive photos the
user put in `assets/` come first; use a generated picture where there is
none. Never fake a document, a newspaper page, a signature or a caption that
claims to be original. When a generated picture stands in for a specific
real thing (a ship, an aircraft, a building), match what the record says
it was, and tell the user what you could not match.

## Real people

Never generate the face of a real, identifiable person. For a real person,
use photos the user put in `characters/<name>/` or `assets/`. If there are
none, say which photo is needed and leave a labelled placeholder (a paper
strip with the person's name) where it goes.
