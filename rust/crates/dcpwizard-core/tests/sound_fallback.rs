//! Sound when the job named no audio file: the source video's own track is
//! extracted for it, and a SMPTE composition with no audio anywhere still
//! packages a silent track so it carries the ST 429-16 metadata asset.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 8;
const CLIP_FRAMES: u32 = 24;
const SAMPLE_RATE: u32 = 48_000;

fn make_clip(path: &Path, with_sound: bool) {
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
    let ok = command
        .args(["-frames:v", &CLIP_FRAMES.to_string(), "-shortest"])
        .arg(path)
        .output()
        .expect("run ffmpeg");
    assert!(
        ok.status.success() && path.exists(),
        "ffmpeg wrote no source clip\n  stdout: {}\n  stderr: {}",
        String::from_utf8_lossy(&ok.stdout).trim(),
        String::from_utf8_lossy(&ok.stderr).trim(),
    );
}

fn make_frames(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(WIDTH, HEIGHT, FPS, &seed).expect("encode frame");
    for i in 0..FRAMES {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
    dir.to_path_buf()
}

fn silent_config(root: &Path, out: &Path, standard: dcpwizard_core::Standard) -> DcpConfig {
    DcpConfig {
        title: "Silent Test".into(),
        standard,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(make_frames(&root.join("frames"))),
        ..Default::default()
    }
}

fn read_cpl(dir: &Path) -> String {
    let path = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_"))
        })
        .expect("CPL written");
    std::fs::read_to_string(path).unwrap()
}

fn sound_mxfs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("sound_") && n.ends_with(".mxf"))
        })
        .collect()
}

#[test]
fn the_sources_own_audio_is_extracted_when_no_wav_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("with_sound.mp4");
    make_clip(&clip, true);

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
    make_clip(&clip, false);

    let work = dir.path().join("work");
    assert!(
        dcpwizard_core::audio_fallback::extract_embedded_audio(&clip, &work)
            .expect("probe the mute clip")
            .is_none()
    );
    assert!(!work.exists(), "nothing to extract, nothing written");
}

#[test]
fn a_smpte_build_with_no_audio_packages_silence_and_the_metadata_asset() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    let config = silent_config(dir.path(), &out, dcpwizard_core::Standard::Smpte);
    assert_eq!(create_dcp(&config), 0, "the silent DCP must package");

    let sound = sound_mxfs(&out);
    assert_eq!(sound.len(), 1, "one silent sound MXF in {out:?}");

    let cpl = read_cpl(&out);
    assert!(
        cpl.contains("CompositionMetadataAsset"),
        "ST 429-16 metadata asset: {cpl}"
    );
    assert!(
        cpl.contains("51/L,R,C,LFE,Ls,Rs"),
        "the silence is labelled 5.1: {cpl}"
    );
    let reels = cpl.matches("<MainPicture>").count();
    assert!(reels > 0, "the CPL must carry picture: {cpl}");
    assert_eq!(
        cpl.matches("<MainSound>").count(),
        reels,
        "every reel carries the sound track: {cpl}"
    );

    // the warning the missing sound track drew out of dcpdoctor
    let report = dcpdoctor_core::verify(&out, &dcpdoctor_core::VerifyOptions::strict());
    let metadata_notes: Vec<&dcpdoctor_core::Note> = report
        .notes
        .iter()
        .filter(|note| note.message.contains("CompositionMetadataAsset"))
        .collect();
    assert!(metadata_notes.is_empty(), "{metadata_notes:?}");

    // the working silence is a build intermediate and must not ship
    assert!(
        std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().starts_with(".dcpwizard_")),
        "no intermediates left in {out:?}"
    );
}

#[test]
fn an_interop_build_with_no_audio_stays_picture_only() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    let config = silent_config(dir.path(), &out, dcpwizard_core::Standard::Interop);
    assert_eq!(create_dcp(&config), 0, "the Interop DCP must package");

    assert!(
        sound_mxfs(&out).is_empty(),
        "Interop asks for no silent track: {:?}",
        sound_mxfs(&out)
    );
    let cpl = read_cpl(&out);
    assert!(!cpl.contains("CompositionMetadataAsset"), "{cpl}");
    assert!(!cpl.contains("<MainSound>"), "{cpl}");
}
