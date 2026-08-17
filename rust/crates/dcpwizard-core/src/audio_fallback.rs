//! The sound a build uses when the job named no audio file: the picture
//! source's own track, or digital silence as long as the picture.
//!
//! Both produce the 48 kHz 24-bit PCM a DCP sound track carries, so whatever
//! comes out of here can be processed and wrapped like a supplied WAV.

use std::io::Write;
use std::path::{Path, PathBuf};

const SAMPLE_RATE: u32 = 48_000;
const BITS_PER_SAMPLE: u16 = 24;
/// A RIFF header declares its sizes in 32 bits, and the header itself takes 36
/// of the bytes the RIFF size counts.
const MAX_RIFF_PAYLOAD_BYTES: u64 = u32::MAX as u64 - 36;

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

/// Write `frames` frames' worth of 48 kHz 24-bit silence across `channels`,
/// sample-accurate at the frame edge so it lines up with the picture.
pub fn write_silent_wav(output: &Path, channels: u32, frames: u64, fps: u32) -> Result<(), String> {
    crate::pad::check_frame_aligned_sample_rate(SAMPLE_RATE, fps)?;
    let block_align = (BITS_PER_SAMPLE / 8) as u64 * channels as u64;
    let payload = frames * (SAMPLE_RATE / fps) as u64 * block_align;
    if payload > MAX_RIFF_PAYLOAD_BYTES {
        return Err(format!(
            "{channels} channels of silence over {frames} frames is {payload} bytes, more than a \
             WAV can declare"
        ));
    }

    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&((36 + payload) as u32).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&(channels as u16).to_le_bytes());
    header.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    header.extend_from_slice(&((SAMPLE_RATE as u64 * block_align) as u32).to_le_bytes());
    header.extend_from_slice(&(block_align as u16).to_le_bytes());
    header.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(payload as u32).to_le_bytes());

    let mut file =
        std::fs::File::create(output).map_err(|e| format!("cannot create {output:?}: {e}"))?;
    file.write_all(&header).map_err(|e| e.to_string())?;
    let zeros = vec![0u8; 1 << 16];
    let mut remaining = payload;
    while remaining > 0 {
        let take = remaining.min(zeros.len() as u64) as usize;
        file.write_all(&zeros[..take]).map_err(|e| e.to_string())?;
        remaining -= take as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_as_long_as_the_picture() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("silence.wav");
        write_silent_wav(&wav, 6, 48, 24).expect("write the silence");

        let reader = hound::WavReader::open(&wav).unwrap();
        assert_eq!(reader.spec().channels, 6);
        assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
        assert_eq!(reader.spec().bits_per_sample, BITS_PER_SAMPLE);
        assert_eq!(reader.duration(), 48 * 2000, "two seconds at 24 fps");
        assert!(
            reader.into_samples::<i32>().all(|s| s.unwrap() == 0),
            "every sample must be silent"
        );
    }

    #[test]
    fn a_rate_that_does_not_divide_by_the_frame_rate_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_silent_wav(&dir.path().join("silence.wav"), 6, 48, 7).is_err());
    }
}
