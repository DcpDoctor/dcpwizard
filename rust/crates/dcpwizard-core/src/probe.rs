pub use postkit::probe::*;

use std::path::Path;

/// Whether the local ffmpeg build lists a given encoder (e.g. "libvpx-vp9").
pub fn ffmpeg_has_encoder(name: &str) -> bool {
    ffmpeg_lists("-encoders", name)
}

/// Whether the local ffmpeg build lists a given decoder (e.g. "apv").
pub fn ffmpeg_has_decoder(name: &str) -> bool {
    ffmpeg_lists("-decoders", name)
}

/// The source's primary video codec name via ffprobe (e.g. "apv", "h264").
pub fn source_video_codec(path: &Path) -> Result<String, String> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffprobe could not inspect {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let codec = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if codec.is_empty() {
        return Err(format!("no video stream found in {}", path.display()));
    }
    Ok(codec)
}

/// Fail loud unless the local ffmpeg can decode the source's video codec. Admits
/// any format ffmpeg decodes (e.g. APV once ffmpeg gained the decoder).
pub fn ensure_video_decodable(path: &Path) -> Result<(), String> {
    let codec = source_video_codec(path)?;
    if !ffmpeg_has_decoder(&codec) {
        return Err(format!(
            "ffmpeg has no decoder for video codec '{codec}'; cannot decode {}",
            path.display()
        ));
    }
    Ok(())
}

/// Match `name` in the second column of `ffmpeg -encoders`/`-decoders` output.
fn ffmpeg_lists(flag: &str, name: &str) -> bool {
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", flag])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().any(|line| {
            let mut cols = line.split_whitespace();
            let _flags = cols.next();
            cols.next() == Some(name)
        }),
        _ => false,
    }
}

pub fn pixel_format_has_alpha(pixel_format: &str) -> bool {
    pixel_format.starts_with("yuva")
        || pixel_format.starts_with("gbrap")
        || matches!(
            pixel_format,
            "rgba" | "bgra" | "argb" | "abgr" | "ya8" | "ya16le" | "ya16be" | "pal8"
        )
}

#[cfg(test)]
mod tests {
    use super::pixel_format_has_alpha;

    #[test]
    fn detects_alpha_pixel_formats() {
        assert!(pixel_format_has_alpha("yuva444p12le"));
        assert!(pixel_format_has_alpha("gbrap12le"));
        assert!(pixel_format_has_alpha("rgba"));
        assert!(pixel_format_has_alpha("pal8"));
        assert!(!pixel_format_has_alpha("yuv444p12le"));
        assert!(!pixel_format_has_alpha("gbrp12le"));
    }
}
