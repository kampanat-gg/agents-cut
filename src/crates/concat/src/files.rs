// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The Files pane's side: the open project's folder as a list of lines,
//! read again every couple of seconds so what the agent makes shows up
//! while it makes it.
//!
//! Folders at the top open on their own and deeper ones closed - a motion
//! sequence is a folder of a hundred frames - and only an open folder is
//! read past its count. Pictures get a small thumbnail, made on a thread
//! of its own and kept until the file changes.
//!
//! A file goes onto the timeline by the payload `file:<path>`, the path
//! relative to the project; it joins the bin on the way (see
//! [`Studio::place_file`]), so the agent's pictures need no import.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use slint::{Model, VecModel};

use crate::host::Shell;
use crate::i18n::tf;
use crate::studio::Studio;
use crate::ui::{FileRow, StoryBeat, VideoCard};

/// How often the open project's folder is read again.
const EVERY: Duration = Duration::from_secs(2);

/// What a project's folder holds that is the editor's, not the user's.
const HIDDEN: [&str; 3] = ["cache", "concat.json", "concat.lock"];

/// Lines past this many are not listed: a folder that size is not one to
/// read through in a side panel.
const MOST: usize = 2000;

/// Toggles of one folder this close together are one.
const TWICE: Duration = Duration::from_millis(500);

/// A thumbnail's longer side, in pixels: twice the 20pt it is drawn at.
const THUMB: u32 = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Text,
    Other,
}

impl Kind {
    pub fn of(path: &Path) -> Kind {
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" => Kind::Image,
            "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" => Kind::Video,
            "mp3" | "wav" | "aac" | "m4a" | "flac" | "ogg" => Kind::Audio,
            "md" | "txt" | "toml" | "json" | "srt" | "vtt" | "csv" => Kind::Text,
            _ => Kind::Other,
        }
    }

    /// The timeline can take it.
    pub fn placeable(self) -> bool {
        matches!(self, Kind::Image | Kind::Video | Kind::Audio)
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
            Kind::Audio => "audio",
            Kind::Text => "text",
            Kind::Other => "other",
        }
    }
}

/// One line as read from the disk.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    /// Relative to the project, with forward slashes.
    path: String,
    name: String,
    depth: usize,
    folder: bool,
    /// A file's size; a folder's count of what is in it.
    size: u64,
    modified: Option<SystemTime>,
}

pub struct Files {
    rows: Rc<VecModel<FileRow>>,
    /// The project's videos, one a timeline: the side panel's cards.
    videos: Rc<VecModel<VideoCard>>,
    /// The storyboard open in the centre - its episode's folder and the
    /// time of the file its beats were read from - and its beats.
    story: Option<(String, Option<SystemTime>)>,
    beats: Rc<VecModel<StoryBeat>>,
    folder: Option<PathBuf>,
    /// When each file in the bin was last changed, as last seen: a file
    /// changed since is drawn again (see [`Studio::poll_files`]).
    stamps: HashMap<PathBuf, SystemTime>,
    listed: Vec<Entry>,
    /// Folders listed open, and every folder seen, so one that appears
    /// later is opened or not by its depth only the first time.
    open: HashSet<String>,
    seen: HashSet<String>,
    /// Each picture's thumbnail, with the time of the file it was made from.
    thumbs: HashMap<String, (Option<SystemTime>, slint::Image)>,
    /// Pictures a thumbnail is being made for.
    asked: HashSet<String>,
    /// Each video's and sound's length, by path and the file's time: a drag
    /// asks on every move over the lanes.
    lengths: RefCell<HashMap<String, (Option<SystemTime>, f64)>>,
    /// Each timeline's episode, by its name: the folder and its brief,
    /// with the times of the `episodes` folder and the brief they were
    /// read at. The cards are drawn on every publish of the lanes, and
    /// two looks at the disk are all an unchanged one costs.
    episodes: RefCell<HashMap<String, Episode>>,
    /// The folder toggled last, and when.
    toggled: Option<(String, Instant)>,
    timer: slint::Timer,
}

impl Default for Files {
    fn default() -> Self {
        Files {
            rows: Rc::new(VecModel::default()),
            videos: Rc::new(VecModel::default()),
            story: None,
            beats: Rc::new(VecModel::default()),
            folder: None,
            stamps: HashMap::new(),
            listed: Vec::new(),
            open: HashSet::new(),
            seen: HashSet::new(),
            thumbs: HashMap::new(),
            asked: HashSet::new(),
            lengths: RefCell::new(HashMap::new()),
            episodes: RefCell::new(HashMap::new()),
            toggled: None,
            timer: slint::Timer::default(),
        }
    }
}

impl Files {
    /// The lines, handed to the pane once.
    pub fn rows(&self) -> Rc<VecModel<FileRow>> {
        Rc::clone(&self.rows)
    }

    /// The storyboard's beats, handed to its view once.
    pub fn beats(&self) -> Rc<VecModel<StoryBeat>> {
        Rc::clone(&self.beats)
    }

    /// Shows the storyboard of the episode in `folder`, and keeps it
    /// current as its file changes.
    pub fn open_storyboard(&mut self, folder: &str) {
        self.story = Some((folder.to_owned(), None));
        self.reread_storyboard();
    }

    pub fn close_storyboard(&mut self) {
        self.story = None;
        self.beats.set_vec(Vec::new());
    }

    /// Reads the open storyboard again when its file has changed since.
    fn reread_storyboard(&mut self) {
        let (Some(project), Some((folder, read))) = (self.folder.clone(), self.story.clone())
        else {
            return;
        };
        let file = project.join(&folder).join("storyboard.md");
        let modified = std::fs::metadata(&file)
            .ok()
            .and_then(|meta| meta.modified().ok());
        if read.is_some() && read == modified {
            return;
        }
        self.story = Some((folder.clone(), modified));
        let beats: Vec<StoryBeat> = crate::episodes::storyboard(&project, &folder)
            .into_iter()
            .map(|beat| {
                let picture = beat
                    .image
                    .as_ref()
                    .and_then(|image| thumbnail_of(&project.join(&folder).join(image), STORY_THUMB))
                    .map(slint::Image::from_rgba8);
                StoryBeat {
                    time: beat.time.into(),
                    title: beat.title.into(),
                    voice: beat.voice.into(),
                    picture: beat.picture.into(),
                    motion: beat.motion.into(),
                    sound: beat.sound.into(),
                    has_image: picture.is_some(),
                    image: picture.unwrap_or_default(),
                    graphic: beat.graphic,
                }
            })
            .collect();
        self.beats.set_vec(beats);
    }

    /// The video cards, handed to the side panel once.
    pub fn videos(&self) -> Rc<VecModel<VideoCard>> {
        Rc::clone(&self.videos)
    }

    /// Lists the project at `folder`, and keeps listing it.
    pub fn load(&mut self, folder: &Path) {
        *self = Files {
            rows: Rc::clone(&self.rows),
            videos: Rc::clone(&self.videos),
            beats: Rc::clone(&self.beats),
            folder: Some(folder.to_path_buf()),
            ..Files::default()
        };
        self.refresh();
        self.timer.start(slint::TimerMode::Repeated, EVERY, || {
            Shell::with(|shell, _| shell.studio.borrow_mut().poll_files());
        });
    }

    /// The file at `path`, relative to the project, as a full path.
    pub fn full(&self, path: &str) -> Option<PathBuf> {
        let path = Path::new(path);
        // Never out of the folder: the path comes back from the pane.
        if path.is_absolute() || path.components().any(|part| part.as_os_str() == "..") {
            return None;
        }
        Some(self.folder.as_ref()?.join(path))
    }

    /// Opens a folder's line, or closes it.
    pub fn toggle(&mut self, path: &str) {
        // A double-click is two clicks and a double-click, and means
        // one toggle.
        let now = Instant::now();
        if let Some((last, at)) = &self.toggled
            && last == path
            && now.duration_since(*at) < TWICE
        {
            return;
        }
        self.toggled = Some((path.to_owned(), now));
        if !self.open.remove(path) {
            self.open.insert(path.to_owned());
        }
        self.refresh();
    }

    /// Reads the folder again, and redraws the list if anything changed.
    pub fn refresh(&mut self) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        self.reread_storyboard();
        let mut listed = Vec::new();
        self.walk(&folder, "", 0, &mut listed);
        if listed != self.listed {
            self.listed = listed;
            self.thumbs
                .retain(|path, _| self.listed.iter().any(|entry| entry.path == *path));
            self.ask_thumbs();
            self.draw();
        }
    }

    fn walk(&mut self, dir: &Path, prefix: &str, depth: usize, out: &mut Vec<Entry>) {
        for (name, path, folder) in children(dir, depth == 0) {
            if out.len() >= MOST {
                return;
            }
            let relative = format!("{prefix}{name}");
            let meta = std::fs::metadata(&path).ok();
            if folder {
                if self.seen.insert(relative.clone()) && depth == 0 {
                    self.open.insert(relative.clone());
                }
                out.push(Entry {
                    path: relative.clone(),
                    name,
                    depth,
                    folder: true,
                    size: children(&path, false).len() as u64,
                    modified: None,
                });
                if self.open.contains(&relative) {
                    self.walk(&path, &format!("{relative}/"), depth + 1, out);
                }
            } else {
                out.push(Entry {
                    path: relative,
                    name,
                    depth,
                    folder: false,
                    size: meta.as_ref().map_or(0, std::fs::Metadata::len),
                    modified: meta.and_then(|meta| meta.modified().ok()),
                });
            }
        }
    }

    /// Starts a thread making the thumbnails the listed pictures lack.
    fn ask_thumbs(&mut self) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        let wanted: Vec<(String, Option<SystemTime>)> = self
            .listed
            .iter()
            .filter(|entry| !entry.folder && Kind::of(Path::new(&entry.name)) == Kind::Image)
            .filter(|entry| {
                self.thumbs
                    .get(&entry.path)
                    .is_none_or(|(made, _)| *made != entry.modified)
            })
            .filter(|entry| !self.asked.contains(&entry.path))
            .map(|entry| (entry.path.clone(), entry.modified))
            .collect();
        if wanted.is_empty() {
            return;
        }
        self.asked
            .extend(wanted.iter().map(|(path, _)| path.clone()));
        let spawned = std::thread::Builder::new()
            .name("thumbnails".to_owned())
            .spawn(move || {
                for (path, modified) in wanted {
                    let pixels = thumbnail(&folder.join(&path));
                    let _ = slint::invoke_from_event_loop(move || {
                        Shell::with(|shell, _| {
                            shell
                                .studio
                                .borrow_mut()
                                .files
                                .thumb_made(path, modified, pixels);
                        });
                    });
                }
            });
        if spawned.is_err() {
            self.asked.clear();
        }
    }

    fn thumb_made(
        &mut self,
        path: String,
        modified: Option<SystemTime>,
        pixels: Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>>,
    ) {
        self.asked.remove(&path);
        let image = pixels.map(slint::Image::from_rgba8).unwrap_or_default();
        let row = self.listed.iter().position(|entry| entry.path == path);
        self.thumbs.insert(path, (modified, image.clone()));
        if let Some(row) = row
            && let Some(mut line) = self.rows.row_data(row)
        {
            line.thumbed = image.size().width > 0;
            line.thumb = image;
            self.rows.set_row_data(row, line);
        }
    }

    fn draw(&self) {
        let lines: Vec<FileRow> = self
            .listed
            .iter()
            .map(|entry| {
                let kind = Kind::of(Path::new(&entry.name));
                let thumb = self
                    .thumbs
                    .get(&entry.path)
                    .map(|(_, image)| image.clone())
                    .unwrap_or_default();
                FileRow {
                    path: entry.path.as_str().into(),
                    name: entry.name.as_str().into(),
                    depth: entry.depth as i32,
                    folder: entry.folder,
                    open: entry.folder && self.open.contains(&entry.path),
                    kind: if entry.folder { "folder" } else { kind.name() }.into(),
                    meta: if entry.folder {
                        entry.size.to_string()
                    } else {
                        crate::format::bytes(entry.size as f32)
                    }
                    .into(),
                    placeable: !entry.folder && kind.placeable(),
                    code: !entry.folder
                        && self
                            .full(&entry.path)
                            .is_some_and(|full| concat_media::svg::is_code(&full)),
                    thumbed: thumb.size().width > 0,
                    thumb,
                }
            })
            .collect();
        self.rows.set_vec(lines);
    }

    /// How long a video or a sound runs, in seconds, asked of the file once
    /// for each time it changes.
    pub fn length(&self, path: &str) -> Option<f64> {
        let full = self.full(path)?;
        let modified = std::fs::metadata(&full).ok()?.modified().ok();
        if let Some((made, seconds)) = self.lengths.borrow().get(path)
            && *made == modified
        {
            return Some(*seconds);
        }
        let seconds = concat_host::media::probe(&full.to_string_lossy())
            .ok()?
            .duration?;
        self.lengths
            .borrow_mut()
            .insert(path.to_owned(), (modified, seconds));
        Some(seconds)
    }
}

/// Opens a file of the project's in the application the system gives it,
/// or with an empty path the project's folder itself.
pub fn open(studio: &mut Studio, path: &str) {
    let Some(full) = studio.files.full(path) else {
        return;
    };
    if let Err(error) = opener::open(&full) {
        studio.notify(&error.to_string(), true);
    }
}

/// What is in `dir` worth a line, folders first and each lot by name: its
/// name, its path, and whether it is a folder. `top` leaves out the
/// editor's own files, which only the project's folder itself has.
fn children(dir: &Path, top: bool) -> Vec<(String, PathBuf, bool)> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf, bool)> = read
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || (top && HIDDEN.contains(&name.as_str())) {
                return None;
            }
            let folder = entry.file_type().ok()?.is_dir();
            Some((name, entry.path(), folder))
        })
        .collect();
    found.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    found
}

/// A frame rate as people write it: "24", "29.97".
fn rate(fps: f64) -> String {
    let rounded = (fps * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded}")
    }
}

/// A storyboard picture's longer side, in pixels: twice what it is drawn at.
const STORY_THUMB: u32 = 320;

/// A picture made small, or None for one that does not read.
fn thumbnail(path: &Path) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    thumbnail_of(path, THUMB)
}

fn thumbnail_of(path: &Path, side: u32) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    let small = image::open(path).ok()?.thumbnail(side, side).into_rgba8();
    Some(slint::SharedPixelBuffer::clone_from_slice(
        small.as_raw(),
        small.width(),
        small.height(),
    ))
}

/// A timeline's episode, as last read: see [`Files::episodes`].
#[derive(Clone, Default)]
struct Episode {
    listing: Option<SystemTime>,
    written: Option<SystemTime>,
    folder: Option<String>,
    brief: crate::episodes::Brief,
}

impl Files {
    /// The episode the timeline named `name` is: its folder relative to
    /// the project, and what its brief says. Read again only when the
    /// `episodes` folder or the brief has changed.
    pub fn episode(&self, name: &str) -> (Option<String>, crate::episodes::Brief) {
        let Some(project) = self.folder.as_ref() else {
            return (None, crate::episodes::Brief::default());
        };
        let modified = |path: &Path| {
            std::fs::metadata(path)
                .ok()
                .and_then(|meta| meta.modified().ok())
        };
        let listing = modified(&project.join(crate::episodes::FOLDER));
        let mut cache = self.episodes.borrow_mut();
        let entry = cache.entry(name.to_owned()).or_default();
        if entry.listing != listing || entry.folder.is_none() && listing.is_some() {
            entry.listing = listing;
            entry.folder = crate::episodes::folder_of(project, name);
            entry.written = None;
        }
        if let Some(folder) = &entry.folder {
            let written = modified(&project.join(folder).join("brief.md"));
            if written != entry.written {
                entry.written = written;
                entry.brief = crate::episodes::brief(project, folder);
            }
        }
        (entry.folder.clone(), entry.brief.clone())
    }
}

impl Studio {
    /// What the timer does every few seconds: lists the folder again, and
    /// lets go of what is cached of any file in the bin that changed on
    /// disk - a code clip or an SVG rewritten, a picture Codex drew
    /// again under its old name - so the monitor shows it as it is now.
    pub fn poll_files(&mut self) {
        self.files.refresh();
        let Some(folder) = self.files.folder.clone() else {
            return;
        };
        let mut changed = Vec::new();
        let mut stamps = HashMap::new();
        for item in &self.project().media {
            let path = folder.join(&item.path);
            let Some(stamp) = std::fs::metadata(&path)
                .ok()
                .and_then(|meta| meta.modified().ok())
            else {
                continue;
            };
            if self
                .files
                .stamps
                .get(&path)
                .is_some_and(|seen| *seen != stamp)
            {
                changed.push(path.clone());
            }
            stamps.insert(path, stamp);
        }
        self.files.stamps = stamps;
        if changed.is_empty() {
            return;
        }
        let pool = concat_host::scheduler().pool();
        for path in &changed {
            log::info!(
                "files: {} changed on disk; drawing it again",
                path.display()
            );
            pool.forget(path);
        }
        self.host.monitor.forget_plan();
        self.request_preview();
    }

    /// The side panel's cards: each timeline, with the picture of the
    /// earliest clip on it that has one - the bin's own thumbnail of its
    /// file, made for the bin whether it is on screen or not.
    pub fn publish_videos(&self) {
        let project = self.project();
        let cards: Vec<VideoCard> = project
            .timelines
            .iter()
            .map(|timeline| {
                let thumb = timeline
                    .clips
                    .iter()
                    .filter(|clip| {
                        matches!(
                            clip.kind,
                            concat_project::model::ClipKind::Video
                                | concat_project::model::ClipKind::Image
                        )
                    })
                    .min_by(|a, b| a.start.total_cmp(&b.start))
                    .and_then(|clip| self.media.thumbs.get(&clip.media_id).cloned());
                let (folder, brief) = self.files.episode(&timeline.name);
                let boarded = match (&folder, self.files.folder.as_ref()) {
                    (Some(folder), Some(project)) => {
                        project.join(folder).join("storyboard.md").is_file()
                    }
                    _ => false,
                };
                VideoCard {
                    boarded,
                    folder: folder.unwrap_or_default().into(),
                    synopsis: brief.synopsis.into(),
                    status: brief.status.into(),
                    name: timeline.name.as_str().into(),
                    seconds: timeline.duration() as f32,
                    frame: format!(
                        "{} × {} · {} fps",
                        timeline.video.width,
                        timeline.video.height,
                        rate(timeline.video.rate())
                    )
                    .into(),
                    aspect: timeline.video.width as f32 / timeline.video.height.max(1) as f32,
                    thumbed: thumb.is_some(),
                    thumb: thumb.unwrap_or_default(),
                    current: timeline.id == project.active_timeline_id,
                }
            })
            .collect();
        // Each episode with a folder has a conversation of its own.
        let episodes: Vec<(String, String)> = cards
            .iter()
            .filter(|card| !card.folder.is_empty())
            .map(|card| (card.folder.to_string(), card.name.to_string()))
            .collect();
        self.agent.list_threads(episodes);
        crate::studio::sync(&self.files.videos, cards);
    }

    /// The side panel's New video: a timeline, on screen, started as the
    /// next episode with its conversation open, so the user can say what
    /// it is about straight away.
    pub fn new_video(&mut self) {
        let Some(id) = self.apply(concat_project::Command::AddTimeline) else {
            return;
        };
        self.selection.clear();
        self.apply(concat_project::Command::SelectTimeline { timeline_id: id });
        self.follow_video();
    }

    /// The conversation a project opens on: the episode's of the video on
    /// screen when it is one, the channel's otherwise. Nothing is started
    /// or renamed: opening a project only shows what is there.
    pub fn open_video_chat(&mut self) {
        let name = {
            let doc = self.project();
            let Some(timeline) = doc
                .timelines
                .iter()
                .find(|timeline| timeline.id == doc.active_timeline_id)
            else {
                return;
            };
            timeline.name.clone()
        };
        if let Some(folder) = self.files.episode(&name).0 {
            self.agent.switch_thread(&folder, &name);
            self.publish_videos();
        }
    }

    /// Puts the video on screen's conversation in the sidebar: its
    /// episode's, after starting it as the next episode - its number on
    /// its name, its folder and brief - when it is not one yet. A project
    /// with no folder has nowhere to keep one, and stays as it is.
    pub fn follow_video(&mut self) {
        let Some(project) = self.files.folder.clone() else {
            return;
        };
        let (id, name) = {
            let doc = self.project();
            let Some(timeline) = doc
                .timelines
                .iter()
                .find(|timeline| timeline.id == doc.active_timeline_id)
            else {
                return;
            };
            (timeline.id.clone(), timeline.name.clone())
        };
        let (folder, name) = match self.files.episode(&name).0 {
            Some(folder) => (folder, name),
            None => {
                let next = crate::episodes::next_number(
                    &project,
                    self.project()
                        .timelines
                        .iter()
                        .map(|timeline| timeline.name.as_str()),
                );
                let (number, renamed) = crate::episodes::named(&name, next);
                let folder = match crate::episodes::start(&project, number) {
                    Ok(folder) => folder,
                    Err(error) => {
                        self.notify(&error, true);
                        return;
                    }
                };
                if renamed != name {
                    self.apply(concat_project::Command::RenameTimeline {
                        timeline_id: id,
                        name: renamed.clone(),
                    });
                }
                (folder, renamed)
            }
        };
        if !self.agent.switch_thread(&folder, &name) {
            self.notify(&crate::i18n::t("agent.busySwitch"), false);
        }
        self.publish_videos();
    }

    /// A file of the project's laid on the timeline: onto `track_id` at
    /// `start`, or at `start` on the first lane with room. It joins the
    /// bin first if it is not in it.
    pub fn place_file(&mut self, path: &str, track_id: Option<String>, start: f64) {
        let media_id = match self.adopt_file(path) {
            Ok(id) => id,
            Err(error) => {
                self.notify(&error, true);
                return;
            }
        };
        self.close_source();
        let created = match track_id {
            Some(track_id) => self.apply(concat_project::Command::AddClip {
                media_id,
                track_id,
                start,
                ripple: true,
            }),
            None => self.apply(concat_project::Command::AddClipAtFirstFree { media_id, start }),
        };
        if let Some(id) = created {
            self.selection = vec![id];
        }
    }

    /// The bin's id for a file of the project's, imported when it is not
    /// in the bin yet.
    fn adopt_file(&mut self, path: &str) -> Result<String, String> {
        let full = self
            .files
            .full(path)
            .ok_or_else(|| tf("files.couldNotAdd", &[&path, &"?"]))?;
        let text = full.to_string_lossy().into_owned();
        let same = |item: &concat_project::model::MediaItem| {
            item.path == text
                || Path::new(&item.path).canonicalize().ok() == full.canonicalize().ok()
        };
        if let Some(item) = self.project().media.iter().find(|item| same(item)) {
            return Ok(item.id.clone());
        }
        let summary = concat_host::media::probe(&text)
            .map_err(|error| tf("files.couldNotAdd", &[&path, &error]))?;
        let created = self.apply(concat_project::Command::AddMedia {
            item: summary.to_new_media(),
        });
        created
            .or_else(|| {
                self.project()
                    .media
                    .iter()
                    .find(|item| item.path == text)
                    .map(|item| item.id.clone())
            })
            .ok_or_else(|| tf("files.couldNotAdd", &[&path, &"?"]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty folder of this test's own, left for the system's sweep.
    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("concat-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("scratch");
        folder
    }

    #[test]
    fn the_list_leaves_out_the_editors_files_and_opens_only_the_top() {
        let folder = scratch("list");
        for dir in ["cache/art", "generated/kraken/frames", "assets"] {
            std::fs::create_dir_all(folder.join(dir)).expect("dir");
        }
        for file in [
            "concat.json",
            ".DS_Store",
            "CLAUDE.md",
            "generated/sea.png",
            "generated/kraken/frames/0001.png",
            "assets/song.mp3",
        ] {
            std::fs::write(folder.join(file), b"x").expect("file");
        }
        let mut files = Files {
            folder: Some(folder.clone()),
            ..Files::default()
        };
        let mut listed = Vec::new();
        files.walk(&folder, "", 0, &mut listed);
        let paths: Vec<&str> = listed.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "assets",
                "assets/song.mp3",
                "generated",
                "generated/kraken",
                "generated/sea.png",
                "CLAUDE.md",
            ]
        );
        let kraken = listed.iter().find(|entry| entry.path == "generated/kraken");
        assert_eq!(
            kraken.map(|entry| entry.size),
            Some(1),
            "a closed folder's count"
        );
    }

    #[test]
    fn a_path_from_the_pane_stays_in_the_project() {
        let files = Files {
            folder: Some(PathBuf::from("/projects/one")),
            ..Files::default()
        };
        assert_eq!(
            files.full("generated/a.png"),
            Some(PathBuf::from("/projects/one/generated/a.png"))
        );
        assert_eq!(files.full("../two/a.png"), None);
        assert_eq!(files.full("/etc/hosts"), None);
    }

    #[test]
    fn a_rate_reads_as_people_write_it() {
        assert_eq!(rate(24.0), "24");
        assert_eq!(rate(30000.0 / 1001.0), "29.97");
    }

    #[test]
    fn kinds_go_by_extension() {
        assert_eq!(Kind::of(Path::new("a.PNG")), Kind::Image);
        assert_eq!(Kind::of(Path::new("a.webm")), Kind::Video);
        assert_eq!(Kind::of(Path::new("a.mp3")), Kind::Audio);
        assert_eq!(Kind::of(Path::new("CLAUDE.md")), Kind::Text);
        assert!(!Kind::of(Path::new("style.toml")).placeable());
    }
}
