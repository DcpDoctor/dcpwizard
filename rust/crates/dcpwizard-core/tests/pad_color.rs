//! Verifies that `create --pad-color` runs the colour through the real DCDM
//! transform: generate a solid frame, decode the stored codestream with
//! grk_decompress, and check the X'Y'Z' code values match an independently
//! computed expectation within tolerance.

use std::path::Path;
use std::process::Command;

const W: u32 = 48;
const H: u32 = 48;
const FPS: u32 = 24;

// Independent expectation: Rec.709 RGB (8-bit) -> DCI X'Y'Z' 12-bit code
// values, using the grok/libdcp DCDM pipeline: gamma 2.2 linearize, Rec.709/D65
// matrix, 48/52.37 DCI companding, 2.6 out gamma. Red [255,0,0] -> ~[2817,2183,870].
fn expected_xyz(rgb8: [u8; 3]) -> [u16; 3] {
    let r = (rgb8[0] as f64 / 255.0).powf(2.2);
    let g = (rgb8[1] as f64 / 255.0).powf(2.2);
    let b = (rgb8[2] as f64 / 255.0).powf(2.2);
    let x = 0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b;
    let z = 0.019_333_9 * r + 0.119_192_0 * g + 0.950_304_1 * b;
    let code =
        |v: f64| ((v * 48.0 / 52.37).clamp(0.0, 1.0).powf(1.0 / 2.6) * 4095.0).round() as u16;
    [code(x), code(y), code(z)]
}

/// Read the first sample of a PGX plane written by grk_decompress.
fn first_pgx_sample(path: &Path) -> u16 {
    let data = std::fs::read(path).unwrap();
    // PGX header: "PG ML <depth> <w> <h>\n" then raw big-endian samples
    let nl = data.iter().position(|&b| b == b'\n').unwrap();
    let body = &data[nl + 1..];
    // 12-bit samples are stored as 2 bytes each, big-endian
    u16::from_be_bytes([body[0], body[1]])
}

fn grk_decompress_bin() -> std::path::PathBuf {
    postkit::grok::find_grk_decompress().expect("grk_decompress on PATH")
}

fn decode_first_pixel(j2c: &Path, dir: &Path) -> [u16; 3] {
    let bin = grk_decompress_bin();
    let out = dir.join("decoded.pgx");
    let decode = Command::new(&bin)
        .arg("-i")
        .arg(j2c)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("run grk_decompress");
    assert!(
        decode.status.success(),
        "grk_decompress failed\n  stdout: {}\n  stderr: {}",
        String::from_utf8_lossy(&decode.stdout).trim(),
        String::from_utf8_lossy(&decode.stderr).trim(),
    );
    // multi-component pgx output names planes decoded_0.pgx ..._2.pgx
    let mut xyz = [0u16; 3];
    for (i, slot) in xyz.iter_mut().enumerate() {
        let plane = dir.join(format!("decoded_{i}.pgx"));
        *slot = first_pgx_sample(&plane);
    }
    xyz
}

#[test]
fn pad_color_frame_carries_expected_xyz() {
    let dir = tempfile::tempdir().unwrap();

    for rgb8 in [[255u8, 0, 0], [0, 128, 0], [64, 96, 200]] {
        let rgb16 = [
            rgb8[0] as u16 * 257,
            rgb8[1] as u16 * 257,
            rgb8[2] as u16 * 257,
        ];
        let j2c = dir
            .path()
            .join(format!("solid_{}_{}_{}.j2c", rgb8[0], rgb8[1], rgb8[2]));
        dcpwizard_core::pad::generate_solid_frame(W, H, FPS, rgb16, &j2c)
            .expect("encode solid frame");

        let got = decode_first_pixel(&j2c, dir.path());
        let want = expected_xyz(rgb8);
        eprintln!("rgb {rgb8:?}: got {got:?} want {want:?}");
        for c in 0..3 {
            let diff = (got[c] as i32 - want[c] as i32).abs();
            assert!(
                diff <= 40,
                "channel {c} off by {diff} (got {got:?}, want {want:?}) for rgb {rgb8:?}"
            );
        }
        // a coloured pad is not black
        assert!(got.iter().any(|&v| v > 0), "colour decoded to all-zero");
    }
}
