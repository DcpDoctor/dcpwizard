//! `create --audio-map 1:L,2:R,1:C@-6`: the mapped WAV carries the source
//! channels where the map put them, at the gains it asked for, and packages into
//! a DCP.

use std::path::{Path, PathBuf};

use dcpwizard_core::audio_map::apply_audio_map;
use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

const SAMPLE_RATE: u32 = 48_000;
const BITS_PER_SAMPLE: u16 = 24;
const FPS: u32 = 24;
const FRAMES: u64 = 6;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;

const LEFT_SAMPLE: i32 = 1 << 22;
const RIGHT_SAMPLE: i32 = 1 << 20;
/// -6 dB as an amplitude factor.
const HALF_GAIN: f64 = 0.501_187_233_627_272_3;

const FIVE_ONE_CHANNELS: usize = 6;

const MAP: &str = "1:L,2:R,1:C@-6";

fn write_stereo(path: &Path, frames: usize) -> PathBuf {
    let spec = WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: BITS_PER_SAMPLE,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).unwrap();
    for _ in 0..frames {
        writer.write_sample(LEFT_SAMPLE).unwrap();
        writer.write_sample(RIGHT_SAMPLE).unwrap();
    }
    writer.finalize().unwrap();
    path.to_path_buf()
}

/// A codestream directory holding `FRAMES` black frames.
fn make_frames(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(WIDTH, HEIGHT, FPS, &seed).expect("encode frame");
    for index in 0..FRAMES {
        std::fs::copy(&seed, dir.join(format!("frame_{index:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
    dir.to_path_buf()
}

#[test]
fn a_mapped_stereo_source_lands_on_the_named_lanes_at_the_named_gains() {
    let dir = tempfile::tempdir().unwrap();
    let frames = (FRAMES * SAMPLE_RATE as u64 / FPS as u64) as usize;
    let source = write_stereo(&dir.path().join("stereo.wav"), frames);
    let mapped = dir.path().join("mapped.wav");

    let applied = apply_audio_map(MAP, &source, &mapped).expect("the map applies");
    assert_eq!(applied.report.input_channels, 2);
    assert_eq!(
        applied.report.output_channels, FIVE_ONE_CHANNELS,
        "the centre lane widens the track to 5.1"
    );
    assert_eq!(applied.report.frames, frames);
    assert_eq!(applied.report.clipped_samples, 0);
    assert!(
        !applied.pure_routing,
        "a -6 dB cell means the samples were scaled, not just moved"
    );

    let mut reader = WavReader::open(&mapped).unwrap();
    assert_eq!(reader.spec().channels as usize, FIVE_ONE_CHANNELS);
    assert_eq!(reader.spec().bits_per_sample, BITS_PER_SAMPLE);
    let first: Vec<i32> = reader
        .samples::<i32>()
        .take(FIVE_ONE_CHANNELS)
        .map(|sample| sample.unwrap())
        .collect();
    assert_eq!(first[0], LEFT_SAMPLE, "L is the first source channel");
    assert_eq!(first[1], RIGHT_SAMPLE, "R is the second source channel");
    let want_centre = (LEFT_SAMPLE as f64 * HALF_GAIN).round() as i32;
    assert_eq!(
        first[2], want_centre,
        "C is L at -6 dB, which is half its amplitude within rounding"
    );
    assert_eq!(
        &first[3..],
        &[0, 0, 0],
        "the lanes the map named nothing for are silent"
    );

    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Mapping".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(make_frames(&dir.path().join("j2k"))),
        audio_path: Some(mapped),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "the mapped sound must package");
    assert!(
        std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("CPL_")),
        "a mapped create must produce a package"
    );
}
