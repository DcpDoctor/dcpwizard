//! End-to-end head/tail padding: black frames + silence extend a real DCP, the
//! duration math holds, subtitles shift by the head offset, and the package
//! validates with dcpdoctor (via dcpwizard's verify module) with zero errors.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::pad::generate_black_frame;
use std::path::Path;

const FPS: u32 = 24;
const W: u32 = 2048;
const H: u32 = 1080;
const SPF: u64 = 2000; // 48000 / 24 samples per frame

/// Build `count` valid content J2K frames by encoding one black frame and
/// copying it. Byte-identical frames are fine: the checks are on structure and
/// duration, not pixels.
fn make_content_frames(dir: &Path, count: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    generate_black_frame(W, H, FPS, &seed).expect("encode content frame");
    for i in 0..count {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

/// Write a 6-channel 24-bit 48 kHz PCM WAV with `samples` frames of a nonzero
/// ramp. create_dcp expands it to the canonical 16-channel DCP layout.
fn write_6ch_wav(path: &Path, samples: u64) {
    let channels = 6u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let sample_rate = 48_000u32;
    let mut data = Vec::new();
    for s in 0..samples {
        for _ in 0..channels {
            let v = ((s % 100) + 1) as i32;
            data.extend_from_slice(&v.to_le_bytes()[..3]);
        }
    }
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    std::fs::write(path, &w).unwrap();
}

fn read_cpl(dir: &Path) -> String {
    let cpl = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_") && n.ends_with(".xml"))
        })
        .expect("CPL written");
    std::fs::read_to_string(cpl).unwrap()
}

#[test]
fn head_and_tail_padding_extends_duration_and_validates() {
    let dir = tempfile::tempdir().unwrap();
    let content = dir.path().join("j2k");
    make_content_frames(&content, 3);

    let audio = dir.path().join("in.wav");
    write_6ch_wav(&audio, 3 * SPF); // content audio == content frames

    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Pad Test".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        audio_path: Some(audio),
        pad_head: Some("2f".into()),
        pad_tail: Some("2f".into()),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "create must succeed");

    // picture frames = head(2) + content(3) + tail(2) = 7
    let cpl = read_cpl(&out);
    let count = cpl.matches("<Duration>7</Duration>").count();
    // both picture and sound reels run the full padded length
    assert!(
        count >= 2,
        "picture and sound Duration should both be 7 frames:\n{cpl}"
    );

    // dcpdoctor (via the verify module) must report no errors
    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(
        result.errors.is_empty(),
        "dcpdoctor errors: {:?}",
        result.errors
    );
}

#[test]
fn tail_only_padding_grows_picture_but_not_head() {
    let dir = tempfile::tempdir().unwrap();
    let content = dir.path().join("j2k");
    make_content_frames(&content, 4);
    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Tail Pad".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        pad_tail: Some("1s".into()), // 24 frames
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0);
    // 4 content + 24 tail = 28
    let cpl = read_cpl(&out);
    assert!(cpl.contains("<Duration>28</Duration>"), "{cpl}");
    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
}

#[test]
fn head_padding_shifts_srt_cues() {
    // direct proof: a cue at 00:00:01,000 (frame 24 at 24fps) shifted by 2 frames
    // of head padding lands at frame 26 = 00:00:01:02.
    let dir = tempfile::tempdir().unwrap();
    let srt = dir.path().join("in.srt");
    std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nHello\n").unwrap();
    let dcst = dir.path().join("out.xml");
    let opts = dcpwizard_core::subtitle::SubtitleOptions {
        // a packaged text track has to embed a font, so name the one in the repo
        // instead of depending on the machine carrying a face
        font_path: Some(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/LiberationSans-Regular.ttf"),
        ),
        ..Default::default()
    };
    dcpwizard_core::subtitle::srt_to_shifted_dcst(&srt, 2, "en", FPS, &opts, &dcst).unwrap();
    let xml = std::fs::read_to_string(&dcst).unwrap();
    assert!(xml.contains("TimeIn=\"00:00:01:02\""), "shifted in: {xml}");
    assert!(
        xml.contains("TimeOut=\"00:00:02:02\""),
        "shifted out: {xml}"
    );
}

#[test]
fn head_padding_with_srt_shifts_subtitle_track_end() {
    // integration: create a padded DCP with an SRT subtitle and confirm the
    // subtitle track end moved later by the head offset (48 -> 50 frames).
    let dir = tempfile::tempdir().unwrap();
    let content = dir.path().join("j2k");
    make_content_frames(&content, 3);
    let srt = dir.path().join("in.srt");
    std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nHello\n").unwrap();

    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Sub Pad".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        subtitle_path: Some(srt),
        subtitle_language: "en".into(),
        subtitle_opts: dcpwizard_core::subtitle::SubtitleOptions {
            // a font in the repo, so the package does not depend on the machine's
            font_path: Some(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/LiberationSans-Regular.ttf"),
            ),
            ..Default::default()
        },
        pad_head: Some("2f".into()),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0);
    let cpl = read_cpl(&out);
    // subtitle track ends at the shifted TimeOut: 2s + 2 frames = 50 frames
    assert!(
        cpl.contains("<Duration>50</Duration>"),
        "subtitle Duration should reflect the 2-frame shift:\n{cpl}"
    );
}

#[test]
fn head_padding_rejects_supplied_smpte_xml() {
    let dir = tempfile::tempdir().unwrap();
    let content = dir.path().join("j2k");
    make_content_frames(&content, 2);
    // a minimal DCST XML file (extension .xml => treated as authored SMPTE XML)
    let xml_sub = dir.path().join("sub.xml");
    std::fs::write(&xml_sub, "<dcst:SubtitleReel/>").unwrap();
    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Bad".into(),
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        subtitle_path: Some(xml_sub),
        pad_head: Some("2f".into()),
        ..Default::default()
    };
    // head padding cannot re-time authored XML, so this must fail loud
    assert_eq!(create_dcp(&config), -1);
    assert!(!out.join("ASSETMAP.xml").exists());
}

#[test]
fn padding_rejects_reel_splitting() {
    let dir = tempfile::tempdir().unwrap();
    let content = dir.path().join("j2k");
    make_content_frames(&content, 2);
    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Bad".into(),
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        reel_length_minutes: 20,
        pad_tail: Some("2f".into()),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), -1);
}

#[test]
fn unpadded_content_matches_expected_frame_math() {
    // guards the black-frame path is only taken when padding is requested
    let dir = tempfile::tempdir().unwrap();
    let content = dir.path().join("j2k");
    make_content_frames(&content, 5);
    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Plain".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0);
    let cpl = read_cpl(&out);
    assert!(cpl.contains("<Duration>5</Duration>"), "{cpl}");
}
