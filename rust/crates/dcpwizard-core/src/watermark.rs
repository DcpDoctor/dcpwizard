//! Visible burn-in watermark.
//!
//! Builds the burn that composites a plainly visible line of text into every
//! picture frame, held for the whole programme. `create` hands it to the
//! encoder alongside a subtitle burn; the `watermark` command hands it to the
//! transcode path, which decodes an existing DCP's picture, marks it and
//! re-encodes it. This is a visible mark, not an invisible or forensic
//! watermark, and carries no recoverable payload.
//!
//! postkit::watermark is a different mark: a faint operator/session hash drawn
//! over an image sequence, which is what imfwizard's watermark command burns.

use std::path::Path;
use std::sync::Arc;

use postkit::subtitle_formats::{Rgba, StyledCue, StyledRun, VAlign};
use postkit::subtitle_raster::{BurnStyle, BurnStyleOverrides, SubtitleBurn};

/// Turns postkit's font size ratio into the percent the flags take.
const RATIO_TO_PERCENT: f32 = 100.0;

/// Text height as a percent of the frame height, the same as a burnt-in
/// subtitle's default so an unnamed size draws at the house size.
pub const DEFAULT_FONT_SIZE_PERCENT: f32 =
    postkit::subtitle_raster::DEFAULT_FONT_SIZE_RATIO * RATIO_TO_PERCENT;

/// How the mark is drawn.
#[derive(Debug, Clone)]
pub struct WatermarkOptions {
    /// The payload rendered visibly: a distributor id, a serial, a name.
    pub text: String,
    /// Text height as a percent of the frame height.
    pub font_size_percent: f32,
    pub colour: Rgba,
    /// Which edge the mark is anchored to, the margin coming from the burn
    /// style's own default.
    pub position: VAlign,
}

impl Default for WatermarkOptions {
    fn default() -> Self {
        WatermarkOptions {
            text: String::new(),
            font_size_percent: DEFAULT_FONT_SIZE_PERCENT,
            colour: Rgba {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            position: VAlign::Bottom,
        }
    }
}

/// Read a placement flag (top, center or bottom), refused under the flag's own
/// name.
pub fn parse_position_flag(flag: &str, text: &str) -> Result<VAlign, String> {
    match text.to_ascii_lowercase().as_str() {
        "top" => Ok(VAlign::Top),
        "center" => Ok(VAlign::Middle),
        "bottom" => Ok(VAlign::Bottom),
        _ => Err(format!(
            "{flag}: {text} is not a placement: pick top, center or bottom"
        )),
    }
}

/// Build the burn that draws `options` on every frame, shaping the text with
/// `font` or with the system faces when there is none.
///
/// One cue covering the whole programme, so no frame of any length of picture
/// goes unmarked.
pub fn watermark_burn(
    options: &WatermarkOptions,
    font: Option<&Path>,
    fps: f64,
) -> Result<Arc<SubtitleBurn>, String> {
    if options.text.trim().is_empty() {
        return Err("--watermark needs the text to draw".to_string());
    }
    if let Some(path) = font
        && !path.is_file()
    {
        return Err(format!("watermark font not found: {}", path.display()));
    }
    let style = BurnStyleOverrides {
        font_size_percent: Some(options.font_size_percent),
        colour: Some(options.colour),
        ..Default::default()
    }
    .apply(BurnStyle::default())
    .map_err(|e| format!("watermark appearance: {e}"))?;
    let mut cue = StyledCue::text(
        0,
        u64::MAX,
        vec![StyledRun::plain(options.text.trim().to_string())],
    );
    cue.valign = Some(options.position);
    SubtitleBurn::new(vec![cue], font, style, fps)
        .map(Arc::new)
        .map_err(|e| format!("cannot draw the watermark: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME_WIDTH: u32 = 640;
    const FRAME_HEIGHT: u32 = 360;
    /// Text height as a percent of the frame height, big enough that a mark
    /// anchored to one edge cannot reach the thirds around it.
    const FONT_SIZE_PERCENT: f32 = 10.0;
    const THIRDS: u32 = 3;

    /// Rows of a black frame the burn drew on, at `frame_index`.
    fn marked_rows(burn: &SubtitleBurn, frame_index: u64) -> Vec<u32> {
        let width = FRAME_WIDTH as usize;
        let mut frame = vec![0u8; width * FRAME_HEIGHT as usize * 6];
        burn.burn_rgb48(
            &mut frame,
            FRAME_WIDTH,
            FRAME_HEIGHT,
            postkit::grok_encoder::SampleOrder::Big,
            frame_index,
        )
        .expect("burn onto a black frame");
        (0..FRAME_HEIGHT)
            .filter(|row| {
                let start = *row as usize * width * 6;
                frame[start..start + width * 6]
                    .iter()
                    .any(|byte| *byte != 0)
            })
            .collect()
    }

    fn mark(position: VAlign) -> Arc<SubtitleBurn> {
        let options = WatermarkOptions {
            text: "DIST-001".into(),
            font_size_percent: FONT_SIZE_PERCENT,
            position,
            ..Default::default()
        };
        watermark_burn(&options, None, 24.0).expect("a system font to draw with")
    }

    #[test]
    fn each_position_draws_in_its_own_third_of_the_frame() {
        for (flag, position, third) in [
            ("top", VAlign::Top, 0),
            ("center", VAlign::Middle, 1),
            ("bottom", VAlign::Bottom, 2),
        ] {
            assert_eq!(
                parse_position_flag("--position", flag).expect("a known placement"),
                position
            );
            let rows = marked_rows(&mark(position), 0);
            assert!(!rows.is_empty(), "{flag} drew nothing");
            let band = (
                third * FRAME_HEIGHT / THIRDS,
                (third + 1) * FRAME_HEIGHT / THIRDS,
            );
            assert!(
                rows.iter().all(|row| (band.0..band.1).contains(row)),
                "{flag} drew on rows {}..{} of a {FRAME_HEIGHT} row frame, outside {band:?}",
                rows[0],
                rows[rows.len() - 1]
            );
        }
        assert!(parse_position_flag("--position", "middle").is_err());
    }

    /// One cue covers the whole programme, so a frame hours in is still marked.
    #[test]
    fn the_mark_is_still_drawn_late_in_the_picture() {
        let burn = mark(VAlign::Bottom);
        assert_eq!(marked_rows(&burn, 0), marked_rows(&burn, 1_000_000));
    }

    #[test]
    fn an_empty_watermark_is_refused() {
        for text in ["", "   "] {
            let options = WatermarkOptions {
                text: text.into(),
                ..Default::default()
            };
            let err = watermark_burn(&options, None, 24.0).expect_err("no text is no mark");
            assert!(err.contains("--watermark"), "got: {err}");
        }
    }

    #[test]
    fn a_missing_font_is_refused_by_path() {
        let options = WatermarkOptions {
            text: "DIST-001".into(),
            ..Default::default()
        };
        let err = watermark_burn(&options, Some(Path::new("/nope/none.ttf")), 24.0)
            .expect_err("a font that is not there cannot draw");
        assert!(err.contains("/nope/none.ttf"), "got: {err}");
    }
}
