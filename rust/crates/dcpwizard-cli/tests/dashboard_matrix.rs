use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAME_RATE: u32 = 24;
const FRAMES: u32 = 3;

// postkit writes a tick for a title released in a territory and an empty cell otherwise
const EXPECTED_MATRIX: &str = "Territory,Aurora,Borealis\nFR,✓,\nGB,,✓\n";

struct Version {
    title: &'static str,
    territory: &'static str,
}

const VERSIONS: [Version; 2] = [
    Version {
        title: "Aurora",
        territory: "FR",
    },
    Version {
        title: "Borealis",
        territory: "GB",
    },
];

// the dashboard database sits beside the user's config, so the whole home moves
fn dcpwizard(config_home: &Path) -> Command {
    let mut command = Command::cargo_bin("dcpwizard").unwrap();
    command.env("XDG_CONFIG_HOME", config_home);
    command.env("HOME", config_home);
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

fn composition_id(package: &Path) -> String {
    let cpl = std::fs::read_dir(package)
        .expect("the package directory has to be readable")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("CPL_"))
        })
        .expect("a CPL in the package");
    let text = std::fs::read_to_string(cpl).unwrap();
    text.split_once("<Id>urn:uuid:")
        .and_then(|(_, rest)| rest.split_once('<'))
        .map(|(id, _)| id.to_string())
        .expect("the CPL names a composition id")
}

fn build_package(source: &Path, out: &Path, config_home: &Path, title: &str) {
    dcpwizard(config_home)
        .args([
            "create",
            "--title",
            title,
            "--video",
            source.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--twok",
        ])
        .assert()
        .success();
}

#[test]
fn the_matrix_names_every_registered_package_in_its_territory() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let source = write_source(directory.path());

    for version in &VERSIONS {
        let package = directory.path().join(version.title);
        build_package(&source, &package, config_home.path(), version.title);
        dcpwizard(config_home.path())
            .args([
                "dashboard",
                "register",
                "--uuid",
                &composition_id(&package),
                "--title",
                version.title,
                "--territory",
                version.territory,
                "--dcp-path",
                package.to_str().unwrap(),
                "--status",
                "released",
            ])
            .assert()
            .success();
    }

    let matrix = directory.path().join("matrix.csv");
    dcpwizard(config_home.path())
        .args(["dashboard", "matrix", "-o", matrix.to_str().unwrap()])
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(&matrix).expect("the matrix CSV has to be written"),
        EXPECTED_MATRIX
    );
}
