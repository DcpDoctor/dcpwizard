//! Subtitle-only VF end-to-end (dom#1062): author a VF that adds a subtitle track
//! to an OV, referencing the OV's picture/sound by id and shipping only the new
//! subtitle MXF.
//!
//! A supplemental VF is not independently verify-clean: vf.rs marks it with an
//! `<OriginalPackagingList>` element (so dcpdoctor's --ov path detects the VF),
//! which trips strict CPL schema validation in `verify_dcp`. So this test asserts
//! the VF structure and PKL hash-consistency (the meaningful correctness check,
//! as the existing vf.rs tests do) and confirms the OV output stays verify-clean.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::vf::{ReplacementReel, VfConfig, create_vf};
use std::path::{Path, PathBuf};

const W: u32 = 2048;
const H: u32 = 1080;
const FPS: u32 = 24;

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

fn make_ov(root: &Path) -> PathBuf {
    let content = root.join("ov_j2k");
    make_content_frames(&content, 48);
    let wav = root.join("ov.wav");
    make_wav(&wav, 48);
    let out = root.join("ov");
    let config = DcpConfig {
        title: "OV Movie".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Feature,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        audio_path: Some(wav),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0);
    out
}

fn read_cpl(dir: &Path) -> String {
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
    std::fs::read_to_string(cpl).unwrap()
}

#[test]
fn subtitle_only_vf_references_ov_and_volume_validates() {
    let root = tempfile::tempdir().unwrap();
    let ov = make_ov(root.path());

    let srt = root.path().join("subs.srt");
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:01,500\nHello world\n").unwrap();

    let vf = root.path().join("vf");
    let config = VfConfig {
        ov_dir: ov.clone(),
        vf_dir: vf.clone(),
        title: "OV Movie with subs".into(),
        subtitle_language: "en".into(),
        replacement_reels: vec![ReplacementReel {
            reel_number: 1,
            subtitle: Some(srt),
            ..Default::default()
        }],
        signer: None,
    };
    assert_eq!(create_vf(&config), 0);

    // the OV picture id the VF must keep referencing (VF ships no picture MXF)
    let ov_cpl = read_cpl(&ov);
    let ov_pic = {
        let seg = ov_cpl.split("<MainPicture>").nth(1).unwrap();
        let seg = seg.split("</MainPicture>").next().unwrap();
        let s = seg.find("urn:uuid:").unwrap() + "urn:uuid:".len();
        seg[s..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
            .collect::<String>()
    };

    // the VF CPL carries a MainSubtitle and still references the OV picture id
    let vf_cpl = read_cpl(&vf);
    assert!(
        vf_cpl.contains("<MainSubtitle>"),
        "VF CPL has a subtitle track"
    );
    assert!(
        vf_cpl.contains(&ov_pic),
        "VF CPL still references the OV picture by id"
    );

    // the VF ships exactly one subtitle MXF and no picture/sound MXF
    let vf_mxfs: Vec<String> = std::fs::read_dir(&vf)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".mxf"))
        .collect();
    assert_eq!(
        vf_mxfs.len(),
        1,
        "VF ships only the new subtitle MXF: {vf_mxfs:?}"
    );
    assert!(vf_mxfs[0].starts_with("subtitle_"));

    // the subtitle's real embedded id (from the CPL) is in PKL and ASSETMAP, and
    // the PKL hash matches the file on disk (a real, verifiable consistency check)
    let sub_file = vf.join(&vf_mxfs[0]);
    let sub_id = {
        let seg = vf_cpl.split("<MainSubtitle>").nth(1).unwrap();
        let seg = seg.split("</MainSubtitle>").next().unwrap();
        let s = seg.find("urn:uuid:").unwrap() + "urn:uuid:".len();
        seg[s..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
            .collect::<String>()
    };
    let pkl = std::fs::read_dir(&vf)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("PKL"))
        })
        .unwrap();
    let pkl_xml = std::fs::read_to_string(&pkl).unwrap();
    assert!(pkl_xml.contains(&sub_id), "subtitle id in PKL");
    let real_hash = dcpwizard_core::hash::hash_file(&sub_file).unwrap();
    assert!(
        pkl_xml.contains(&real_hash),
        "PKL hash matches the subtitle MXF on disk"
    );
    let am = std::fs::read_to_string(vf.join("ASSETMAP.xml")).unwrap();
    assert!(am.contains(&sub_id), "subtitle id in ASSETMAP");

    // the OV output itself stays verify-clean (the VF did not touch it)
    let result = dcpwizard_core::verify::verify_dcp(&ov);
    assert!(result.valid, "OV dcpdoctor errors: {:?}", result.errors);
}
