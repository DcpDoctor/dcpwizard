//! Assemble OV end-to-end (dom#1675): build small real DCPs, assemble them into
//! one composition, validate with dcpdoctor, and check the loud-failure cases
//! (frame-rate mismatch, encrypted input).

use dcpwizard_core::assemble::{AssembleConfig, assemble};
use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use std::path::{Path, PathBuf};

const W: u32 = 2048;
const H: u32 = 1080;

/// What `create_dcp` packages when a job brings no audio: 5.1 silence.
const FILL_SOUND_CONFIGURATION: &str = "51/L,R,C,LFE,Ls,Rs";
const DCP_SAMPLE_RATE: u32 = 48_000;

fn make_content_frames(dir: &Path, count: usize, fps: u32) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(W, H, fps, &seed).expect("encode content frame");
    for i in 0..count {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

fn make_dcp(
    root: &Path,
    name: &str,
    title: &str,
    frames: usize,
    fps: u32,
    encrypt: bool,
) -> PathBuf {
    let content = root.join(format!("{name}_j2k"));
    make_content_frames(&content, frames, fps);
    let out = root.join(name);
    let config = DcpConfig {
        title: title.into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: fps,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        encrypt,
        key_out: encrypt.then(|| root.join(format!("{name}_keys.json"))),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "create {name} must succeed");
    out
}

fn cpl_reel_count(dir: &Path) -> usize {
    let cpl = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_"))
        })
        .unwrap();
    let xml = std::fs::read_to_string(cpl).unwrap();
    xml.matches("<Reel>").count()
}

#[test]
fn assembles_two_dcps_into_one_valid_ov() {
    let root = tempfile::tempdir().unwrap();
    let a = make_dcp(root.path(), "short", "The Short", 48, 24, false);
    let b = make_dcp(root.path(), "feature", "The Feature", 72, 24, false);

    let out = root.path().join("program");
    let config = AssembleConfig {
        inputs: vec![a, b],
        output_dir: out.clone(),
        title: "Program".into(),
        signer: None,
    };
    assert_eq!(assemble(&config), 0);

    // one new CPL with both source reels, in input order
    let cpls: Vec<PathBuf> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_"))
        })
        .collect();
    assert_eq!(cpls.len(), 1, "assemble writes exactly one CPL");
    assert_eq!(cpl_reel_count(&out), 2, "two reels, one per input");
    let xml = std::fs::read_to_string(&cpls[0]).unwrap();
    assert!(xml.contains("<ContentTitleText>Program</ContentTitleText>"));

    // the sound the program ships is the sources' 5.1 silent fill, read back off
    // the copied track file
    assert!(
        xml.contains("<meta:CompositionMetadataAsset"),
        "SMPTE Bv2.1 wants the metadata asset: {xml}"
    );
    assert!(
        xml.contains(&format!(
            "<meta:MainSoundConfiguration>{FILL_SOUND_CONFIGURATION}</meta:MainSoundConfiguration>"
        )),
        "assembled CPL must declare the sound it ships: {xml}"
    );
    assert!(
        xml.contains(&format!(
            "<meta:MainSoundSampleRate>{DCP_SAMPLE_RATE} 1</meta:MainSoundSampleRate>"
        )),
        "{xml}"
    );

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}

#[test]
fn frame_rate_mismatch_fails_loud() {
    let root = tempfile::tempdir().unwrap();
    let a = make_dcp(root.path(), "a", "A", 48, 24, false);
    let b = make_dcp(root.path(), "b", "B", 50, 25, false);

    let config = AssembleConfig {
        inputs: vec![a, b],
        output_dir: root.path().join("out"),
        title: "X".into(),
        signer: None,
    };
    assert_eq!(assemble(&config), -1, "mixed frame rates must fail");
}

#[test]
fn encrypted_input_fails_loud() {
    let root = tempfile::tempdir().unwrap();
    let a = make_dcp(root.path(), "a", "A", 48, 24, false);
    let enc = make_dcp(root.path(), "enc", "Enc", 48, 24, true);

    let config = AssembleConfig {
        inputs: vec![a, enc],
        output_dir: root.path().join("out"),
        title: "X".into(),
        signer: None,
    };
    assert_eq!(assemble(&config), -1, "encrypted input must fail");
}
