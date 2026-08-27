//! The QC report measures the packaged sound and states the Leq(m) limit for
//! the composition's content kind, so a trailer that is too loud is visible in
//! the report rather than only in a separate loudness tool.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const SECONDS: usize = 2;
const FRAMES: usize = SECONDS * FPS as usize;
const SAMPLE_RATE: u32 = 48_000;
const SINE_HZ: f64 = 1000.0;
const SINE_AMPLITUDE: f64 = 0.1;

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

/// A stereo 24-bit sine, loud enough that Leq(m) lands on a real number rather
/// than the negative infinity silence gives.
fn make_sine_wav(path: &Path) -> PathBuf {
    let channels = 2u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let samples = SECONDS as u64 * SAMPLE_RATE as u64;
    let mut data = Vec::with_capacity(samples as usize * block_align as usize);
    for n in 0..samples {
        let phase = 2.0 * std::f64::consts::PI * SINE_HZ * n as f64 / SAMPLE_RATE as f64;
        let value = (phase.sin() * SINE_AMPLITUDE * 8_388_607.0) as i32;
        for _ in 0..channels {
            data.extend_from_slice(&value.to_le_bytes()[..3]);
        }
    }
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * block_align as u32).to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(path, &wav).unwrap();
    path.to_path_buf()
}

#[test]
fn the_report_measures_the_packaged_sound_against_the_trailer_limit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let out = root.join("dcp");
    let config = DcpConfig {
        title: "Loud Trailer".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Trailer,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(make_frames(&root.join("frames"))),
        audio_path: Some(make_sine_wav(&root.join("audio.wav"))),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "create the package");

    let report_path = root.join("report.html");
    assert_eq!(
        dcpwizard_core::report::generate_report(&out, &report_path, false),
        0,
        "generate the report"
    );
    let html = std::fs::read_to_string(&report_path).unwrap();

    assert!(html.contains("<h2>Sound level</h2>"), "{html}");
    assert!(html.contains("ISO 21727"), "the report states the measure");
    assert!(
        html.contains("85 dB maximum"),
        "the trailer limit is stated inline"
    );
    let row = html
        .split("<h2>Sound level</h2>")
        .nth(1)
        .expect("sound level section");
    assert!(
        row.contains("sound_") && row.contains(".mxf"),
        "the sound track is named: {row}"
    );
    let leq: f64 = row
        .split("<td>")
        .find(|cell| cell.contains(" dB</td>"))
        .expect("a measured Leq(m) cell")
        .split(" dB</td>")
        .next()
        .unwrap()
        .parse()
        .expect("Leq(m) must be a number");
    assert!(
        leq.is_finite() && leq > 0.0,
        "Leq(m) must be a real level, got {leq}"
    );
    assert!(
        row.contains(">pass</td>") || row.contains(">fail</td>"),
        "a trailer is judged against its limit: {row}"
    );
}
