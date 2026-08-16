//! Gain, fades and the picture/sound delay applied to the programme audio before
//! it is wrapped, and the matching fade filter for the picture.
//!
//! Every adjustment here keeps the running time the same, so the sound still
//! matches the picture duration the CPL declares. Anything that would change
//! the length belongs with the head/tail padding instead, which moves picture
//! and sound together.

use std::path::{Path, PathBuf};

/// What to do to the audio, in the order ffmpeg applies it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioAdjust {
    /// Gain in dB, positive or negative.
    pub gain_db: Option<f64>,
    /// Fade up from silence over this many seconds from the start.
    pub fade_in_seconds: Option<f64>,
    /// Fade down to silence over this many seconds at the end.
    pub fade_out_seconds: Option<f64>,
}

impl AudioAdjust {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Build the ffmpeg filter chain. `duration_seconds` positions the fade-out,
/// which ffmpeg needs as a start time rather than an offset from the end.
fn filter_chain(adjust: &AudioAdjust, duration_seconds: f64) -> Result<String, String> {
    let mut filters = Vec::new();
    if let Some(gain) = adjust.gain_db {
        if !gain.is_finite() {
            return Err(format!("audio gain {gain} is not a number"));
        }
        filters.push(format!("volume={gain}dB"));
    }
    if let Some(seconds) = adjust.fade_in_seconds {
        check_fade("fade-in", seconds, duration_seconds)?;
        filters.push(format!("afade=t=in:st=0:d={seconds}"));
    }
    if let Some(seconds) = adjust.fade_out_seconds {
        check_fade("fade-out", seconds, duration_seconds)?;
        // ffmpeg wants the moment the fade starts, not its length from the end
        let start = duration_seconds - seconds;
        filters.push(format!("afade=t=out:st={start}:d={seconds}"));
    }
    Ok(filters.join(","))
}

fn check_fade(name: &str, seconds: f64, duration_seconds: f64) -> Result<(), String> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(format!(
            "audio {name} of {seconds}s must be greater than zero"
        ));
    }
    if seconds > duration_seconds {
        return Err(format!(
            "audio {name} of {seconds}s is longer than the {duration_seconds}s of audio"
        ));
    }
    Ok(())
}

/// Running time of a WAV, read from its header.
pub fn duration_seconds(path: &Path) -> Result<f64, String> {
    let reader =
        hound::WavReader::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let rate = reader.spec().sample_rate;
    if rate == 0 {
        return Err(format!("{} declares a sample rate of zero", path.display()));
    }
    Ok(reader.duration() as f64 / rate as f64)
}

/// Apply `adjust` to `input`, writing `output`. Returns the path actually to
/// use, which is `input` unchanged when there is nothing to do.
pub fn apply(
    input: &Path,
    output: &Path,
    adjust: &AudioAdjust,
    duration_seconds: f64,
) -> Result<PathBuf, String> {
    if adjust.is_empty() {
        return Ok(input.to_path_buf());
    }
    let chain = filter_chain(adjust, duration_seconds)?;
    if chain.is_empty() {
        return Ok(input.to_path_buf());
    }

    let status = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-af")
        .arg(&chain)
        // the wrap wants the same PCM it would have had, only quieter
        .args(["-c:a", "pcm_s24le"])
        .arg(output)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("could not run ffmpeg to adjust the audio: {e}"))?;
    if !status.success() {
        return Err(format!(
            "ffmpeg failed to apply the audio filters '{chain}'"
        ));
    }
    Ok(output.to_path_buf())
}

/// Shift the sound against the picture by `delay_ms`, keeping the running time.
/// A positive delay prepends that much silence and drops the same from the tail
/// (the sound arrives later); a negative delay drops from the head and appends
/// silence. A delay at least as long as the programme is refused, since it would
/// leave nothing but silence.
pub fn apply_delay(input: &Path, output: &Path, delay_ms: i64) -> Result<PathBuf, String> {
    if delay_ms == 0 {
        return Ok(input.to_path_buf());
    }
    let info = crate::reel::parse_wav(input)?;
    if info.sample_rate == 0 || info.block_align == 0 {
        return Err(format!("{} has no usable WAV format", input.display()));
    }
    let total_samples = info.data_size / info.block_align as u64;
    let shift = delay_ms.unsigned_abs() * info.sample_rate as u64 / 1000;
    if shift >= total_samples {
        let programme_ms = total_samples * 1000 / info.sample_rate as u64;
        return Err(format!(
            "audio delay of {delay_ms}ms is at least as long as the {programme_ms}ms programme, \
             which would leave silence"
        ));
    }
    let (head_silence, start_sample) = if delay_ms > 0 { (shift, 0) } else { (0, shift) };
    crate::reel::write_shifted_wav(
        input,
        &info,
        head_silence,
        start_sample,
        total_samples,
        output,
    )?;
    Ok(output.to_path_buf())
}

/// ffmpeg `-vf` fade chain for the picture, or `None` when neither fade is
/// asked for. Fades darken frames in place, so the frame count the CPL declares
/// is unchanged.
pub fn video_fade_filter(
    fade_in_seconds: Option<f64>,
    fade_out_seconds: Option<f64>,
    duration_seconds: f64,
) -> Result<Option<String>, String> {
    let mut filters = Vec::new();
    if let Some(seconds) = fade_in_seconds {
        check_fade("video fade-in", seconds, duration_seconds)?;
        filters.push(format!("fade=t=in:st=0:d={seconds}"));
    }
    if let Some(seconds) = fade_out_seconds {
        check_fade("video fade-out", seconds, duration_seconds)?;
        let start = duration_seconds - seconds;
        filters.push(format!("fade=t=out:st={start}:d={seconds}"));
    }
    if filters.is_empty() {
        return Ok(None);
    }
    Ok(Some(filters.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DURATION: f64 = 10.0;

    #[test]
    fn nothing_to_do_leaves_the_input_alone() {
        let adjust = AudioAdjust::default();
        assert!(adjust.is_empty());
        let input = Path::new("in.wav");
        assert_eq!(
            apply(input, Path::new("out.wav"), &adjust, DURATION).unwrap(),
            input.to_path_buf(),
            "an empty adjustment must not spawn ffmpeg or rewrite the audio"
        );
    }

    #[test]
    fn gain_and_both_fades_build_one_chain_in_order() {
        let chain = filter_chain(
            &AudioAdjust {
                gain_db: Some(-3.5),
                fade_in_seconds: Some(2.0),
                fade_out_seconds: Some(4.0),
            },
            DURATION,
        )
        .unwrap();
        assert_eq!(
            chain,
            "volume=-3.5dB,afade=t=in:st=0:d=2,afade=t=out:st=6:d=4"
        );
    }

    // the fade-out is positioned from the end, so its start moves with the length
    #[test]
    fn the_fade_out_starts_its_own_length_before_the_end() {
        for (duration, fade, start) in [(10.0, 4.0, 6.0), (100.0, 4.0, 96.0), (5.0, 5.0, 0.0)] {
            let chain = filter_chain(
                &AudioAdjust {
                    fade_out_seconds: Some(fade),
                    ..Default::default()
                },
                duration,
            )
            .unwrap();
            assert_eq!(chain, format!("afade=t=out:st={start}:d={fade}"));
        }
    }

    #[test]
    fn a_fade_longer_than_the_audio_is_refused() {
        for adjust in [
            AudioAdjust {
                fade_in_seconds: Some(DURATION + 1.0),
                ..Default::default()
            },
            AudioAdjust {
                fade_out_seconds: Some(DURATION + 1.0),
                ..Default::default()
            },
        ] {
            let err = filter_chain(&adjust, DURATION).unwrap_err();
            assert!(err.contains("longer than"), "got: {err}");
        }
    }

    #[test]
    fn a_zero_or_negative_fade_is_refused() {
        for seconds in [0.0, -1.0] {
            let err = filter_chain(
                &AudioAdjust {
                    fade_in_seconds: Some(seconds),
                    ..Default::default()
                },
                DURATION,
            )
            .unwrap_err();
            assert!(err.contains("greater than zero"), "got: {err}");
        }
    }

    #[test]
    fn video_fades_build_a_chain_and_none_when_unasked() {
        assert_eq!(video_fade_filter(None, None, DURATION).unwrap(), None);
        assert_eq!(
            video_fade_filter(Some(1.0), Some(2.0), DURATION).unwrap(),
            Some("fade=t=in:st=0:d=1,fade=t=out:st=8:d=2".to_string())
        );
        let err = video_fade_filter(Some(DURATION + 1.0), None, DURATION).unwrap_err();
        assert!(err.contains("longer than"), "got: {err}");
    }

    /// A 48 kHz mono ramp of `samples`, and the sample count of a WAV.
    fn ramp(dir: &std::path::Path, name: &str, samples: u64) -> PathBuf {
        let path = dir.join(name);
        crate::test_wav::write_ramp_wav(&path, 1, samples);
        path
    }

    fn sample_count(path: &Path) -> u64 {
        let info = crate::reel::parse_wav(path).unwrap();
        info.data_size / info.block_align as u64
    }

    fn sample_at(path: &Path, index: u64) -> u16 {
        let info = crate::reel::parse_wav(path).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let at = (info.data_offset + index * info.block_align as u64) as usize;
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    // the running-time invariant this whole module is built on: a delay may not
    // change how many samples ship, only which ones
    #[test]
    fn a_delay_keeps_the_sample_count() {
        let dir = tempfile::tempdir().unwrap();
        let src = ramp(dir.path(), "in.wav", 48_000);
        for delay_ms in [250i64, -250, 1, -1] {
            let out = dir.path().join(format!("out{delay_ms}.wav"));
            apply_delay(&src, &out, delay_ms).unwrap();
            assert_eq!(
                sample_count(&out),
                48_000,
                "a {delay_ms}ms delay must not change the running time"
            );
        }
    }

    #[test]
    fn a_positive_delay_leads_with_silence_and_a_negative_one_starts_late() {
        let dir = tempfile::tempdir().unwrap();
        let src = ramp(dir.path(), "in.wav", 48_000);

        // +250ms is 12000 samples of silence, then the ramp from its own zero
        let later = dir.path().join("later.wav");
        apply_delay(&src, &later, 250).unwrap();
        assert_eq!(sample_at(&later, 11_999), 0, "lead-in is silent");
        assert_eq!(sample_at(&later, 12_000), 0, "the ramp restarts at zero");
        assert_eq!(
            sample_at(&later, 12_100),
            100,
            "the ramp follows the silence"
        );

        // -250ms drops the first 12000 samples and appends the same in silence
        let earlier = dir.path().join("earlier.wav");
        apply_delay(&src, &earlier, -250).unwrap();
        assert_eq!(sample_at(&earlier, 0), 12_000);
        assert_eq!(
            sample_at(&earlier, 47_999),
            0,
            "the tail is made up in silence"
        );
    }

    #[test]
    fn a_delay_longer_than_the_programme_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let src = ramp(dir.path(), "in.wav", 48_000);
        let err = apply_delay(&src, &dir.path().join("out.wav"), 2_000).unwrap_err();
        assert!(err.contains("leave silence"), "got: {err}");
        assert!(apply_delay(&src, &dir.path().join("out.wav"), -2_000).is_err());
    }

    #[test]
    fn a_zero_delay_leaves_the_input_alone() {
        let input = Path::new("in.wav");
        assert_eq!(
            apply_delay(input, Path::new("out.wav"), 0).unwrap(),
            input.to_path_buf(),
            "nothing to do must not rewrite the audio"
        );
    }

    // a gain of zero is a real request (it is what --audio-gain 0 means), so it
    // must build a chain rather than being treated as absent
    #[test]
    fn a_zero_gain_still_builds_a_filter() {
        let chain = filter_chain(
            &AudioAdjust {
                gain_db: Some(0.0),
                ..Default::default()
            },
            DURATION,
        )
        .unwrap();
        assert_eq!(chain, "volume=0dB");
    }
}
