// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! A channel's narration voice: the `Voice:` line of its `CLAUDE.md`,
//! which the agent reads before it narrates, and the voices on offer.
//!
//! The line is the agent's contract (its brief says to narrate in the
//! voice it names, and to write it there once asked), so the sidebar's
//! picker writes the same line rather than a setting of its own: whoever
//! sets it, the other sees it.
//!
//! The voices on offer are `voices.json` in the project's folder. The app
//! holds no ElevenLabs key; the agent, signed in through the user's
//! connector, lists the voices and writes the file when asked.

use std::path::Path;

use serde::Deserialize;

/// The file the channel's recipe and its voice line live in.
const GUIDE: &str = "CLAUDE.md";
/// The voices the agent found, for the picker.
pub const CHOICES: &str = "voices.json";

/// A voice: what the user calls it and ElevenLabs' id for it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Voice {
    #[serde(default)]
    pub name: String,
    pub id: String,
    /// One of the user's own - cloned or designed - rather than shared.
    #[serde(default)]
    pub mine: bool,
    /// A few words on how it sounds.
    #[serde(default)]
    pub note: String,
}

/// The voice the channel's `CLAUDE.md` names, if it names one.
pub fn of_project(folder: &Path) -> Option<Voice> {
    let text = std::fs::read_to_string(folder.join(GUIDE)).ok()?;
    parse(&text)
}

/// The `Voice:` line's voice. Written by the picker as "Voice: Duang
/// (2hqnj6...)", but the agent words it its own way too - "Voice:
/// ElevenLabs 2hqnj6... (the user's own voice), model eleven_v4" - so the
/// id is whatever word looks like one, and the name what is in brackets,
/// or else the words before the id.
fn parse(text: &str) -> Option<Voice> {
    let line = text
        .lines()
        .map(str::trim)
        .find_map(|line| {
            line.trim_start_matches(['-', '*', ' '])
                .strip_prefix("Voice:")
        })?
        .trim();
    let looks_like_id = |word: &str| {
        (16..=32).contains(&word.len()) && word.chars().all(|c| c.is_ascii_alphanumeric())
    };
    let id = line
        .split(|c: char| c.is_whitespace() || "(),;:".contains(c))
        .find(|word| looks_like_id(word))?;
    let bracketed = line
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(inside, _)| inside.trim())
        .filter(|inside| *inside != id && !inside.is_empty());
    let before = line
        .split_once(id)
        .map(|(before, _)| {
            before
                .trim_end_matches(|c: char| c.is_whitespace() || "(,;:".contains(c))
                .trim()
                .trim_start_matches("ElevenLabs")
                .trim()
        })
        .filter(|before| !before.is_empty());
    Some(Voice {
        name: bracketed.or(before).unwrap_or_default().to_owned(),
        id: id.to_owned(),
        ..Voice::default()
    })
}

/// Writes `voice` as the channel's `Voice:` line: in place of the one
/// there, or at the end of the file. A project without a `CLAUDE.md`
/// gets one with the line alone.
pub fn set(folder: &Path, voice: &Voice) -> Result<(), String> {
    let path = folder.join(GUIDE);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, with_voice(&text, voice))
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn with_voice(text: &str, voice: &Voice) -> String {
    let line = if voice.name.is_empty() {
        format!("Voice: {}", voice.id)
    } else {
        format!("Voice: {} ({})", voice.name, voice.id)
    };
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|existing| {
            if !found && existing.trim_start().starts_with("Voice:") {
                found = true;
                line.clone()
            } else {
                existing.to_owned()
            }
        })
        .collect();
    if !found {
        if lines.last().is_some_and(|last| !last.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(line);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The voices the agent listed in `voices.json`: the user's own first.
/// None found is an empty list, and a file that does not read is logged.
pub fn choices(folder: &Path) -> Vec<Voice> {
    let Ok(text) = std::fs::read_to_string(folder.join(CHOICES)) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<Voice>>(&text) {
        Ok(mut voices) => {
            voices.retain(|voice| !voice.id.trim().is_empty());
            voices.sort_by_key(|voice| !voice.mine);
            voices
        }
        Err(error) => {
            log::warn!("{CHOICES}: {error}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_voice_line_reads_with_a_name_or_without() {
        let named = parse("# Sea Legends\n\nVoice: Duang (2hqnj6qelkp9E3ega0Xy)\n");
        assert_eq!(named.as_ref().map(|v| v.name.as_str()), Some("Duang"));
        assert_eq!(named.map(|v| v.id), Some("2hqnj6qelkp9E3ega0Xy".to_owned()));
        let agents =
            parse("Voice: ElevenLabs 2hqnj6qeIkp9E3ega0Xy (the user's own voice), model eleven_v4")
                .expect("a voice");
        assert_eq!(agents.id, "2hqnj6qeIkp9E3ega0Xy");
        assert_eq!(agents.name, "the user's own voice");
        assert_eq!(
            parse("- Voice: 2hqnj6qeIkp9E3ega0Xy").map(|v| v.name),
            Some(String::new())
        );
        assert_eq!(parse("No voice here"), None);
    }

    #[test]
    fn setting_the_voice_replaces_the_line_or_adds_one() {
        let voice = Voice {
            name: "Duang".to_owned(),
            id: "x1".to_owned(),
            ..Voice::default()
        };
        assert_eq!(
            with_voice("# Channel\nVoice: old\nMore", &voice),
            "# Channel\nVoice: Duang (x1)\nMore\n"
        );
        assert_eq!(
            with_voice("# Channel", &voice),
            "# Channel\n\nVoice: Duang (x1)\n"
        );
    }
}
