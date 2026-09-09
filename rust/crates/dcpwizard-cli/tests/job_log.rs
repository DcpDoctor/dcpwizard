use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAME_RATE: u32 = 24;
const FRAMES: u32 = 3;
const LOG_NAME: &str = "dcpwizard.log";
const DEVICE_WARNING: &str =
    "[ENCODE] WARNING: the GPU was requested and no frame ran on the device";

fn dcpwizard(config_home: &Path) -> Command {
    let mut command = Command::cargo_bin("dcpwizard").unwrap();
    command.env("XDG_CONFIG_HOME", config_home);
    // no accelerator plugin loads, so every run here encodes on the CPU
    command.env("GRK_NO_PLUGIN", "1");
    command
}

fn write_source(directory: &Path) -> PathBuf {
    let path = directory.join("source.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=size={WIDTH}x{HEIGHT}:rate={FRAME_RATE}"),
            "-frames:v",
            &FRAMES.to_string(),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .output()
        .expect("ffmpeg has to run");
    assert!(
        made.status.success(),
        "ffmpeg could not write the source: {}",
        String::from_utf8_lossy(&made.stderr)
    );
    path
}

fn create_and_read_log(source: &Path, out: &Path, config_home: &Path) -> String {
    dcpwizard(config_home)
        .args([
            "create",
            "--title",
            "Job Log",
            "--video",
            source.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--twok",
        ])
        .assert()
        .success();
    std::fs::read_to_string(out.join(LOG_NAME))
        .unwrap_or_else(|e| panic!("the job log has to sit beside the package: {e}"))
}

#[test]
fn a_cpu_create_logs_the_accelerator_off_and_no_frames_on_the_device() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let source = write_source(directory.path());
    let out = directory.path().join("dcp");

    let log = create_and_read_log(&source, &out, config_home.path());
    assert!(
        log.contains("Accelerator: off"),
        "a run that asked for no device says so: {log}"
    );
    assert!(
        log.contains(&format!("[ENCODE] Frames on the device: 0 of {FRAMES}")),
        "the encode has to report the count against the frames it encoded: {log}"
    );
    assert!(
        !log.contains(DEVICE_WARNING),
        "nothing asked for the device, so nothing is warned about: {log}"
    );
}

#[test]
fn a_create_under_the_gpu_preference_logs_why_the_device_never_started() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    dcpwizard(config_home.path())
        .args(["preferences", "set", "gpu", "true"])
        .assert()
        .success();

    let source = write_source(directory.path());
    let out = directory.path().join("dcp");
    let log = create_and_read_log(&source, &out, config_home.path());

    let accelerator = log
        .lines()
        .find(|line| line.starts_with("Accelerator: "))
        .unwrap_or_else(|| panic!("the header has to name the accelerator: {log}"));
    assert!(
        accelerator.starts_with("Accelerator: requested, inactive: "),
        "the preference asked for the device and no plugin loaded: {accelerator}"
    );
    assert!(
        log.contains(&format!("[ENCODE] Frames on the device: 0 of {FRAMES}")),
        "a CPU run under the preference still counts the frames: {log}"
    );
    assert!(
        log.contains(DEVICE_WARNING),
        "the device was asked for and took no frame: {log}"
    );
}
