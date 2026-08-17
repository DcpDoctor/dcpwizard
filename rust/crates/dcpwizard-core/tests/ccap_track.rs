//! Closed-caption (ST 429-12, MainClosedCaption CPL role) across the multi-reel,
//! versions, and VF packaging paths. Mirrors the subtitle-path tests: build small
//! real packages and assert the CCAP track lands in the CPL and ships an MXF.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::versions::{VersionSpec, create_versioned_dcp};
use dcpwizard_core::vf::{ReplacementReel, VfConfig, create_vf};
use std::path::Path;

const FPS: u32 = 24;
const W: u32 = 2048;
const H: u32 = 1080;

fn make_content_frames(dir: &Path, count: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(W, H, FPS, &seed).expect("encode content frame");
    for i in 0..count {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

fn make_wav(path: &Path, frames: usize) {
    let sample_rate = 48_000u32;
    let channels = 2u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let n_samples = frames as u64 * (sample_rate as u64 / FPS as u64);
    let data_len = n_samples * block_align as u64;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data_len as u32).to_le_bytes());
    w.resize(w.len() + data_len as usize, 0);
    std::fs::write(path, &w).unwrap();
}

fn make_srt(path: &Path, text: &str) {
    // spans the whole 48-frame program so both reels of a mid-split get a cue
    std::fs::write(path, format!("1\n00:00:00,100 --> 00:00:02,000\n{text}\n")).unwrap();
}

fn read_cpls(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_") && n.ends_with(".xml"))
        })
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect()
}

fn mxf_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".mxf"))
        .collect()
}

// Reel splitting must emit a MainClosedCaption per reel, alongside the picture.
#[test]
fn multi_reel_carries_closed_captions() {
    let root = tempfile::tempdir().unwrap();
    let j2k = root.path().join("j2k");
    make_content_frames(&j2k, 48);
    let wav = root.path().join("a.wav");
    make_wav(&wav, 48);
    // one cue in each reel (reel 1 = frames 0..24, reel 2 = 24..48 at 24fps)
    let ccap = root.path().join("cc.srt");
    std::fs::write(
        &ccap,
        "1\n00:00:00,100 --> 00:00:00,900\nReel one\n\n\
         2\n00:00:01,100 --> 00:00:01,900\nReel two\n",
    )
    .unwrap();

    let out = root.path().join("dcp");
    let config = DcpConfig {
        title: "Reeled".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Feature,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(j2k),
        audio_path: Some(wav),
        ccap_path: Some(ccap),
        ccap_language: "en".into(),
        subtitle_opts: font_options(),
        reel_split_frames: vec![24],
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "reel-split create with ccap");

    let cpl = read_cpls(&out).pop().expect("one CPL");
    let reels = cpl.matches("<Reel>").count();
    assert_eq!(reels, 2, "split into two reels");
    assert_eq!(
        cpl.matches("<MainClosedCaption>").count(),
        2,
        "each reel carries a MainClosedCaption"
    );
    let ccap_mxfs = mxf_names(&out)
        .into_iter()
        .filter(|n| n.starts_with("ccap_"))
        .count();
    assert_eq!(ccap_mxfs, 2, "one ccap MXF per reel");
}

// A version with a ccap track emits MainClosedCaption in its CPL.
#[test]
fn version_carries_closed_captions() {
    let root = tempfile::tempdir().unwrap();
    let j2k = root.path().join("j2k");
    make_content_frames(&j2k, 24);
    let ccap = root.path().join("cc.srt");
    make_srt(&ccap, "Version caption");

    let out = root.path().join("pkg");
    let base = DcpConfig {
        title: "Master".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(j2k),
        subtitle_opts: font_options(),
        ..Default::default()
    };
    let versions = vec![VersionSpec {
        title: "Captioned".into(),
        subtitle: None,
        subtitle_language: None,
        ccap: Some(ccap),
        audio: None,
        kind: None,
    }];
    assert_eq!(create_versioned_dcp(&base, &versions), 0);

    let cpl = read_cpls(&out).pop().expect("one version CPL");
    assert!(
        cpl.contains("<MainClosedCaption>"),
        "version CPL carries a MainClosedCaption"
    );
}

// A VF that adds a ccap track references the OV picture and ships one ccap MXF.
#[test]
fn vf_adds_closed_caption_track() {
    let root = tempfile::tempdir().unwrap();
    let ov_j2k = root.path().join("ov_j2k");
    make_content_frames(&ov_j2k, 24);
    let ov_wav = root.path().join("ov.wav");
    make_wav(&ov_wav, 24);
    let ov = root.path().join("ov");
    let ov_config = DcpConfig {
        title: "OV".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Feature,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: ov.clone(),
        j2k_dir: Some(ov_j2k),
        audio_path: Some(ov_wav),
        ..Default::default()
    };
    assert_eq!(create_dcp(&ov_config), 0);

    let ccap = root.path().join("cc.srt");
    make_srt(&ccap, "VF caption");
    let vf = root.path().join("vf");
    let vf_config = VfConfig {
        ov_dir: ov,
        vf_dir: vf.clone(),
        title: "OV with CC".into(),
        subtitle_language: "en".into(),
        subtitle_opts: font_options(),
        replacement_reels: vec![ReplacementReel {
            reel_number: 1,
            ccap: Some(ccap),
            ..Default::default()
        }],
        signer: None,
    };
    assert_eq!(create_vf(&vf_config), 0);

    let cpl = read_cpls(&vf).pop().expect("VF CPL");
    assert!(
        cpl.contains("<MainClosedCaption>"),
        "VF CPL carries a MainClosedCaption"
    );
    let ccap_mxfs: Vec<String> = mxf_names(&vf)
        .into_iter()
        .filter(|n| n.starts_with("ccap_"))
        .collect();
    assert_eq!(ccap_mxfs.len(), 1, "VF ships one ccap MXF: {ccap_mxfs:?}");
}

/// A packaged text track has to embed a font, so the tests name the one in the
/// repo instead of depending on the machine carrying a face.
fn font_options() -> dcpwizard_core::subtitle::SubtitleOptions {
    dcpwizard_core::subtitle::SubtitleOptions {
        font_path: Some(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/LiberationSans-Regular.ttf"),
        ),
        ..Default::default()
    }
}
