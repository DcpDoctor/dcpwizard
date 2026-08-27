//! DCP preview and playback support.
//!
//! Delegates to [`postkit::preview`] for frame-accurate media preview,
//! frame extraction, and image-sequence rendering.

pub use postkit::preview::{
    FrameInfo, PlaybackOptions, extract_frame, get_frame_info, play, render_to_sequence,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_default_playback_options() {
        let opts = PlaybackOptions::default();
        assert_eq!(opts.start_frame, 0);
        assert_eq!(opts.end_frame, 0);
        assert!(!opts.loop_playback);
        assert!(opts.decode_to_display);
        assert_eq!(opts.display_colourspace, "sRGB");
        assert_eq!(opts.gpu_device, -1);
    }

    #[test]
    fn test_frame_info_default() {
        let info = FrameInfo::default();
        assert_eq!(info.frame_number, 0);
        assert_eq!(info.width, 0);
        assert_eq!(info.height, 0);
    }

    #[test]
    fn test_extract_frame_missing_input() {
        let rc = extract_frame(
            &PathBuf::from("/nonexistent.mxf"),
            0,
            &PathBuf::from("/tmp/out.png"),
            None,
        );
        assert_ne!(rc, 0);
    }
}
