//! `create --hdr-dci` over a synthetic HDR10 master: the DCI HDR Addendum
//! signalling on the picture MXF and the CPL, the ISDCF content modifier, and
//! the addendum reference white read back out of the packaged codestream.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 2;
const FRAME_RATE: u32 = 24;

const REFERENCE_WHITE_NITS: f64 = 299.6;
const ADDENDUM_REFERENCE_WHITE_CODES: [i32; 3] = [2524, 2546, 2583];
const CODE_TOLERANCE: i32 = 2;
const DCI_PRECISION_BITS: u8 = 12;

const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;
const PQ_C1: f64 = PQ_C3 - PQ_C2 + 1.0;
const PQ_PEAK_NITS: f64 = 10_000.0;

const BT2020_LUMINANCE_WEIGHTS: [f64; 3] = [0.2627, 0.6780, 0.0593];

// 10-bit studio range: luma 64 to 940, chroma 512 either way by 448
const TEN_BIT_LUMA_OFFSET: f64 = 64.0;
const TEN_BIT_LUMA_SPAN: f64 = 876.0;
const TEN_BIT_CHROMA_OFFSET: f64 = 512.0;
const TEN_BIT_CHROMA_SPAN: f64 = 896.0;

// a J2K codestream at the addendum's raised cap
const CODESTREAM_BUFFER_BYTES: usize = 4 * 1024 * 1024;

fn dcpwizard(config_home: &Path) -> Command {
    let mut command = Command::cargo_bin("dcpwizard").unwrap();
    command.env("XDG_CONFIG_HOME", config_home);
    command
}

fn pq_signal_from_nits(nits: f64) -> f64 {
    let ratio = (nits / PQ_PEAK_NITS).max(0.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * ratio) / (1.0 + PQ_C3 * ratio)).powf(PQ_M2)
}

fn studio_range_ycbcr(rgb_signal: [f64; 3]) -> [u16; 3] {
    let weights = BT2020_LUMINANCE_WEIGHTS;
    let luma = weights[0] * rgb_signal[0] + weights[1] * rgb_signal[1] + weights[2] * rgb_signal[2];
    let blue_chroma = (rgb_signal[2] - luma) / (2.0 * (1.0 - weights[2]));
    let red_chroma = (rgb_signal[0] - luma) / (2.0 * (1.0 - weights[0]));
    [
        (TEN_BIT_LUMA_OFFSET + TEN_BIT_LUMA_SPAN * luma).round() as u16,
        (TEN_BIT_CHROMA_OFFSET + TEN_BIT_CHROMA_SPAN * blue_chroma).round() as u16,
        (TEN_BIT_CHROMA_OFFSET + TEN_BIT_CHROMA_SPAN * red_chroma).round() as u16,
    ]
}

// tagged PQ / BT.2020 on the raw input as well, so ffmpeg converts no range on the way in
fn hdr10_master(dir: &Path) -> PathBuf {
    let white = studio_range_ycbcr([pq_signal_from_nits(REFERENCE_WHITE_NITS); 3]);
    let luma_samples = (WIDTH * HEIGHT) as usize;
    let chroma_samples = luma_samples / 4;
    let mut frame = Vec::with_capacity((luma_samples + 2 * chroma_samples) * 2);
    for (plane, sample) in white.iter().enumerate() {
        let samples = if plane == 0 {
            luma_samples
        } else {
            chroma_samples
        };
        for _ in 0..samples {
            frame.extend_from_slice(&sample.to_le_bytes());
        }
    }
    let raw = dir.join("hdr10.yuv");
    std::fs::write(&raw, frame.repeat(FRAMES)).expect("the raw planes have to be written");

    const COLOUR_TAGS: [&str; 8] = [
        "-color_primaries",
        "bt2020",
        "-color_trc",
        "smpte2084",
        "-colorspace",
        "bt2020nc",
        "-color_range",
        "tv",
    ];
    let master = dir.join("hdr10.mkv");
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "rawvideo"])
        .args(["-pix_fmt", "yuv420p10le"])
        .args(["-s", &format!("{WIDTH}x{HEIGHT}")])
        .args(["-r", &FRAME_RATE.to_string()])
        .args(COLOUR_TAGS)
        .arg("-i")
        .arg(&raw)
        .args(["-c:v", "ffv1"])
        .args(COLOUR_TAGS)
        .arg(&master)
        .status()
        .expect("ffmpeg must be installed to build the HDR master");
    assert!(status.success(), "ffmpeg could not write the HDR master");
    let _ = std::fs::remove_file(&raw);
    master
}

fn only_file_starting_with(dir: &Path, prefix: &str) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("the package directory has to be readable")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    found.sort();
    assert_eq!(found.len(), 1, "one {prefix}* in {}", dir.display());
    found.remove(0)
}

fn decoded_first_frame_pixel(picture_mxf: &Path, x: u32, y: u32) -> [i32; 3] {
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&picture_mxf.to_string_lossy())
        .expect("the picture MXF has to open");
    let mut buffer = vec![0u8; CODESTREAM_BUFFER_BYTES];
    let read = reader
        .read_frame(0, &mut buffer, None, None)
        .expect("the first codestream has to read");
    buffer.truncate(read);
    let frame = postkit::grok_decoder::decode(buffer, 0).expect("the codestream has to decode");
    let at = (y * frame.width + x) as usize;
    let shift = i32::from(frame.precision) - i32::from(DCI_PRECISION_BITS);
    assert!(
        shift >= 0,
        "a {}-bit codestream is below the {DCI_PRECISION_BITS} bits a DCI picture carries",
        frame.precision
    );
    [0, 1, 2].map(|component| frame.components[component][at] >> shift)
}

#[test]
fn an_hdr10_master_packages_as_a_dci_hdr_addendum_dcp() {
    let dir = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let master = hdr10_master(dir.path());
    let out = dir.path().join("dcp");

    dcpwizard(config_home.path())
        .args([
            "create",
            "--title",
            "HDR Master",
            "--video",
            master.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--hdr-dci",
            "--hdr-peak-nits",
            &REFERENCE_WHITE_NITS.to_string(),
            "--isdcf-name",
        ])
        .assert()
        .success();

    // the picture descriptor carries the addendum's one colour item and no other
    let picture_mxf = only_file_starting_with(&out, "picture_");
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&picture_mxf.to_string_lossy())
        .expect("the picture MXF has to open");
    assert_eq!(
        reader.transfer_characteristic().unwrap(),
        Some(asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084),
        "the descriptor must carry the ST 2084 TransferCharacteristic UL"
    );
    assert_eq!(
        reader.hdr_metadata().unwrap().color_primaries,
        None,
        "the addendum names no ColorPrimaries item, so the descriptor must carry none"
    );

    let cpl = std::fs::read_to_string(only_file_starting_with(&out, "CPL_")).unwrap();
    assert!(
        cpl.contains(
            "<meta:ExtensionMetadata scope=\"http://www.dcimovies.com/schemas/2018/HDR-Metadata\">"
        ),
        "{cpl}"
    );
    assert!(
        cpl.contains("<meta:Name>Image Encoding Parameters</meta:Name>"),
        "{cpl}"
    );
    assert!(cpl.contains("<meta:Name>EOTF</meta:Name>"), "{cpl}");
    assert!(cpl.contains("<meta:Value>ST 2084</meta:Value>"), "{cpl}");
    let title = cpl
        .split_once("<ContentTitleText>")
        .and_then(|(_, rest)| rest.split_once("</ContentTitleText>"))
        .map(|(title, _)| title.to_string())
        .expect("the CPL has a content title");
    assert!(title.contains("-HDR1"), "ISDCF title without HDR1: {title}");

    // dcpdoctor reads the transfer straight off the essence descriptor
    let hdr = dcpdoctor_core::hdr_validate::validate_hdr_metadata(
        &dcpdoctor_core::hdr_validate::HdrValidateOptions {
            video_path: picture_mxf.clone(),
            expected_transfer: dcpdoctor_core::hdr_validate::TransferFunction::Pq,
            expected_colorimetry: dcpdoctor_core::hdr_validate::Colorimetry::Bt709,
            expected_bit_depth: 0,
            expected_max_cll: 0,
            expected_max_fall: 0,
            expected_max_luminance: 0,
        },
    );
    assert!(hdr.success, "hdr-validate failed: {}", hdr.error);
    assert_eq!(
        hdr.detected.transfer,
        dcpdoctor_core::hdr_validate::TransferFunction::Pq,
        "hdr-validate must read PQ off the packaged picture"
    );

    let white = decoded_first_frame_pixel(&picture_mxf, WIDTH / 2, HEIGHT / 2);
    assert!(
        white
            .iter()
            .zip(ADDENDUM_REFERENCE_WHITE_CODES)
            .all(|(have, want)| (have - want).abs() <= CODE_TOLERANCE),
        "D65 white at {REFERENCE_WHITE_NITS} cd/m² came out {white:?}, not the addendum's \
         {ADDENDUM_REFERENCE_WHITE_CODES:?}"
    );
}

#[test]
fn the_hdr_source_flags_refuse_what_they_cannot_deliver() {
    let dir = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let master = hdr10_master(dir.path());
    let out = dir.path().join("refused");
    let create = [
        "create",
        "--title",
        "T",
        "--video",
        master.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ];

    // --hdr-source names the grade a DCI HDR package is authored from, so it
    // says nothing on its own
    dcpwizard(config_home.path())
        .args(create)
        .args(["--hdr-source", "hdr10"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--hdr-dci"));

    // the tone map lands on SDR, which is not an HDR grade to signal
    dcpwizard(config_home.path())
        .args(create)
        .args(["--hdr-dci", "--allow-generic-hdr-tonemap"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--hdr-dci"));

    assert!(!out.exists(), "a refused request must write no package");
}

#[test]
fn a_master_with_no_dolby_vision_rpu_is_refused_by_name() {
    let dir = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let master = hdr10_master(dir.path());
    let out = dir.path().join("refused");

    dcpwizard(config_home.path())
        .args([
            "create",
            "--title",
            "T",
            "--video",
            master.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--hdr-dci",
            "--hdr-source",
            "dolby-vision",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("no Dolby Vision RPU"));
    assert!(!out.exists(), "a refused request must write no package");
}
