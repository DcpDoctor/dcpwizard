//! Verifies that `create --source-colourspace p3` reaches the codestream as real
//! P3 X'Y'Z': encode a solid frame through the route the create handler builds,
//! decode the stored codestream with grk_decompress, and check the X'Y'Z' code
//! values against an independently computed expectation.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use dcpwizard_core::encode::{XyzRoute, xyz_route};
use postkit::colour::ColourSpace;
use postkit::grok_encoder::{self, CompressParams, RawFrame};

const W: u32 = 48;
const H: u32 = 48;
const FPS: u32 = 24;

// SMPTE RP 431-2 P3-DCI primaries with the DCI white point.
const P3_DCI_TO_XYZ: [[f64; 3]; 3] = [
    [0.4451698, 0.2771344, 0.1722827],
    [0.2094917, 0.7215952, 0.0689131],
    [0.0, 0.0470606, 0.9073747],
];

/// Independent expectation: P3 code values (16-bit, 2.6 gamma) to DCI X'Y'Z'
/// 12-bit code values, in f64 with no shared code.
fn expected_xyz(rgb16: [u16; 3]) -> [u16; 3] {
    let lin: Vec<f64> = rgb16
        .iter()
        .map(|&v| (v as f64 / 65535.0).powf(2.6))
        .collect();
    let mut out = [0u16; 3];
    for (i, row) in P3_DCI_TO_XYZ.iter().enumerate() {
        let xyz = (row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2]) * 48.0 / 52.37;
        out[i] = (xyz.clamp(0.0, 1.0).powf(1.0 / 2.6) * 4095.0).round() as u16;
    }
    out
}

fn grk_decompress_bin() -> std::path::PathBuf {
    std::env::var("GRK_DECOMPRESS_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            std::path::PathBuf::from(home).join("bin/grok/bin/grk_decompress")
        })
}

/// Read the first sample of a PGX plane written by grk_decompress.
fn first_pgx_sample(path: &Path) -> u16 {
    let data = std::fs::read(path).unwrap();
    let nl = data.iter().position(|&b| b == b'\n').unwrap();
    let body = &data[nl + 1..];
    u16::from_be_bytes([body[0], body[1]])
}

fn decode_first_pixel(j2c: &Path, dir: &Path) -> [u16; 3] {
    let out = dir.join("decoded.pgx");
    let status = Command::new(grk_decompress_bin())
        .arg("-i")
        .arg(j2c)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("run grk_decompress");
    assert!(status.success(), "grk_decompress failed");
    let mut xyz = [0u16; 3];
    for (i, slot) in xyz.iter_mut().enumerate() {
        *slot = first_pgx_sample(&dir.join(format!("decoded_{i}.pgx")));
    }
    xyz
}

/// Encode one solid frame through `route`, exactly the two compressor fields the
/// create handler sets, and return the codestream directory.
fn encode_solid(rgb16: [u16; 3], route: XyzRoute, dir: &Path) -> std::path::PathBuf {
    let mut data = Vec::with_capacity((W * H) as usize * 6);
    for _ in 0..(W * H) {
        for c in rgb16 {
            data.extend_from_slice(&c.to_be_bytes());
        }
    }
    let params = CompressParams {
        frame_rate: FPS as u16,
        apply_xyz_transform: route.compressor_transform(),
        source_transform: route.frame_transform().unwrap(),
        ..CompressParams::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    grok_encoder::initialize(0);
    let mut produced = false;
    let result = grok_encoder::encode_pipeline(
        dir,
        &params,
        1,
        &cancel,
        || {
            if produced {
                return None;
            }
            produced = true;
            Some(RawFrame::Packed {
                data: std::mem::take(&mut data),
                width: W,
                height: H,
                precision: 16,
                index: 0,
            })
        },
        |_p| {},
    );
    assert!(result.success, "encode failed: {}", result.error);
    dir.join("frame_00000000.j2c")
}

#[test]
fn a_p3_source_encodes_to_p3_xyz_code_values() {
    if !grk_decompress_bin().is_file() {
        eprintln!("skip: grk_decompress not found");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let route = xyz_route(ColourSpace::P3).unwrap();

    for (index, rgb16) in [[65535u16, 0, 0], [0, 45000, 0], [30000, 30000, 30000]]
        .into_iter()
        .enumerate()
    {
        let work = dir.path().join(format!("p3_{index}"));
        let j2c = encode_solid(rgb16, route, &work);
        let got = decode_first_pixel(&j2c, &work);
        let want = expected_xyz(rgb16);
        eprintln!("p3 {rgb16:?}: got {got:?} want {want:?}");
        for c in 0..3 {
            let diff = (got[c] as i32 - want[c] as i32).abs();
            assert!(
                diff <= 40,
                "channel {c} off by {diff} (got {got:?}, want {want:?}) for p3 {rgb16:?}"
            );
        }
    }
}

#[test]
fn p3_red_is_not_the_rec709_red_the_compressor_would_produce() {
    if !grk_decompress_bin().is_file() {
        eprintln!("skip: grk_decompress not found");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let red = [65535u16, 0, 0];

    let p3_dir = dir.path().join("p3");
    let p3 = decode_first_pixel(
        &encode_solid(red, xyz_route(ColourSpace::P3).unwrap(), &p3_dir),
        &p3_dir,
    );
    let rec709_dir = dir.path().join("rec709");
    let rec709 = decode_first_pixel(
        &encode_solid(red, xyz_route(ColourSpace::Rec709).unwrap(), &rec709_dir),
        &rec709_dir,
    );

    assert!(
        p3[0] > rec709[0] + 40,
        "P3 red must reach further than Rec.709 red: {p3:?} vs {rec709:?}"
    );
}
