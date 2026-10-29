// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! SVG, which FFmpeg cannot draw.
//!
//! FFmpeg reads an SVG only through librsvg, which no build of Concat
//! links; without it the file opened as a picture with no size, and the
//! import said "video stream has no usable size". The SVG is drawn once
//! with resvg - already in the build, through Slint - into a PNG in the
//! temporary folder, and everything that reads pictures reads that: the
//! probe, the monitor, the cards, proxies and export alike. The project
//! keeps the SVG's own path, so a changed file is drawn again.
//!
//! An SVG with `{{ expression }}` holes is a code clip (template.rs): its
//! holes are filled with the time and it is drawn once a frame, for as
//! long as its `data-duration` says, into a video with an alpha channel
//! in the same folder. To everything that reads it, it is then a clip
//! that moves.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use concat_core::frame::Frame;
use concat_core::time::{FrameRate, Rational};
use resvg::usvg::fontdb;

use crate::encode::{EncodeOptions, Encoder, FrameSink, VideoCodec};
use crate::error::{Error, Result};
use crate::template::{Moment, Template};

/// The long side of the drawing, in pixels: a 4K frame's width, so a logo
/// laid over a UHD export is as sharp as the canvas. A vector has no size
/// of its own worth keeping - an icon says 24 px or 800 px for the same
/// shape.
const LONG_SIDE: f32 = 3840.0;

/// A code clip's long side: a 1080 × 1920 frame's. It is drawn once a
/// frame, so it is drawn at the size it is shown.
const MOTION_LONG_SIDE: f32 = 1920.0;

/// How long a code clip runs, and how many frames a second, when its
/// `<svg>` does not say (`data-duration`, `data-fps`).
const MOTION_SECONDS: f64 = 4.0;
const MOTION_FPS: f64 = 30.0;
/// The longest a code clip runs: thirty seconds is 900 frames, drawn in
/// about half a minute before it plays. Longer is a job for a timeline.
const MOTION_MOST_SECONDS: f64 = 30.0;

/// Whether `path` names an SVG, by its extension.
pub fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("svg") || extension.eq_ignore_ascii_case("svgz")
        })
}

/// What FFmpeg should open for `path`: the SVG's drawing, drawn now if it
/// is not drawn already, or `path` itself when it is not an SVG.
pub fn readable(path: &Path) -> Result<PathBuf> {
    if !is_svg(path) {
        return Ok(path.to_path_buf());
    }
    let fail = |detail: String| Error::Probe {
        path: path.to_path_buf(),
        detail,
    };
    let metadata = std::fs::metadata(path).map_err(|error| fail(error.to_string()))?;
    let drawn = drawing_of(path, &metadata);
    if drawn.is_file() {
        return Ok(drawn);
    }
    let baked = drawn.with_extension(motion_codec().container());
    if baked.is_file() {
        return Ok(baked);
    }
    let bytes = std::fs::read(path).map_err(|error| fail(error.to_string()))?;
    if let Ok(source) = std::str::from_utf8(&bytes)
        && crate::template::is_animated(source)
    {
        // One bake at a time: it takes seconds, and every reader that
        // asks meanwhile wants the same file, there when it is done.
        static BAKING: Mutex<()> = Mutex::new(());
        let _turn = BAKING.lock().unwrap_or_else(|e| e.into_inner());
        if baked.is_file() {
            return Ok(baked);
        }
        if let Some(dir) = baked.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let partial = baked.with_extension(format!(
            "{}.part.{}",
            std::process::id(),
            motion_codec().container()
        ));
        if let Err(why) = bake(source, path.parent(), &partial) {
            let _ = std::fs::remove_file(&partial);
            return Err(fail(why));
        }
        std::fs::rename(&partial, &baked).map_err(|error| fail(error.to_string()))?;
        return Ok(baked);
    }
    let png = draw(&bytes, path.parent()).map_err(fail)?;
    if let Some(dir) = drawn.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Written beside and renamed over, so a reader on another thread never
    // opens half a PNG. Beside under a name of this call's own: the
    // monitor, the prefetcher and an export's decoders all open the file,
    // and when its drawing is not there yet - a new SVG, an edited one, or
    // one the system's cleaner took out of the temporary folder - several
    // draw it at once. Under one name per process they wrote into each
    // other's file, so one renamed a PNG another was still writing, which
    // then stood as the drawing for good, and the others' renames found
    // nothing to move and failed the decoder: the SVG was missing from the
    // export.
    static DRAWING: AtomicU64 = AtomicU64::new(0);
    let partial = drawn.with_extension(format!(
        "{}-{}.part",
        std::process::id(),
        DRAWING.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&partial, png).map_err(|error| fail(error.to_string()))?;
    if let Err(error) = std::fs::rename(&partial, &drawn) {
        let _ = std::fs::remove_file(&partial);
        // Another drawing of the same file landed first; it is as good.
        if !drawn.is_file() {
            return Err(fail(error.to_string()));
        }
    }
    Ok(drawn)
}

/// Where the drawing of `path`, as it is now, lives: named for its path,
/// size and modification time, so an edited SVG gets a new drawing.
fn drawing_of(path: &Path, metadata: &std::fs::Metadata) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata.modified().ok().hash(&mut hasher);
    LONG_SIDE.to_bits().hash(&mut hasher);
    std::env::temp_dir()
        .join("concat-svg")
        .join(format!("{:016x}.png", hasher.finish()))
}

/// The SVG in `bytes`, drawn with its long side at [`LONG_SIDE`], as a PNG.
/// `folder` is the one the file is in, where a picture it names by a
/// relative path - `<image href="photo.png">`, as a design tool writes a
/// placed bitmap - is found; without it such pictures were looked for
/// beside the app and left out of the drawing.
fn draw(bytes: &[u8], folder: Option<&Path>) -> std::result::Result<Vec<u8>, String> {
    let options = options(folder);
    let tree = resvg::usvg::Tree::from_data(bytes, &options)
        .map_err(|error| format!("not an SVG Concat can read: {error}"))?;
    let size = tree.size();
    let scale = LONG_SIDE / size.width().max(size.height());
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("an SVG of {width}×{height} is too large to draw"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|error| format!("could not write the SVG's drawing: {error}"))
}

/// How an SVG is read: pictures it names beside it, and the fonts - the
/// machine's and every face titles can use, so a code clip's numbers can
/// be set in the captions' Kanit. The fonts are gathered once.
fn options(folder: Option<&Path>) -> resvg::usvg::Options<'static> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut fonts = fontdb::Database::new();
        fonts.load_system_fonts();
        for face in concat_text::bundled_faces() {
            fonts.load_font_source(fontdb::Source::Binary(Arc::new(face)));
        }
        Arc::new(fonts)
    });
    resvg::usvg::Options {
        resources_dir: folder.map(Path::to_path_buf),
        fontdb: Arc::clone(fonts),
        ..resvg::usvg::Options::default()
    }
}

/// What a code clip is written in: VP9 with alpha, small, where this
/// FFmpeg has libvpx; ProRes 4444, which every FFmpeg writes, where not.
fn motion_codec() -> VideoCodec {
    if VideoCodec::Vp9Alpha.encodable() {
        VideoCodec::Vp9Alpha
    } else {
        VideoCodec::ProRes4444
    }
}

/// A number an `<svg>` gives in `name="..."`, when it gives one.
fn attribute(source: &str, name: &str) -> Option<f64> {
    let start = source.find(&format!("{name}=\""))? + name.len() + 2;
    let end = source[start..].find('"')? + start;
    source[start..end].trim().trim_end_matches('s').parse().ok()
}

/// How long a code clip runs and how many frames a second, from its
/// `<svg>`: at most [`MOTION_MOST_SECONDS`], since every frame is drawn
/// before the clip can play, and 1 to 60 frames a second.
fn timing(source: &str) -> (f64, f64) {
    let seconds = attribute(source, "data-duration")
        .filter(|seconds| seconds.is_finite())
        .unwrap_or(MOTION_SECONDS)
        .clamp(0.1, MOTION_MOST_SECONDS);
    let fps = attribute(source, "data-fps")
        .filter(|fps| fps.is_finite())
        .unwrap_or(MOTION_FPS)
        .round()
        .clamp(1.0, 60.0);
    (seconds, fps)
}

/// Draws the code clip in `source` a frame at a time into `out`.
fn bake(source: &str, folder: Option<&Path>, out: &Path) -> std::result::Result<(), String> {
    let template = Template::parse(source)?;
    let (seconds, fps) = timing(source);
    let frames = ((seconds * fps).round() as u64).max(1);
    let options = options(folder);
    let read = |frame: u64| {
        // The last frame is the end itself, so a count reaches its number.
        let t = if frame + 1 == frames {
            seconds
        } else {
            frame as f64 / fps
        };
        let filled = template.fill(Moment {
            t,
            d: seconds,
            frame,
            fps,
        });
        resvg::usvg::Tree::from_data(filled.as_bytes(), &options)
            .map_err(|error| format!("frame {frame}: not an SVG Concat can read: {error}"))
    };
    let first = read(0)?;
    let size = first.size();
    let scale = MOTION_LONG_SIDE / size.width().max(size.height());
    // Encoders take even sizes.
    let even = |side: f32| (((side * scale) / 2.0).round() as u32).max(1) * 2;
    let (width, height) = (even(size.width()), even(size.height()));
    let (sx, sy) = (width as f32 / size.width(), height as f32 / size.height());
    let encode = EncodeOptions {
        codec: motion_codec(),
        preset: "ultrafast".to_owned(),
        threads: 2,
        ..EncodeOptions::default()
    };
    let rate = FrameRate::new(Rational::new(fps as i64, 1));
    let mut encoder =
        Encoder::create(out, width, height, rate, &encode).map_err(|error| error.to_string())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("a code clip of {width}×{height} is too large to draw"))?;
    let mut tree = Some(first);
    for frame in 0..frames {
        let this = match tree.take() {
            Some(tree) => tree,
            None => read(frame)?,
        };
        pixmap.fill(resvg::tiny_skia::Color::TRANSPARENT);
        resvg::render(
            &this,
            resvg::tiny_skia::Transform::from_scale(sx, sy),
            &mut pixmap.as_mut(),
        );
        let mut rgba = Vec::with_capacity(pixmap.pixels().len() * 4);
        for pixel in pixmap.pixels() {
            let straight = pixel.demultiply();
            rgba.extend_from_slice(&[
                straight.red(),
                straight.green(),
                straight.blue(),
                straight.alpha(),
            ]);
        }
        let picture = Frame::from_rgba(width, height, rgba)
            .ok_or_else(|| "a frame of the wrong size".to_owned())?;
        encoder
            .write_frame(&picture)
            .map_err(|error| error.to_string())?;
    }
    encoder.finish().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELIX: &str = r##"<svg width="800px" height="400px" viewBox="0 0 1024 512" xmlns="http://www.w3.org/2000/svg"><rect x="8" y="8" width="1000" height="490" fill="#c0ff33"/></svg>"##;

    #[test]
    fn an_svg_is_drawn_at_the_long_side_and_probes_as_a_still() {
        let dir = std::env::temp_dir().join(format!("concat-svg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("helix.svg");
        std::fs::write(&svg, HELIX).unwrap();

        let drawn = readable(&svg).unwrap();
        assert_ne!(drawn, svg);
        assert_eq!(readable(&svg).unwrap(), drawn, "drawn once");

        let info = crate::probe(&svg).unwrap();
        assert_eq!(info.path, svg, "the project keeps the SVG");
        let video = info.video.expect("a picture");
        assert_eq!((video.width, video.height), (3840, 1920));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Everything that reads pictures asks for a drawing not made yet at
    /// once - the monitor, the prefetcher, an export's decoders - and every
    /// one of them gets it, whole.
    #[test]
    fn an_svg_asked_for_on_many_threads_at_once_is_drawn_for_all_of_them() {
        let dir = std::env::temp_dir().join(format!("concat-svg-race-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("race.svg");
        std::fs::write(&svg, HELIX).unwrap();

        let drawings: Vec<PathBuf> = std::thread::scope(|scope| {
            let asks: Vec<_> = (0..8).map(|_| scope.spawn(|| readable(&svg))).collect();
            asks.into_iter()
                .map(|ask| ask.join().expect("no panic").expect("drawn"))
                .collect()
        });
        assert!(drawings.iter().all(|drawn| *drawn == drawings[0]));
        let png = std::fs::read(&drawings[0]).unwrap();
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&png).expect("a whole PNG");
        assert_eq!((pixmap.width(), pixmap.height()), (3840, 1920));
        let leftovers = std::fs::read_dir(drawings[0].parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let stem = drawings[0]
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                name.starts_with(&stem)
            })
            .count();
        assert_eq!(leftovers, 0, "no half-written drawing is left behind");
        let _ = std::fs::remove_file(&drawings[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An SVG with holes is a code clip: drawn a frame at a time into a
    /// clip as long as it says, at the frame's size, with Kanit to hand.
    #[test]
    fn a_code_clip_is_drawn_into_a_clip_that_moves() {
        let dir = std::env::temp_dir().join(format!("concat-svg-code-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("count.svg");
        std::fs::write(
            &svg,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1080 1920" data-duration="0.5" data-fps="10"><rect width="{{ p * 1080 }}" height="200" fill="#e4572e"/><text x="540" y="960" font-family="Kanit" font-weight="600" font-size="200" text-anchor="middle" fill="#fff">{{ num(ease(p) * 12500) }}</text></svg>"##,
        )
        .unwrap();

        let baked = readable(&svg).unwrap();
        assert_eq!(
            baked.extension().and_then(|e| e.to_str()),
            Some(motion_codec().container())
        );
        assert_eq!(readable(&svg).unwrap(), baked, "baked once");
        let info = crate::probe(&svg).unwrap();
        let video = info.video.expect("a picture");
        assert_eq!((video.width, video.height), (1080, 1920));
        let seconds = info.duration.expect("a length").as_f64();
        assert!((seconds - 0.5).abs() < 0.11, "{seconds}");

        std::fs::write(
            &svg,
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{{ wobble(t) }}</svg>",
        )
        .unwrap();
        let error = readable(&svg).expect_err("a bad hole").to_string();
        assert!(error.contains("no function 'wobble'"), "{error}");
        let _ = std::fs::remove_file(&baked);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_code_clip_says_how_long_it_runs_within_bounds() {
        let svg = |attributes: &str| format!("<svg {attributes}>{{{{ t }}}}</svg>");
        assert_eq!(timing(&svg("")), (4.0, 30.0));
        assert_eq!(
            timing(&svg(r#"data-duration="2s" data-fps="24""#)),
            (2.0, 24.0)
        );
        assert_eq!(
            timing(&svg(r#"data-duration="abc" data-fps="x""#)),
            (4.0, 30.0)
        );
        assert_eq!(
            timing(&svg(r#"data-duration="0" data-fps="0""#)),
            (0.1, 1.0)
        );
        assert_eq!(
            timing(&svg(r#"data-duration="-3" data-fps="-5""#)),
            (0.1, 1.0)
        );
        assert_eq!(
            timing(&svg(r#"data-duration="500" data-fps="1000""#)),
            (30.0, 60.0)
        );
        assert_eq!(
            timing(&svg(r#"data-duration="inf" data-fps="NaN""#)),
            (4.0, 30.0)
        );
        assert_eq!(timing(&svg(r#"data-duration=" 1.5 ""#)), (1.5, 30.0));
    }

    /// Sizes an encoder refuses - odd, a sliver, none at all - are drawn
    /// at even sizes, the long side at the frame's.
    #[test]
    fn a_code_clip_of_any_shape_is_drawn_at_an_even_size() {
        let dir = std::env::temp_dir().join(format!("concat-svg-sizes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, root, expect) in [
            ("odd", r#"viewBox="0 0 101 99""#, (1920, 1882)),
            ("sliver", r#"viewBox="0 0 10000 1""#, (1920, 2)),
            ("unsized", "", (1920, 1920)),
        ] {
            let svg = dir.join(format!("{name}.svg"));
            std::fs::write(
                &svg,
                format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" {root} data-duration="0.2" data-fps="10"><rect width="{{{{ p * 50 }}}}" height="1" fill="#fff"/></svg>"##
                ),
            )
            .unwrap();
            let info = crate::probe(&svg).unwrap_or_else(|e| panic!("{name}: {e}"));
            let video = info.video.expect("a picture");
            assert_eq!((video.width, video.height), expect, "{name}");
            let _ = std::fs::remove_file(readable(&svg).unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A map placed as another SVG, by a path from the clip's folder, is
    /// drawn: the brief has code clips stand on a shared map that way.
    #[test]
    fn an_svg_placed_inside_another_is_drawn() {
        let dir = std::env::temp_dir().join(format!("concat-svg-nested-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("library/maps")).unwrap();
        std::fs::create_dir_all(dir.join("graphics")).unwrap();
        std::fs::write(
            dir.join("library/maps/land.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#00ff00"/></svg>"##,
        )
        .unwrap();
        let svg = dir.join("graphics/map.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><image width="100" height="100" href="../library/maps/land.svg"/></svg>"#,
        )
        .unwrap();
        let png = std::fs::read(readable(&svg).unwrap()).unwrap();
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&png).unwrap();
        let middle = pixmap
            .pixel(pixmap.width() / 2, pixmap.height() / 2)
            .unwrap();
        assert_eq!(
            (middle.green(), middle.alpha()),
            (255, 255),
            "the map is drawn"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A picture the SVG places by a path relative to itself is drawn:
    /// it is looked for beside the SVG.
    #[test]
    fn a_picture_named_beside_the_svg_is_drawn() {
        let dir = std::env::temp_dir().join(format!("concat-svg-image-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut dot = resvg::tiny_skia::Pixmap::new(4, 4).unwrap();
        dot.fill(resvg::tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        std::fs::write(dir.join("dot.png"), dot.encode_png().unwrap()).unwrap();
        let svg = dir.join("placed.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100"><image x="0" y="0" width="100" height="100" xlink:href="dot.png"/></svg>"#,
        )
        .unwrap();

        let drawn = readable(&svg).unwrap();
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&std::fs::read(&drawn).unwrap())
            .expect("a whole PNG");
        let middle = pixmap.pixel(1920, 1920).expect("inside");
        assert_eq!(
            (middle.red(), middle.green(), middle.blue(), middle.alpha()),
            (255, 0, 0, 255),
            "the placed picture is in the drawing"
        );
        let _ = std::fs::remove_file(&drawn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_an_svg_says_so() {
        let dir = std::env::temp_dir().join(format!("concat-svg-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("broken.svg");
        std::fs::write(&svg, "not xml at all").unwrap();
        let error = readable(&svg).unwrap_err().to_string();
        assert!(error.contains("not an SVG"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn other_files_are_read_as_they_are() {
        let path = Path::new("clip.mp4");
        assert_eq!(readable(path).unwrap(), path);
        assert!(is_svg(Path::new("logo.SVG")));
    }
}
