use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("dcpwizard").unwrap()
}

#[test]
fn version_flag() {
    cmd().arg("--version").assert().success().stdout(
        predicate::str::contains("dcpwizard")
            .and(predicate::str::contains(env!("CARGO_PKG_VERSION"))),
    );
}

#[test]
fn help_flag() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("verify"))
        .stdout(predicate::str::contains("kdm"));
}

#[test]
fn verify_missing_directory() {
    let dir = TempDir::new().unwrap();
    let nonexistent = dir.path().join("does_not_exist");

    cmd()
        .args(["verify", nonexistent.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn verify_empty_directory() {
    let dir = TempDir::new().unwrap();

    cmd()
        .args(["verify", dir.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn verify_with_output_report() {
    let dir = TempDir::new().unwrap();
    let report = dir.path().join("report.txt");

    cmd()
        .args([
            "verify",
            dir.path().to_str().unwrap(),
            "--output",
            report.to_str().unwrap(),
        ])
        .assert()
        .failure(); // Fails because dir is empty, but exercises the output path
}

#[test]
fn create_missing_video() {
    let dir = TempDir::new().unwrap();
    let output = dir.path().join("output_dcp");

    cmd()
        .args([
            "create",
            "--title",
            "Test DCP",
            "--video",
            "/nonexistent/video.mxf",
            "--output",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure();
}

#[test]
fn create_help() {
    cmd()
        .args(["create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--title"))
        .stdout(predicate::str::contains("--video"))
        .stdout(predicate::str::contains("--output"))
        .stdout(predicate::str::contains("--content-type"))
        .stdout(predicate::str::contains("--frame-rate"))
        .stdout(predicate::str::contains("--twok"))
        .stdout(predicate::str::contains("--fourk"));
}

#[test]
fn kdm_help() {
    cmd()
        .args(["kdm", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--cert"))
        .stdout(predicate::str::contains("--signer-cert"))
        .stdout(predicate::str::contains("--signer-key"))
        .stdout(predicate::str::contains("--valid-from"))
        .stdout(predicate::str::contains("--valid-to"));
}

#[test]
fn verify_help() {
    cmd()
        .args(["verify", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--no-hash-check"))
        .stdout(predicate::str::contains("--no-picture-check"))
        .stdout(predicate::str::contains("--strict"))
        .stdout(predicate::str::contains("--output"))
        .stdout(predicate::str::contains("--quiet"));
}

#[test]
fn kdm_missing_inputs() {
    cmd()
        .args([
            "kdm",
            "--cert",
            "/nonexistent/cert.pem",
            "--signer-cert",
            "/nonexistent/signer.pem",
            "--signer-key",
            "/nonexistent/signer.key",
            "--cpl-id",
            "urn:uuid:00000000-0000-0000-0000-000000000000",
            "--content-title",
            "Test",
            "--output",
            "/tmp/test.kdm.xml",
        ])
        .assert()
        .failure();
}

// ── W5 audio subcommands ────────────────────────────────────────────────────

fn write_wav(path: &std::path::Path, channels: u16, frames: &[i32]) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: 48000,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for &s in frames {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
}

#[test]
fn crossfade_joins_two_wavs() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    let out = dir.path().join("joined.wav");
    let fs = 1i32 << 22;
    write_wav(&a, 1, &vec![fs; 48000]); // 1s mono
    write_wav(&b, 1, &vec![fs / 2; 48000]);

    cmd()
        .args([
            "crossfade",
            "--a",
            a.to_str().unwrap(),
            "--b",
            b.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--overlap",
            "0.5",
        ])
        .assert()
        .success();

    // output = a + b - overlap = 48000 + 48000 - 24000 frames.
    let r = hound::WavReader::open(&out).unwrap();
    assert_eq!(r.duration(), 72000);
}

#[test]
fn mid_side_decode_writes_lr() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("ms.wav");
    let out = dir.path().join("lr.wav");
    // interleaved 2ch: M=0.5fs, S=0.25fs -> L=0.75fs, R=0.25fs.
    let fs = (1i64 << 23) as f32;
    let m = (0.5 * fs) as i32;
    let s = (0.25 * fs) as i32;
    let frames: Vec<i32> = (0..100).flat_map(|_| [m, s]).collect();
    write_wav(&src, 2, &frames);

    cmd()
        .args([
            "mid-side-decode",
            "-i",
            src.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--mid",
            "0",
            "--side",
            "1",
        ])
        .assert()
        .success();

    let mut r = hound::WavReader::open(&out).unwrap();
    let samples: Vec<i32> = r.samples::<i32>().map(|x| x.unwrap()).collect();
    let l = samples[0] as f32 / fs;
    let rr = samples[1] as f32 / fs;
    assert!((l - 0.75).abs() < 1e-3, "L was {l}");
    assert!((rr - 0.25).abs() < 1e-3, "R was {rr}");
}

#[test]
fn create_signer_cert_requires_a_key() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args([
            "create",
            "--title",
            "T",
            "--video",
            dir.path().to_str().unwrap(),
            "-o",
            dir.path().join("out").to_str().unwrap(),
            "--signer-cert",
            "signer.pem",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--signer-key"));
}

#[test]
fn create_rejects_a_key_that_does_not_match_the_certificate() {
    let dir = TempDir::new().unwrap();
    let certs = dir.path().join("certs");
    std::fs::create_dir_all(&certs).unwrap();
    assert_eq!(postkit::certificate::generate_chain("Acme", &certs), 0);
    let out = dir.path().join("out");

    cmd()
        .args([
            "create",
            "--title",
            "T",
            "--video",
            dir.path().to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--signer-cert",
            certs.join("signer.pem").to_str().unwrap(),
            "--signer-key",
            certs.join("root.key").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("does not match"));
    assert!(!out.exists(), "a bad signer must stop before any output");
}
