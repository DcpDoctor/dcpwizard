//! The sound a build uses when the job named no audio file: the picture
//! source's own track.
//!
//! It comes out as the 48 kHz 24-bit PCM a DCP sound track carries, so it can be
//! processed and wrapped like a supplied WAV.

use std::path::{Path, PathBuf};

const SAMPLE_RATE: u32 = 48_000;

/// Pull the video's own audio out to a 48 kHz 24-bit WAV in `work_dir`, every
/// channel as the source carries it. None when the source has no audio stream.
pub fn extract_embedded_audio(video: &Path, work_dir: &Path) -> Result<Option<PathBuf>, String> {
    if !crate::probe::probe_video(video).is_some_and(|info| info.has_audio) {
        return Ok(None);
    }
    std::fs::create_dir_all(work_dir)
        .map_err(|e| format!("cannot create {}: {e}", work_dir.display()))?;
    let output = work_dir.join("embedded.wav");
    let result = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(video)
        .arg("-vn")
        .args(["-c:a", "pcm_s24le", "-ar", &SAMPLE_RATE.to_string()])
        .arg(&output)
        .output()
        .map_err(|e| format!("failed to run ffmpeg to extract the source's audio: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "could not extract the audio from {}: {}",
            video.display(),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(Some(output))
}
