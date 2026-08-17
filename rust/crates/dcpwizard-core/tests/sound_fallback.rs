//! Sound when the job named no audio file: the source video's own track is
//! extracted for it.

use std::path::Path;

const FPS: u32 = 24;
const CLIP_FRAMES: u32 = 24;
const SAMPLE_RATE: u32 = 48_000;

fn make_clip(path: &Path, with_sound: bool) -> bool {
    let mut command = std::process::Command::new("ffmpeg");
    command.args([
        "-y",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc=size=320x240:rate={FPS}"),
    ]);
    if with_sound {
        command.args([
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:sample_rate={SAMPLE_RATE}"),
            "-ac",
            "2",
        ]);
    }
    command
        .args(["-frames:v", &CLIP_FRAMES.to_string(), "-shortest"])
        .arg(path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
        && path.exists()
}

#[test]
fn the_sources_own_audio_is_extracted_when_no_wav_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("with_sound.mp4");
    if !make_clip(&clip, true) {
        eprintln!("skipping: ffmpeg could not build the source clip");
        return;
    }

    let extracted =
        dcpwizard_core::audio_fallback::extract_embedded_audio(&clip, &dir.path().join("work"))
            .expect("extract the embedded audio")
            .expect("the clip carries audio");

    let reader = hound::WavReader::open(&extracted).expect("a readable WAV");
    assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
    assert_eq!(reader.spec().bits_per_sample, 24);
    assert_eq!(reader.spec().channels, 2, "no downmix, no widening");
    let expected = (SAMPLE_RATE / FPS) as u64 * CLIP_FRAMES as u64;
    assert!(
        reader.duration() as u64 >= expected / 2,
        "{} samples for a {CLIP_FRAMES} frame clip",
        reader.duration()
    );
    assert!(
        reader.into_samples::<i32>().any(|s| s.unwrap() != 0),
        "the sine must survive the extraction"
    );
}

#[test]
fn a_source_without_audio_extracts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("mute.mp4");
    if !make_clip(&clip, false) {
        eprintln!("skipping: ffmpeg could not build the source clip");
        return;
    }

    let work = dir.path().join("work");
    assert!(
        dcpwizard_core::audio_fallback::extract_embedded_audio(&clip, &work)
            .expect("probe the mute clip")
            .is_none()
    );
    assert!(!work.exists(), "nothing to extract, nothing written");
}
