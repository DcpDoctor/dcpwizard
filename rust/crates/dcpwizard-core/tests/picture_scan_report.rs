//! `report --scan-picture` decodes the packaged picture and lists its black and
//! frozen runs, so a package whose picture went wrong is visible in the report
//! rather than only in the log of the encode that made it.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
/// Comfortably over the 2 seconds both detectors need before they report.
const SECONDS: usize = 3;
const FRAMES: usize = SECONDS * FPS as usize;

/// Every frame identical and black, which is what both detectors look for.
fn make_black_frames(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(WIDTH, HEIGHT, FPS, &seed).expect("encode frame");
    for i in 0..FRAMES {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
    dir.to_path_buf()
}

fn build_black_package(out: &Path, frames: PathBuf) {
    let config = DcpConfig {
        title: "Black Reel".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(frames),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "create the package");
}

#[test]
fn the_scanned_report_names_the_black_run_in_the_packaged_picture() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let out = root.join("dcp");
    build_black_package(&out, make_black_frames(&root.join("frames")));

    let report_path = root.join("report.html");
    assert_eq!(
        dcpwizard_core::report::generate_report(&out, &report_path, true),
        0,
        "generate the report"
    );
    let html = std::fs::read_to_string(&report_path).unwrap();
    let section = html
        .split("<h2>Picture</h2>")
        .nth(1)
        .expect("picture section");

    assert!(
        section.contains("picture_") && section.contains(".mxf"),
        "the picture track is named: {section}"
    );
    assert!(
        section.contains("black picture from 00:00:00:00"),
        "the black run is listed from the first frame: {section}"
    );
    assert!(
        section.contains("frozen picture from 00:00:00:00"),
        "identical frames read as frozen: {section}"
    );
    assert!(
        section.contains(">review</td>"),
        "a run wants a look: {section}"
    );
}

#[test]
fn a_report_without_the_scan_says_the_picture_was_not_scanned() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let out = root.join("dcp");
    build_black_package(&out, make_black_frames(&root.join("frames")));

    let report_path = root.join("report.html");
    assert_eq!(
        dcpwizard_core::report::generate_report(&out, &report_path, false),
        0,
        "generate the report"
    );
    let html = std::fs::read_to_string(&report_path).unwrap();
    let section = html
        .split("<h2>Picture</h2>")
        .nth(1)
        .expect("the section is there either way");
    assert!(section.contains("Not scanned"), "{section}");
    assert!(section.contains("--scan-picture"), "{section}");
    assert!(
        !section.contains("black picture from"),
        "nothing decoded, so nothing can be reported: {section}"
    );
}
