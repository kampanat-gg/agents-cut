// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! A channel's episodes as the files the agent keeps for them: each
//! video's folder under `episodes/`, its `brief.md` (title, status,
//! synopsis) and its `storyboard.md` (the beats). The agent's brief says
//! how they are written; this reads them for the side panel's cards, the
//! storyboard view and each episode's own conversation.
//!
//! A timeline finds its folder by its number: "EP3 Flying Dutchman" is
//! the folder that starts `ep03-`.

use std::path::{Path, PathBuf};

/// The folder episodes live in, in a project's.
pub const FOLDER: &str = "episodes";

/// What an episode's `brief.md` says about it up front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Brief {
    pub title: String,
    /// `idea`, `storyboard`, `building`, `done` or `exported`; empty when
    /// the brief does not say.
    pub status: String,
    pub synopsis: String,
}

/// One beat of a storyboard: its span and name, and what is heard, seen,
/// moved and played.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beat {
    pub time: String,
    pub title: String,
    pub voice: String,
    pub picture: String,
    /// The picture's file, relative to the episode's folder, when the
    /// line names one that is there.
    pub image: Option<PathBuf>,
    pub motion: String,
    pub sound: String,
}

/// The episode number in a timeline's name: "EP3 Flying Dutchman" is 3.
pub fn number_of(name: &str) -> Option<u32> {
    let rest = name.trim();
    let rest = rest
        .strip_prefix("EP")
        .or_else(|| rest.strip_prefix("Ep"))
        .or_else(|| rest.strip_prefix("ep"))?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The folder of the episode a timeline named `name` is, relative to the
/// project: the one under `episodes/` that starts `epNN-` for its number.
pub fn folder_of(project: &Path, name: &str) -> Option<String> {
    let number = number_of(name)?;
    let prefix = format!("ep{number:02}-");
    let read = std::fs::read_dir(project.join(FOLDER)).ok()?;
    let mut found: Vec<String> = read
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|folder| folder.to_lowercase().starts_with(&prefix))
        .collect();
    found.sort();
    found
        .into_iter()
        .next()
        .map(|folder| format!("{FOLDER}/{folder}"))
}

/// The front matter of `folder`'s `brief.md`; an empty brief when there
/// is none.
pub fn brief(project: &Path, folder: &str) -> Brief {
    std::fs::read_to_string(project.join(folder).join("brief.md"))
        .map(|text| parse_brief(&text))
        .unwrap_or_default()
}

fn parse_brief(text: &str) -> Brief {
    let mut brief = Brief::default();
    let Some(rest) = text.trim_start().strip_prefix("---") else {
        return brief;
    };
    let Some((front, _)) = rest.split_once("\n---") else {
        return brief;
    };
    for line in front.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        match key.trim() {
            "title" => brief.title = value,
            "status" => brief.status = value.to_lowercase(),
            "synopsis" => brief.synopsis = value,
            _ => {}
        }
    }
    brief
}

/// The beats of `folder`'s `storyboard.md`, in order.
pub fn storyboard(project: &Path, folder: &str) -> Vec<Beat> {
    let dir = project.join(folder);
    let Ok(text) = std::fs::read_to_string(dir.join("storyboard.md")) else {
        return Vec::new();
    };
    let mut beats = parse_storyboard(&text);
    for beat in &mut beats {
        beat.image = picture_file(&beat.picture).filter(|file| dir.join(file).is_file());
    }
    beats
}

fn parse_storyboard(text: &str) -> Vec<Beat> {
    let mut beats: Vec<Beat> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(heading) = line.strip_prefix("## ") {
            let (time, title) = match heading.split_once('·') {
                Some((time, title)) => (time.trim(), title.trim()),
                None => (heading.trim(), ""),
            };
            beats.push(Beat {
                time: time.to_owned(),
                title: title.to_owned(),
                ..Beat::default()
            });
            continue;
        }
        let Some(beat) = beats.last_mut() else {
            continue;
        };
        let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) else {
            continue;
        };
        let Some((key, value)) = item.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        match key.trim().to_lowercase().as_str() {
            "voice" | "narration" | "เสียงพากย์" | "พากย์" => {
                beat.voice = value
            }
            "picture" | "image" | "visual" | "ภาพ" => beat.picture = value,
            "motion" | "move" | "การเคลื่อนไหว" => beat.motion = value,
            "sound" | "music" | "sfx" | "เสียง" => beat.sound = value,
            _ => {}
        }
    }
    beats
}

/// The file a beat's picture line names, when it names one: its first
/// word, if that is an image path.
fn picture_file(picture: &str) -> Option<PathBuf> {
    let first = picture.split_whitespace().next()?;
    let first = first.trim_matches(['`', '"', ',']);
    let image = ["png", "jpg", "jpeg", "webp", "gif", "webm"]
        .iter()
        .any(|extension| first.to_lowercase().ends_with(&format!(".{extension}")));
    (image && !first.contains("..")).then(|| PathBuf::from(first))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timeline_names_its_episode_by_number() {
        assert_eq!(number_of("EP3 Flying Dutchman"), Some(3));
        assert_eq!(number_of("ep12 Lanna"), Some(12));
        assert_eq!(number_of("Timeline 1"), None);
    }

    #[test]
    fn a_brief_reads_its_front_matter() {
        let brief = parse_brief(
            "---\ntitle: EP3 Flying Dutchman\nstatus: Storyboard\nsynopsis: \"A ship that never lands.\"\n---\nFacts.",
        );
        assert_eq!(brief.title, "EP3 Flying Dutchman");
        assert_eq!(brief.status, "storyboard");
        assert_eq!(brief.synopsis, "A ship that never lands.");
        assert_eq!(parse_brief("No front matter"), Brief::default());
    }

    #[test]
    fn a_storyboard_reads_beat_by_beat() {
        let beats = parse_storyboard(
            "# EP3\n\n## 0:00-0:03 · Hook\n- Voice: \"Never see it.\"\n- Picture: images/ship.png - a ship in fog\n- Motion: slow push\n- Sound: a bell\n\n## 0:03-0:08\n- Voice: Next.\n",
        );
        assert_eq!(beats.len(), 2);
        assert_eq!(beats[0].time, "0:00-0:03");
        assert_eq!(beats[0].title, "Hook");
        assert_eq!(beats[0].voice, "\"Never see it.\"");
        assert_eq!(
            picture_file(&beats[0].picture),
            Some(PathBuf::from("images/ship.png"))
        );
        assert_eq!(beats[0].sound, "a bell");
        assert_eq!(beats[1].title, "");
        assert_eq!(picture_file("a ship in fog"), None);
    }
}
