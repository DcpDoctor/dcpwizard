//! A 24000/1001 source packaged at 24 fps: the picture is read 1:1 and plays
//! 0.1% faster, and the sound is pulled up by the same 1000/1001, so both tracks
//! hold exactly as many frames as the source did.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use dcpwizard_core::dcp::{DcpConfig, create_dcp};

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const DCP_FPS: u32 = 24;
const SOURCE_FRAMES: u32 = 1001;
/// The source runs 1001/24000 s per frame, and the sound covers all of it:
/// 1001 * 1001 * 48000 / 24000 samples.
const SOURCE_SAMPLES: u64 = 1001 * 1001 * 2;
const AUDIO_SAMPLE_RATE: u64 = 48_000;

fn make_ntsc_clip(path: &Path) {
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=size={WIDTH}x{HEIGHT}:rate=24000/1001"),
            "-frames:v",
            &SOURCE_FRAMES.to_string(),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .status()
        .expect("run ffmpeg");
    assert!(ok.success() && path.exists(), "ffmpeg wrote no 23.976 clip");
}

fn make_sine(path: &Path) {
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-af",
            &format!("atrim=end_sample={SOURCE_SAMPLES}"),
            "-ac",
            "2",
            "-c:a",
            "pcm_s24le",
        ])
        .arg(path)
        .status()
        .expect("run ffmpeg");
    assert!(ok.success() && path.exists(), "ffmpeg wrote no sine");
}

fn find_essence(dir: &Path, prefix: &str) -> std::path::PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".mxf"))
        })
        .unwrap_or_else(|| panic!("{prefix} MXF written to {}", dir.display()))
}

fn wav_sample_count(path: &Path) -> u64 {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=duration_ts",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("a sample count")
}

#[test]
fn a_23_976_source_packages_1001_frames_of_picture_and_sound_at_24_fps() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("ntsc.mp4");
    let sine = dir.path().join("sound.wav");
    make_ntsc_clip(&clip);
    make_sine(&sine);

    let probed = dcpwizard_core::probe::probe_video(&clip).expect("probe the source");
    assert_eq!((probed.fps_num, probed.fps_den), (24_000, 1_001));

    let conform =
        dcpwizard_core::hfr::conform_source_to_dcp(probed.fps_num, probed.fps_den, DCP_FPS);
    assert!(conform.audio_pull_up);

    let work = dir.path().join("encode");
    let encoded = postkit::pipeline::run_encode_with_options(
        &clip,
        &work,
        &postkit::pipeline::EncodeRunOptions {
            fps: postkit::encode::FrameRate::whole(DCP_FPS),
            read_source_at: conform.read_source_at,
            ..postkit::pipeline::EncodeRunOptions::default()
        },
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(AtomicBool::new(false)),
        |_progress| {},
        |_message| {},
    )
    .expect("the conformed encode runs");
    assert_eq!(
        encoded.frames_encoded,
        u64::from(SOURCE_FRAMES),
        "every source frame is packaged once, none duplicated"
    );

    let pulled_up = dir.path().join("pullup.wav");
    dcpwizard_core::hfr::audio_pull_up(&sine, &pulled_up).expect("pull the sound up");
    let samples = wav_sample_count(&pulled_up);
    let picture_samples = u64::from(SOURCE_FRAMES) * AUDIO_SAMPLE_RATE / u64::from(DCP_FPS);
    assert!(
        samples.abs_diff(picture_samples) <= AUDIO_SAMPLE_RATE / u64::from(DCP_FPS),
        "{samples} samples against {picture_samples} of picture"
    );

    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "NTSC Film".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: DCP_FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(encoded.j2k_dir),
        audio_path: Some(pulled_up),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "the conformed DCP must package");

    let mut picture = asdcplib::jp2k::MxfReader::new();
    picture
        .open_read(find_essence(&out, "picture").to_str().unwrap())
        .expect("open picture MXF");
    assert_eq!(
        picture
            .picture_descriptor()
            .expect("picture descriptor")
            .container_duration,
        SOURCE_FRAMES,
        "an fps filter on its own would have made {}",
        SOURCE_FRAMES + 1
    );

    let mut sound = asdcplib::pcm::MxfReader::new();
    sound
        .open_read(find_essence(&out, "sound").to_str().unwrap())
        .expect("open sound MXF");
    assert_eq!(
        sound
            .audio_descriptor()
            .expect("audio descriptor")
            .container_duration,
        SOURCE_FRAMES,
        "the pulled-up sound covers the picture to the frame"
    );

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(
        result.errors.is_empty(),
        "verify errors: {:?}",
        result.errors
    );
}
