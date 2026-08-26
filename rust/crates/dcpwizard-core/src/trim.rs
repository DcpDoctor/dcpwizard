//! Head/tail trim of the source: fewer picture frames, a shorter sound track and
//! timed text that follows both.
//!
//! Trim runs before padding, so `--trim-start 2s --pad-head 1s` drops two seconds
//! of source and then prepends one second of black. A source the encoder reads
//! is windowed by [`encode_window`], so only the kept frames are ever compressed.
//! A codestream directory is never encoded, so its kept frames are relinked by
//! [`link_trimmed_frames`] instead.

use std::path::Path;

/// The window a trimmed encode asks for, or None when the picture reaches the
/// package without being encoded here and the kept codestreams are linked out of
/// it instead.
///
/// This is the one place that decides which of the two the picture takes, so the
/// command line and the GUI cannot answer it differently.
pub fn encode_window(
    picture: &Path,
    start_frames: u64,
    kept_frames: u64,
) -> Option<postkit::encode::FrameRange> {
    if kept_frames == 0 {
        return None;
    }
    match postkit::encode::detect_input_type(picture) {
        postkit::encode::InputType::Video | postkit::encode::InputType::ImageSequence => {
            Some(postkit::encode::FrameRange {
                first_frame: start_frames,
                frame_count: kept_frames,
            })
        }
        postkit::encode::InputType::J2kSequence | postkit::encode::InputType::Unknown => None,
    }
}

/// Number of J2K codestreams in a frame directory, in the order they wrap.
pub fn frame_count(dir: &Path) -> u64 {
    crate::reel::collect_frames(dir).len() as u64
}

/// Frames left after trimming `start` off the head and `end` off the tail of a
/// `total`-frame source. A trim that leaves nothing is refused, naming both.
pub fn kept_frames(total: u64, start: u64, end: u64) -> Result<u64, String> {
    let cut = start + end;
    if cut >= total {
        return Err(format!(
            "trimming {start} frame(s) off the head and {end} off the tail leaves nothing of the \
             {total}-frame source"
        ));
    }
    Ok(total - cut)
}

/// Link the kept J2K codestreams of `src_dir` into `out_dir`, renumbered from
/// zero. Hardlinks where the filesystem allows it and copies otherwise, so the
/// trimmed picture costs no extra disk. Returns the frame count linked.
pub fn link_trimmed_frames(
    src_dir: &Path,
    start_frames: u64,
    end_frames: u64,
    out_dir: &Path,
) -> Result<u64, String> {
    let frames = crate::reel::collect_frames(src_dir);
    let kept = kept_frames(frames.len() as u64, start_frames, end_frames)?;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
    crate::reel::clear_stale_frames(out_dir)?;
    for (index, frame) in frames
        .iter()
        .skip(start_frames as usize)
        .take(kept as usize)
        .enumerate()
    {
        let target = out_dir.join(format!("frame_{index:08}.j2c"));
        let _ = std::fs::remove_file(&target);
        if std::fs::hard_link(frame, &target).is_err() {
            std::fs::copy(frame, &target)
                .map_err(|e| format!("cannot place {}: {e}", target.display()))?;
        }
    }
    Ok(kept)
}

/// Write the kept part of a WAV: `start_frames` frames dropped from the head and
/// `kept_frames` frames retained, converted to samples at `fps`. A source shorter
/// than the window is filled out with silence so the sound still spans the picture.
pub fn trim_wav(
    src: &Path,
    start_frames: u64,
    kept_frames: u64,
    fps: u32,
    out: &Path,
) -> Result<(), String> {
    let info = crate::reel::parse_wav(src)?;
    if info.sample_rate == 0 || info.block_align == 0 {
        return Err(format!("{} has no usable WAV format", src.display()));
    }
    let fps = fps.max(1) as u64;
    let rate = info.sample_rate as u64;
    let start_sample = start_frames * rate / fps;
    let sample_count = kept_frames * rate / fps;
    crate::reel::write_reel_wav(src, &info, start_sample, sample_count, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trim_that_leaves_nothing_is_refused() {
        assert_eq!(kept_frames(100, 10, 20).unwrap(), 70);
        let err = kept_frames(100, 60, 40).unwrap_err();
        assert!(err.contains("leaves nothing"), "got: {err}");
        assert!(kept_frames(100, 100, 0).is_err());
    }

    #[test]
    fn only_an_encoded_source_gets_a_window() {
        let dir = tempfile::tempdir().unwrap();
        let window = encode_window(Path::new("/in/movie.mov"), 2, 5).unwrap();
        assert_eq!(window.first_frame, 2);
        assert_eq!(window.frame_count, 5);

        let images = dir.path().join("images");
        std::fs::create_dir_all(&images).unwrap();
        std::fs::write(images.join("frame_00000000.tif"), b"x").unwrap();
        assert!(encode_window(&images, 2, 5).is_some());

        let codestreams = dir.path().join("j2k");
        std::fs::create_dir_all(&codestreams).unwrap();
        std::fs::write(codestreams.join("frame_00000000.j2c"), b"x").unwrap();
        assert!(
            encode_window(&codestreams, 2, 5).is_none(),
            "a codestream directory is never encoded, so its kept frames are linked"
        );

        assert!(encode_window(Path::new("/in/movie.mov"), 0, 0).is_none());
    }

    #[test]
    fn only_the_kept_frames_are_linked_and_renumbered() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("j2k");
        std::fs::create_dir_all(&src).unwrap();
        for i in 0..10u64 {
            std::fs::write(src.join(format!("frame_{i:08}.j2c")), [i as u8]).unwrap();
        }
        let out = dir.path().join("trimmed");
        let kept = link_trimmed_frames(&src, 2, 3, &out).unwrap();
        assert_eq!(kept, 5);

        let linked = crate::reel::collect_frames(&out);
        assert_eq!(linked.len(), 5, "one file per kept frame");
        // the first kept frame is source frame 2, renumbered to 0
        assert_eq!(std::fs::read(&linked[0]).unwrap(), vec![2u8]);
        assert_eq!(std::fs::read(&linked[4]).unwrap(), vec![6u8]);
        assert!(
            out.join("frame_00000000.j2c").is_file(),
            "renumbered from 0"
        );
    }

    #[test]
    fn a_shorter_rerun_clears_the_longer_runs_frames() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("j2k");
        std::fs::create_dir_all(&src).unwrap();
        for i in 0..10u64 {
            std::fs::write(src.join(format!("frame_{i:08}.j2c")), [i as u8]).unwrap();
        }
        let out = dir.path().join("trimmed");
        assert_eq!(link_trimmed_frames(&src, 1, 1, &out).unwrap(), 8);
        assert_eq!(link_trimmed_frames(&src, 3, 3, &out).unwrap(), 4);
        assert_eq!(
            crate::reel::collect_frames(&out).len(),
            4,
            "frames from the longer run must not survive the rerun"
        );
    }

    #[test]
    fn trimmed_audio_keeps_the_kept_window() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.wav");
        // 48 kHz mono 16-bit, 4 frames at 24 fps = 8000 samples
        crate::test_wav::write_ramp_wav(&src, 1, 8000);

        let out = dir.path().join("trimmed.wav");
        trim_wav(&src, 1, 2, 24, &out).unwrap();

        let info = crate::reel::parse_wav(&out).unwrap();
        assert_eq!(
            info.data_size / info.block_align as u64,
            4000,
            "two frames at 24 fps is 4000 samples"
        );
        let bytes = std::fs::read(&out).unwrap();
        let start = info.data_offset as usize;
        // the window starts one frame (2000 samples) into the ramp
        assert_eq!(u16::from_le_bytes([bytes[start], bytes[start + 1]]), 2000);
    }
}
