use asdcplib::jp2k::StereoscopicPhase;
use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// the 2K flat container, so no fitting runs between the source and the encode
const WIDTH: u32 = 1998;
const HEIGHT: u32 = 1080;
// a whole number of frames at every rate the tests package, 50 included
const SOURCE_SECONDS: f64 = 0.5;
const SOUND_SAMPLE_RATE: u32 = 48_000;
const SOUND_CHANNELS: u16 = 2;

// a J2K codestream at the 250 Mbit/s DCI cap
const CODESTREAM_BUFFER_BYTES: usize = 2 * 1024 * 1024;

const STEREOSCOPIC_PICTURE_NAMESPACE: &str =
    "http://www.smpte-ra.org/schemas/429-10/2008/Main-Stereo-Picture-CPL";

fn dcpwizard(config_home: &Path) -> Command {
    let mut command = Command::cargo_bin("dcpwizard").unwrap();
    command.env("XDG_CONFIG_HOME", config_home);
    command
}

fn frames_at(fps: u32) -> u32 {
    (f64::from(fps) * SOURCE_SECONDS) as u32
}

fn colour_bars(path: &Path, fps: u32) {
    lavfi_source(path, &format!("testsrc=size={WIDTH}x{HEIGHT}:rate={fps}"));
}

fn solid_colour(path: &Path, colour: &str, fps: u32) {
    lavfi_source(
        path,
        &format!("color=c={colour}:size={WIDTH}x{HEIGHT}:rate={fps}"),
    );
}

fn lavfi_source(path: &Path, source: &str) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!("{source}:duration={SOURCE_SECONDS}"))
        .args(["-pix_fmt", "yuv420p"])
        .arg(path)
        .status()
        .expect("ffmpeg must be installed to build the test source");
    assert!(
        status.success(),
        "ffmpeg failed to write {}",
        path.display()
    );
}

// long enough to cover the picture, so the reel needs no sound padding
fn write_silence(path: &Path) {
    let spec = hound::WavSpec {
        channels: SOUND_CHANNELS,
        sample_rate: SOUND_SAMPLE_RATE,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let samples = (f64::from(SOUND_SAMPLE_RATE) * SOURCE_SECONDS) as usize;
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..samples * SOUND_CHANNELS as usize {
        writer.write_sample(0i32).unwrap();
    }
    writer.finalize().unwrap();
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

fn every_element(xml: &str, element: &str) -> Vec<String> {
    xml.split(&format!("<{element}>"))
        .skip(1)
        .filter_map(|rest| rest.split_once(&format!("</{element}>")))
        .map(|(value, _)| value.to_string())
        .collect()
}

fn verify_errors(package: &Path) -> Vec<String> {
    dcpwizard_core::verify::verify_dcp(package).errors
}

fn verify_reports_no_error(package: &Path) {
    let errors = verify_errors(package);
    assert!(
        errors.is_empty(),
        "verify reported {} error(s) over {}: {errors:?}",
        errors.len(),
        package.display()
    );
}

// the X'Y'Z' triple at the frame's centre, at whatever precision it declares
fn centre_pixel(codestream: Vec<u8>) -> [i32; 3] {
    let frame = postkit::grok_decoder::decode(codestream, 0).expect("the codestream has to decode");
    let at = ((frame.height / 2) * frame.width + frame.width / 2) as usize;
    [0, 1, 2].map(|component| frame.components[component][at])
}

// colour bars shot and packaged at fps, into a 2K flat DCP
fn package_at(dir: &Path, config_home: &Path, fps: u32) -> PathBuf {
    let video = dir.join(format!("bars{fps}.mp4"));
    colour_bars(&video, fps);
    let sound = dir.join("silence.wav");
    write_silence(&sound);
    let out = dir.join(format!("dcp{fps}"));

    dcpwizard(config_home)
        .args([
            "create",
            "--title",
            "HFR",
            "--video",
            video.to_str().unwrap(),
            "--audio",
            sound.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--container",
            "2k-flat",
            "--frame-rate",
            &fps.to_string(),
        ])
        .assert()
        .success();
    out
}

#[test]
fn every_hfr_rate_packages_a_2k_dcp_at_its_own_rate() {
    let dir = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();

    for fps in [48, 50, 60] {
        let out = package_at(dir.path(), config_home.path(), fps);
        let rate = format!("{fps} 1");

        let cpl = std::fs::read_to_string(only_file_starting_with(&out, "CPL_")).unwrap();
        let edit_rates = every_element(&cpl, "EditRate");
        assert!(
            !edit_rates.is_empty() && edit_rates.iter().all(|have| *have == rate),
            "the CPL has to carry {rate} as its only edit rate, not {edit_rates:?}"
        );
        assert_eq!(
            every_element(&cpl, "FrameRate"),
            vec![rate.clone()],
            "the picture's FrameRate has to be {rate}"
        );

        let picture_mxf = only_file_starting_with(&out, "picture_");
        let mut reader = asdcplib::jp2k::MxfReader::new();
        reader
            .open_read(&picture_mxf.to_string_lossy())
            .expect("the picture MXF has to open");
        let descriptor = reader
            .picture_descriptor()
            .expect("the picture descriptor has to read");
        assert_eq!(
            (
                descriptor.edit_rate.numerator,
                descriptor.edit_rate.denominator
            ),
            (fps as i32, 1),
            "the picture essence has to be wrapped at {fps} fps"
        );
        assert_eq!(
            descriptor.container_duration,
            frames_at(fps),
            "the picture MXF has to carry every source frame at {fps} fps"
        );

        verify_reports_no_error(&out);
    }
}

// ST 429-2 gives a composition edit rate no value above 60, whatever the addendum adds
#[test]
fn a_rate_above_the_st_429_2_cap_is_refused_before_it_encodes() {
    let dir = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let sound = dir.path().join("silence.wav");
    write_silence(&sound);

    for fps in [96, 100, 120] {
        let video = dir.path().join(format!("bars{fps}.mp4"));
        colour_bars(&video, fps);
        let out = dir.path().join(format!("refused{fps}"));

        dcpwizard(config_home.path())
            .args([
                "create",
                "--title",
                "HFR",
                "--video",
                video.to_str().unwrap(),
                "--audio",
                sound.to_str().unwrap(),
                "-o",
                out.to_str().unwrap(),
                "--container",
                "2k-flat",
                "--frame-rate",
                &fps.to_string(),
            ])
            .assert()
            .failure()
            .stdout(predicate::str::contains(format!(
                "frame rate {fps} fps cannot be packaged: ST 429-2 stops a 2K composition edit rate at 60 fps"
            )));
        assert!(!out.exists(), "a refused request must write no package");
    }
}

// the HFR rates need the 2K container, so a 4K request at one is a refusal
#[test]
fn four_k_refuses_an_hfr_rate_before_it_encodes() {
    let dir = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let video = dir.path().join("bars60.mp4");
    colour_bars(&video, 60);
    let out = dir.path().join("refused");

    dcpwizard(config_home.path())
        .args([
            "create",
            "--title",
            "T",
            "--video",
            video.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--container",
            "4k-flat",
            "--frame-rate",
            "60",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "4K DCP is limited to 24/25/30 fps; 60 fps requires a 2K container",
        ));
    assert!(!out.exists(), "a refused request must write no package");
}

// red is the X-heavy colour in X'Y'Z' and blue the Z-heavy one
#[test]
fn a_stereoscopic_dcp_carries_one_eye_in_each_phase() {
    const FPS: u32 = 24;
    let dir = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let left = dir.path().join("left.mp4");
    let right = dir.path().join("right.mp4");
    solid_colour(&left, "red", FPS);
    solid_colour(&right, "blue", FPS);
    let sound = dir.path().join("silence.wav");
    write_silence(&sound);
    let out = dir.path().join("dcp3d");

    dcpwizard(config_home.path())
        .args([
            "create",
            "--title",
            "ThreeD",
            "--video",
            left.to_str().unwrap(),
            "--right-eye",
            right.to_str().unwrap(),
            "--audio",
            sound.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--container",
            "2k-flat",
        ])
        .assert()
        .success();

    let cpl = std::fs::read_to_string(only_file_starting_with(&out, "CPL_")).unwrap();
    assert!(
        cpl.contains(":MainStereoscopicPicture") && cpl.contains(STEREOSCOPIC_PICTURE_NAMESPACE),
        "a 3D reel declares its picture as ST 429-10 MainStereoscopicPicture: {cpl}"
    );
    assert!(
        !cpl.contains("<MainPicture>"),
        "a 3D reel carries no 2D MainPicture beside it: {cpl}"
    );

    // one track file holds both eyes, so it opens as a stereoscopic essence
    let picture_mxf = only_file_starting_with(&out, "picture_");
    let mut reader = asdcplib::jp2k::StereoMxfReader::new();
    reader
        .open_read(&picture_mxf.to_string_lossy())
        .expect("the picture MXF has to open as a stereoscopic essence");
    let descriptor = reader
        .picture_descriptor()
        .expect("the picture descriptor has to read");
    assert_eq!(
        descriptor.container_duration,
        frames_at(FPS),
        "one edit unit per source frame, both eyes inside it"
    );
    assert_eq!(
        (
            descriptor.sample_rate.numerator,
            descriptor.edit_rate.numerator
        ),
        (2 * FPS as i32, FPS as i32),
        "a stereoscopic essence samples two frames per edit unit"
    );

    let mut eye = |phase| {
        let mut buffer = vec![0u8; CODESTREAM_BUFFER_BYTES];
        let read = reader
            .read_frame(0, phase, &mut buffer, None, None)
            .expect("the eye's first codestream has to read");
        buffer.truncate(read);
        centre_pixel(buffer)
    };
    let left_eye = eye(StereoscopicPhase::Left);
    let right_eye = eye(StereoscopicPhase::Right);

    assert!(
        left_eye[0] > left_eye[2],
        "the left eye is the red source, so X' outweighs Z': {left_eye:?}"
    );
    assert!(
        right_eye[2] > right_eye[0],
        "the right eye is the blue source, so Z' outweighs X': {right_eye:?}"
    );

    verify_reports_no_error(&out);
}
