//! Visible burn-in watermark.
//!
//! Burns a plainly visible text mark (the payload) into the video/image using
//! postkit's ffmpeg drawtext burn-in. This is a visible mark, not an invisible
//! or forensic watermark, and carries no recoverable payload.
//!
//! postkit::watermark is a different mark: a faint operator/session hash drawn
//! over an image sequence, which is what imfwizard's watermark command burns.

use std::path::PathBuf;

/// How the mark is drawn, and what the marked copy is encoded as.
#[derive(Debug, Clone)]
pub struct WatermarkStyle {
    pub font_size: u32,
    /// Any ffmpeg colour name or hex, which is what the drawtext branch takes.
    pub colour: String,
    /// "top", "center" or "bottom".
    pub position: String,
    /// Video encoder for the marked copy. Empty leaves it to ffmpeg's guess from
    /// the output file name.
    pub video_codec: String,
    pub video_crf: Option<u32>,
}

impl Default for WatermarkStyle {
    fn default() -> Self {
        Self {
            font_size: 24,
            colour: "white".to_string(),
            position: "bottom".to_string(),
            video_codec: String::new(),
            video_crf: None,
        }
    }
}

/// Burn `payload` as a visible text overlay into `input`, writing `output`.
/// Requires ffmpeg; returns Err with the ffmpeg error if it is missing or fails.
pub fn embed_watermark(
    input: PathBuf,
    output: PathBuf,
    payload: &str,
    style: &WatermarkStyle,
) -> std::io::Result<()> {
    let opts = postkit::burnin::BurninOptions {
        input,
        output,
        text: Some(payload.to_string()),
        font_size: style.font_size,
        font_colour: style.colour.clone(),
        position: style.position.clone(),
        video_codec: style.video_codec.clone(),
        video_crf: style.video_crf,
        ..Default::default()
    };
    postkit::burnin::burnin(&opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_watermark_missing_input_fails() {
        let dir = tempfile::tempdir().unwrap();
        let result = embed_watermark(
            dir.path().join("nope.mov"),
            dir.path().join("out.mov"),
            "DIST-001",
            &WatermarkStyle::default(),
        );
        assert!(result.is_err());
    }
}
