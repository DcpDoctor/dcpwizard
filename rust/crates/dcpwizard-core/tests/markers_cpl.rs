//! Markers reach the package: a created DCP's CPL carries a real ST 429-7
//! MainMarkers asset, and dcpdoctor's Bv2.1 marker checks pass on it.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 8;

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

fn base_config(root: &Path, out: &Path) -> DcpConfig {
    DcpConfig {
        title: "Marker Test".into(),
        standard: dcpwizard_core::Standard::Smpte,
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

#[test]
fn a_created_dcp_carries_a_main_markers_asset_dcpdoctor_accepts() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    assert_eq!(create_dcp(&base_config(dir.path(), &out)), 0);

    let cpl = read_cpl(&out);
    let markers = cpl.find("<MainMarkers>").expect("CPL carries MainMarkers");
    let picture = cpl.find("<MainPicture>").expect("CPL carries MainPicture");
    // ST 429-7 orders the AssetList: MainMarkers, MainPicture, MainSound, ...
    assert!(markers < picture, "MainMarkers must precede MainPicture");
    assert!(cpl.contains("<Label>FFOC</Label>"), "{cpl}");
    assert!(cpl.contains("<Offset>1</Offset>"), "FFOC at frame 1: {cpl}");
    assert!(cpl.contains("<Label>LFOC</Label>"), "{cpl}");
    assert!(
        cpl.contains(&format!("<Offset>{}</Offset>", FRAMES - 1)),
        "LFOC one before the reel end: {cpl}"
    );
    // the marker track counts in the picture's units or its offsets mean nothing
    assert!(
        cpl.contains(&format!("<EditRate>{FPS} 1</EditRate>")),
        "{cpl}"
    );

    // dcpdoctor also asks for FFMC/LFMC in strict mode, but those bound the
    // moving credits, which this composition does not have, so only the Bv2.1
    // pair and the marker track itself have to come out clean.
    let report = dcpdoctor_core::verify(&out, &dcpdoctor_core::VerifyOptions::strict());
    let marker_notes: Vec<&dcpdoctor_core::Note> = report
        .notes
        .iter()
        .filter(|n| {
            matches!(
                n.code,
                dcpdoctor_core::Code::MarkerMissing | dcpdoctor_core::Code::MarkerInvalid
            ) && ["FFOC", "LFOC", "MainMarkers"]
                .iter()
                .any(|subject| n.message.contains(subject))
        })
        .collect();
    assert!(
        marker_notes.is_empty(),
        "dcpdoctor must accept the marker track, got: {marker_notes:?}"
    );
}

#[test]
fn given_markers_replace_the_default_pair_in_the_cpl() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    let config = DcpConfig {
        markers: vec!["FFEC=00:00:00:02".into(), "LFEC=5".into()],
        ..base_config(dir.path(), &out)
    };
    assert_eq!(create_dcp(&config), 0);

    let cpl = read_cpl(&out);
    assert!(cpl.contains("<Label>FFEC</Label>"), "{cpl}");
    assert!(cpl.contains("<Offset>2</Offset>"), "{cpl}");
    assert!(cpl.contains("<Label>LFEC</Label>"), "{cpl}");
    assert!(cpl.contains("<Offset>5</Offset>"), "{cpl}");
    assert!(
        !cpl.contains("<Label>FFOC</Label>"),
        "given markers replace the default pair: {cpl}"
    );
}

#[test]
fn a_marker_past_the_composition_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    let config = DcpConfig {
        markers: vec![format!("FFEC={}", FRAMES * 10)],
        ..base_config(dir.path(), &out)
    };
    assert_ne!(create_dcp(&config), 0, "an out-of-range marker must fail");
}
