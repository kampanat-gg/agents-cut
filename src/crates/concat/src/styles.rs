// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Styles: what a new project starts as, so a channel's every episode has
//! one look without anyone writing a prompt for it.
//!
//! A style is a folder under `styles/` in this crate, built in: a
//! `style.toml` with its name, its frame and the sidebar's buttons, and a
//! `CLAUDE.md` with the recipe - episode structure, writing, the look,
//! the layers. A project made with a style gets both files and the folders
//! the agent works in; Claude Code reads a `CLAUDE.md` in its working
//! directory on its own, and the sidebar runs it in the project's folder,
//! so the recipe reaches the agent with nothing more. Both files are the
//! project's from then on, to edit like any other.

use std::path::Path;

use serde::Deserialize;

/// The file a project's style settings are copied to.
pub const SETTINGS: &str = "style.toml";

/// The folders a styled project starts with: pictures the agent made,
/// characters to keep the same, and what the user brings.
const FOLDERS: [&str; 3] = ["generated", "characters", "assets"];

/// The built-in styles: their folder name, settings and recipe.
const BUILT_IN: [(&str, &str, &str); 1] = [(
    "paper-collage-history",
    include_str!("../styles/paper-collage-history/style.toml"),
    include_str!("../styles/paper-collage-history/CLAUDE.md"),
)];

/// One button in the sidebar: a prompt sent as it is, or put in the box
/// for the user to finish.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Starter {
    pub label: String,
    #[serde(default)]
    pub label_th: String,
    pub prompt: String,
    #[serde(default)]
    pub prompt_th: String,
    #[serde(default)]
    pub fill: bool,
}

impl Starter {
    /// The label in the interface's language.
    pub fn label(&self) -> &str {
        in_language(&self.label, &self.label_th)
    }

    /// The prompt in the interface's language.
    pub fn prompt(&self) -> &str {
        in_language(&self.prompt, &self.prompt_th)
    }
}

/// A style's `style.toml`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Settings {
    pub name: String,
    #[serde(default)]
    pub name_th: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub description_th: String,
    /// The frame's shape, as `ASPECTS` names it: "9:16".
    #[serde(default)]
    pub aspect: String,
    #[serde(default)]
    pub starters: Vec<Starter>,
}

impl Settings {
    pub fn name(&self) -> &str {
        in_language(&self.name, &self.name_th)
    }

    pub fn description(&self) -> &str {
        in_language(&self.description, &self.description_th)
    }

    /// Its frame's shape as an index into `ASPECTS`, if it names one.
    pub fn aspect_index(&self) -> Option<usize> {
        crate::studio::ASPECTS
            .iter()
            .position(|(label, _, _)| *label == self.aspect)
    }
}

/// A built-in style.
pub struct Style {
    pub settings: Settings,
    settings_text: &'static str,
    guide: &'static str,
}

/// The Thai text in Thai, the English otherwise or where there is none.
fn in_language<'a>(english: &'a str, thai: &'a str) -> &'a str {
    if !thai.is_empty() && crate::i18n::current() == "th" {
        thai
    } else {
        english
    }
}

/// Every built-in style, in the order the New Project sheet lists them.
/// One that does not parse is left out and logged; the tests keep that
/// from shipping.
pub fn all() -> Vec<Style> {
    BUILT_IN
        .iter()
        .filter_map(
            |(id, settings_text, guide)| match toml::from_str(settings_text) {
                Ok(settings) => Some(Style {
                    settings,
                    settings_text,
                    guide,
                }),
                Err(error) => {
                    log::error!("style {id}: {error}");
                    None
                }
            },
        )
        .collect()
}

/// Lays `style` into a new project's `folder`: its settings, its recipe
/// as the folder's `CLAUDE.md`, and the folders the agent works in. A
/// `CLAUDE.md` already there is kept: the folder's own notes win.
pub fn apply(style: &Style, folder: &Path) -> Result<(), String> {
    let could_not =
        |what: &Path, error: std::io::Error| format!("could not write {}: {error}", what.display());
    for name in FOLDERS {
        let path = folder.join(name);
        std::fs::create_dir_all(&path).map_err(|error| could_not(&path, error))?;
    }
    let settings = folder.join(SETTINGS);
    std::fs::write(&settings, style.settings_text).map_err(|error| could_not(&settings, error))?;
    let guide = folder.join("CLAUDE.md");
    if !guide.exists() {
        std::fs::write(&guide, style.guide).map_err(|error| could_not(&guide, error))?;
    }
    Ok(())
}

/// The style settings of the project at `folder`, if it was made with one.
pub fn of_project(folder: &Path) -> Option<Settings> {
    let text = std::fs::read_to_string(folder.join(SETTINGS)).ok()?;
    match toml::from_str(&text) {
        Ok(settings) => Some(settings),
        Err(error) => {
            log::warn!("{}: {error}", folder.join(SETTINGS).display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty folder of this test's own, left for the system's sweep.
    fn scratch(name: &str) -> std::path::PathBuf {
        let folder =
            std::env::temp_dir().join(format!("concat-styles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("scratch");
        folder
    }

    #[test]
    fn every_built_in_style_reads() {
        let styles = all();
        assert_eq!(styles.len(), BUILT_IN.len(), "one did not parse");
        for style in &styles {
            let name = &style.settings.name;
            assert!(!name.is_empty());
            assert!(
                style.settings.aspect_index().is_some(),
                "{name}: unknown aspect {:?}",
                style.settings.aspect
            );
            assert!(!style.settings.starters.is_empty(), "{name}");
            assert!(!style.guide.is_empty(), "{name}");
        }
    }

    #[test]
    fn a_style_lays_its_files_into_a_project() {
        let folder = scratch("lays");
        let style = all().into_iter().next().expect("a style");
        apply(&style, &folder).expect("applied");
        for name in FOLDERS {
            assert!(folder.join(name).is_dir(), "{name}");
        }
        assert_eq!(
            std::fs::read_to_string(folder.join("CLAUDE.md")).expect("guide"),
            style.guide
        );
        assert_eq!(of_project(&folder), Some(style.settings));
    }

    #[test]
    fn a_projects_own_notes_are_kept() {
        let folder = scratch("kept");
        std::fs::write(folder.join("CLAUDE.md"), "mine").expect("written");
        let style = all().into_iter().next().expect("a style");
        apply(&style, &folder).expect("applied");
        assert_eq!(
            std::fs::read_to_string(folder.join("CLAUDE.md")).expect("guide"),
            "mine"
        );
    }
}
