use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAME_RATE: u32 = 24;
const FRAMES: u32 = 3;

const PASSED_LINE: &str = "DCP verification PASSED";
const SKIP_LINE: &str = "--no-verify: the finished package was not verified";

fn dcpwizard(config_home: &Path) -> Command {
    let mut command = Command::cargo_bin("dcpwizard").unwrap();
    command.env("XDG_CONFIG_HOME", config_home);
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

fn everything_printed(run: &std::process::Output) -> String {
    String::from_utf8_lossy(&run.stdout).into_owned() + &String::from_utf8_lossy(&run.stderr)
}

fn create(source: &Path, out: &Path, config_home: &Path, extra: &[&str]) -> std::process::Output {
    let mut command = dcpwizard(config_home);
    command.args([
        "create",
        "--title",
        "Verified",
        "--video",
        source.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--twok",
    ]);
    command.args(extra).output().expect("dcpwizard has to run")
}

#[test]
fn a_create_verifies_the_package_it_wrote() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let source = write_source(directory.path());
    let out = directory.path().join("dcp");

    let run = create(&source, &out, config_home.path(), &[]);
    let printed = everything_printed(&run);
    assert!(
        run.status.success(),
        "a package that verifies exits 0: {printed}"
    );
    assert!(
        printed.contains(PASSED_LINE),
        "create has to end with the verification the verify command prints: {printed}"
    );
    assert!(!printed.contains(SKIP_LINE), "{printed}");
    // the verification read a real package, not an empty directory
    assert!(out.join("ASSETMAP.xml").exists() || out.join("ASSETMAP").exists());
}

#[test]
fn no_verify_says_the_package_went_out_unread() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let source = write_source(directory.path());
    let out = directory.path().join("dcp");

    let run = create(&source, &out, config_home.path(), &["--no-verify"]);
    assert!(run.status.success());
    assert!(
        String::from_utf8_lossy(&run.stderr).contains(SKIP_LINE),
        "the skip has to be said out loud: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let printed = everything_printed(&run);
    assert!(
        !printed.contains(PASSED_LINE),
        "nothing was verified, so nothing passed: {printed}"
    );
}
