//! A single image held for a duration, as a directory of J2K frames.
//!
//! The image is encoded once and the codestream linked for every frame of the
//! hold: a frame-wrapped picture MXF may repeat one codestream, so a two-minute
//! title card costs one encode instead of two thousand. Same trick as the pad
//! frames in [`crate::pad`].

use std::path::Path;

/// Image extensions `--video` accepts as a still. A file with one of these is a
/// still input and needs a hold duration; anything else is a video or a
/// codestream directory.
pub const STILL_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "tif", "tiff", "bmp", "dpx", "exr"];

/// Is `path` a single image file rather than a video or a J2K directory?
pub fn is_still(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| STILL_EXTENSIONS.contains(&e.to_lowercase().as_str()))
            .unwrap_or(false)
}

/// Decode `image` to one rgb48be frame at `width`x`height`, sized by the caller
/// from its own probe so a mismatch is refused before this runs.
fn decode_rgb48(image: &Path, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(image)
        .args(["-frames:v", "1", "-pix_fmt", "rgb48be", "-f", "rawvideo"])
        .arg("pipe:1")
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run ffmpeg to read {}: {e}", image.display()))?;
    if !output.status.success() {
        return Err(format!("ffmpeg could not decode {}", image.display()));
    }
    let want = (width as usize) * (height as usize) * 6;
    if output.stdout.len() != want {
        return Err(format!(
            "{} decoded to {} bytes, not the {want} a {width}x{height} frame needs",
            image.display(),
            output.stdout.len()
        ));
    }
    Ok(output.stdout)
}

/// Encode `image` once and link the codestream for each of `frames` frames of
/// `out_dir`. `apply_xyz_transform` is the encoder's Rec.709 to DCI X'Y'Z' pass,
/// off when the source is already X'Y'Z'.
pub fn build_still_frames(
    image: &Path,
    frames: u64,
    fps: u32,
    width: u32,
    height: u32,
    apply_xyz_transform: bool,
    out_dir: &Path,
) -> Result<(), String> {
    use postkit::grok_encoder::{self, CompressParams, RawFrame};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    if frames == 0 {
        return Err("a still needs a hold of at least one frame".into());
    }
    let mut data = decode_rgb48(image, width, height)?;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
    crate::reel::clear_stale_frames(out_dir)?;

    let params = CompressParams {
        frame_rate: fps.max(1) as u16,
        apply_xyz_transform,
        ..CompressParams::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    grok_encoder::initialize(0);
    let mut produced = false;
    let result = grok_encoder::encode_pipeline(
        out_dir,
        &params,
        1,
        &cancel,
        || {
            if produced {
                return None;
            }
            produced = true;
            Some(RawFrame::Packed {
                data: std::mem::take(&mut data),
                width,
                height,
                precision: 16,
                index: 0,
            })
        },
        |_p| {},
    );
    if !result.success {
        return Err(format!("still frame encode failed: {}", result.error));
    }

    let first = out_dir.join("frame_00000000.j2c");
    for index in 1..frames {
        let target = out_dir.join(format!("frame_{index:08}.j2c"));
        let _ = std::fs::remove_file(&target);
        if std::fs::hard_link(&first, &target).is_err() {
            std::fs::copy(&first, &target)
                .map_err(|e| format!("cannot place {}: {e}", target.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_files_are_stills_and_videos_are_not() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["card.png", "card.TIF", "card.dpx"] {
            let path = dir.path().join(name);
            std::fs::write(&path, "x").unwrap();
            assert!(is_still(&path), "{name} must read as a still");
        }
        for name in ["movie.mov", "movie.mp4", "frame.j2c"] {
            let path = dir.path().join(name);
            std::fs::write(&path, "x").unwrap();
            assert!(!is_still(&path), "{name} must not read as a still");
        }
        assert!(!is_still(dir.path()), "a directory is not a still");
    }

    #[test]
    fn a_zero_length_hold_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("card.png");
        std::fs::write(&image, "x").unwrap();
        let err = build_still_frames(&image, 0, 24, 2048, 1080, true, dir.path()).unwrap_err();
        assert!(err.contains("at least one frame"), "got: {err}");
    }
}
