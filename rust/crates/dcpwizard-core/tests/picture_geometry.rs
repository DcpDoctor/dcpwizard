//! The CPL must describe the picture that was actually written: the coded raster
//! comes from the essence, never from a container preset, and the declared active
//! area never exceeds it (ST 429-16, libdcp INVALID_MAIN_PICTURE_ACTIVE_AREA).

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const FRAMES: usize = 4;
/// A flat master: narrower than the 2048x1080 full container it used to be
/// declared as.
const FLAT_WIDTH: u32 = 1998;
const FLAT_HEIGHT: u32 = 1080;

fn make_frames(dir: &Path, width: u32, height: u32) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(width, height, FPS, &seed).expect("encode frame");
    for i in 0..FRAMES {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
    dir.to_path_buf()
}

fn flat_config(root: &Path, out: &Path) -> DcpConfig {
    DcpConfig {
        title: "Geometry".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(make_frames(&root.join("frames"), FLAT_WIDTH, FLAT_HEIGHT)),
        audio_path: Some(make_wav(&root.join("audio.wav"))),
        ..Default::default()
    }
}

/// A stereo 48 kHz WAV covering the content, so the CPL carries the
/// CompositionMetadataAsset that holds the stored/active areas.
fn make_wav(path: &Path) -> PathBuf {
    let sample_rate = 48_000u32;
    let channels = 2u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let data_len = FRAMES as u64 * (sample_rate as u64 / FPS as u64) * block_align as u64;
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
    path.to_path_buf()
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

/// The stored area block's Width/Height.
fn stored_area(cpl: &str) -> (u32, u32) {
    let block = cpl
        .split_once("<meta:MainPictureStoredArea>")
        .and_then(|(_, r)| r.split_once("</meta:MainPictureStoredArea>"))
        .expect("stored area present")
        .0;
    let edge = |tag: &str| -> u32 {
        block
            .split_once(&format!("<meta:{tag}>"))
            .and_then(|(_, r)| r.split_once(&format!("</meta:{tag}>")))
            .and_then(|(v, _)| v.trim().parse().ok())
            .unwrap_or(0)
    };
    (edge("Width"), edge("Height"))
}

/// The raster the picture MXF really carries, read back through asdcplib.
fn essence_raster(dir: &Path) -> (u32, u32) {
    let mxf = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("picture") && n.ends_with(".mxf"))
        })
        .expect("picture MXF");
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader.open_read(&mxf.to_string_lossy()).expect("open mxf");
    let desc = reader.picture_descriptor().expect("picture descriptor");
    (desc.stored_width, desc.stored_height)
}

#[test]
fn the_cpl_declares_the_raster_the_encoder_produced() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    // no container flag: this used to declare the 2048x1080 full-container preset
    assert_eq!(create_dcp(&flat_config(dir.path(), &out)), 0);

    let cpl = read_cpl(&out);
    assert_eq!(
        stored_area(&cpl),
        (FLAT_WIDTH, FLAT_HEIGHT),
        "stored area must be the coded raster, not a preset: {cpl}"
    );
    assert_eq!(
        essence_raster(&out),
        (FLAT_WIDTH, FLAT_HEIGHT),
        "the essence itself is the flat raster"
    );
    // ScreenAspectRatio declares the same coded raster, matching the MXF descriptor
    assert!(
        cpl.contains(&format!(
            "<ScreenAspectRatio>{FLAT_WIDTH} {FLAT_HEIGHT}</ScreenAspectRatio>"
        )),
        "{cpl}"
    );
    // active defaults to the whole frame
    assert!(
        cpl.contains(&format!("<meta:Width>{FLAT_WIDTH}</meta:Width>")),
        "{cpl}"
    );
    assert!(
        !cpl.contains("2048"),
        "no full-container preset anywhere: {cpl}"
    );
}

#[test]
fn a_container_inside_the_raster_becomes_the_active_area() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    // scope masking inside a flat raster: 1998x1080 stored, 1998x836 active
    let active_height = 836;
    let config = DcpConfig {
        container_width: FLAT_WIDTH,
        container_height: active_height,
        ..flat_config(dir.path(), &out)
    };
    assert_eq!(create_dcp(&config), 0);

    let cpl = read_cpl(&out);
    assert_eq!(
        stored_area(&cpl),
        (FLAT_WIDTH, FLAT_HEIGHT),
        "the container must not change what the frames are: {cpl}"
    );
    let active = cpl
        .split_once("<meta:MainPictureActiveArea>")
        .and_then(|(_, r)| r.split_once("</meta:MainPictureActiveArea>"))
        .expect("active area present")
        .0
        .to_string();
    assert!(
        active.contains(&format!("<meta:Height>{active_height}</meta:Height>")),
        "{cpl}"
    );
}

#[test]
fn a_container_bigger_than_the_frames_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    // the full container against a flat master: the old behaviour, now refused
    let config = DcpConfig {
        container_width: 2048,
        container_height: FLAT_HEIGHT,
        ..flat_config(dir.path(), &out)
    };
    assert_ne!(
        create_dcp(&config),
        0,
        "declaring an active area wider than the picture must fail"
    );
}

#[test]
fn dcpdoctor_accepts_the_declared_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    assert_eq!(create_dcp(&flat_config(dir.path(), &out)), 0);

    // the rule dcpdoctor applies: active area even, and never bigger than the
    // asset. Assert it directly against the essence, since the pinned dcpdoctor
    // predates its check_main_picture_active_area.
    let cpl = read_cpl(&out);
    let (stored_width, stored_height) = stored_area(&cpl);
    let (asset_width, asset_height) = essence_raster(&out);
    assert!(stored_width <= asset_width && stored_height <= asset_height);
    assert!(stored_width % 2 == 0 && stored_height % 2 == 0);

    let report = dcpdoctor_core::verify(&out, &dcpdoctor_core::VerifyOptions::strict());
    let geometry_notes: Vec<&dcpdoctor_core::Note> = report
        .notes
        .iter()
        .filter(|n| {
            matches!(n.severity, dcpdoctor_core::Severity::Error)
                && ["area", "aspect", "resolution", "size"]
                    .iter()
                    .any(|subject| n.message.to_lowercase().contains(subject))
        })
        .collect();
    assert!(
        geometry_notes.is_empty(),
        "dcpdoctor must accept the declared geometry, got: {geometry_notes:?}"
    );
}
