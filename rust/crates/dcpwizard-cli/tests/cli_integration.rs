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

// ── create raster fitting ───────────────────────────────────────────────────

/// A few frames of colour bars at `width`x`height`, 24 fps, as an mp4.
fn write_test_video(path: &std::path::Path, width: u32, height: u32) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-f", "lavfi", "-i"])
        .arg(format!(
            "testsrc=size={width}x{height}:rate=24:duration=0.25"
        ))
        .args(["-pix_fmt", "yuv420p"])
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("ffmpeg must be installed to build the test source");
    assert!(status.success(), "ffmpeg failed to write the test source");
}

#[test]
fn create_fits_a_source_that_is_not_the_forced_raster_onto_it() {
    let dir = TempDir::new().unwrap();
    // a flat master: 1998 wide, so --twok has to pad it out to 2048
    let video = dir.path().join("flat.mp4");
    write_test_video(&video, 1998, 1080);
    let out = dir.path().join("out");

    cmd()
        .args([
            "create",
            "--title",
            "T",
            "--video",
            video.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--twok",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "scale to 1998x1080, pad to 2048x1080 at (25,0)",
        ));
    assert!(
        std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("CPL_")),
        "a fitted source must produce a package"
    );
}

#[test]
fn create_encodes_a_source_that_already_is_the_forced_container_raster() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("full.mp4");
    write_test_video(&video, 2048, 1080);
    let out = dir.path().join("out");

    cmd()
        .args([
            "create",
            "--title",
            "T",
            "--video",
            video.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--twok",
        ])
        .assert()
        .success();
    assert!(
        std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("CPL_")),
        "a matching source must still produce a package"
    );
}

// ── create picture processing and audio map flags ───────────────────────────

/// A `create` that fails before any encoding, so only the refusal is exercised.
fn create_with(dir: &TempDir, video: &std::path::Path, extra: &[&str]) -> assert_cmd::Command {
    let mut command = cmd();
    command.args([
        "create",
        "--title",
        "T",
        "--video",
        video.to_str().unwrap(),
        "-o",
        dir.path().join("out").to_str().unwrap(),
    ]);
    command.args(extra);
    command
}

#[test]
fn create_lists_every_picture_processing_flag() {
    cmd()
        .args(["create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--crop-left"))
        .stdout(predicate::str::contains("--crop-right"))
        .stdout(predicate::str::contains("--crop-top"))
        .stdout(predicate::str::contains("--crop-bottom"))
        .stdout(predicate::str::contains("--auto-crop"))
        .stdout(predicate::str::contains("--auto-crop-threshold"))
        .stdout(predicate::str::contains("--fill-crop"))
        .stdout(predicate::str::contains("--deinterlace"))
        .stdout(predicate::str::contains("--denoise"))
        .stdout(predicate::str::contains("--rotate"))
        .stdout(predicate::str::contains("--flip"))
        .stdout(predicate::str::contains("--audio-map"))
        .stdout(predicate::str::contains("before any rotation"));
}

#[test]
fn create_refuses_two_ways_of_choosing_a_crop() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);

    create_with(&dir, &video, &["--twok", "--fill-crop", "--auto-crop"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("one or the other"));
    create_with(
        &dir,
        &video,
        &["--twok", "--fill-crop", "--crop-left", "10"],
    )
    .assert()
    .failure()
    .stdout(predicate::str::contains("one or the other"));
}

#[test]
fn create_refuses_a_fill_crop_with_no_aspect_to_fill() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);

    create_with(&dir, &video, &["--fill-crop"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("--container"));
}

#[test]
fn create_refuses_picture_processing_on_a_codestream_directory() {
    let dir = TempDir::new().unwrap();
    let j2k = dir.path().join("j2k");
    std::fs::create_dir_all(&j2k).unwrap();

    create_with(&dir, &j2k, &["--deinterlace"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("already compressed"));
}

#[test]
fn create_refuses_an_audio_map_beside_another_way_of_placing_channels() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);
    let audio = dir.path().join("stereo.wav");
    write_wav(&audio, 2, &[0; 96]);

    for extra in [
        vec!["--upmix", "a"],
        vec!["--audio-input-order", "lrc-ls-rs-lfe"],
    ] {
        let mut args = vec!["--audio", audio.to_str().unwrap(), "--audio-map", "1:L,2:R"];
        args.extend(extra);
        create_with(&dir, &video, &args)
            .assert()
            .failure()
            .stdout(predicate::str::contains("one or the other"));
    }
}

#[test]
fn create_refuses_an_audio_map_that_names_an_unknown_lane() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);
    let audio = dir.path().join("stereo.wav");
    write_wav(&audio, 2, &[0; 96]);

    create_with(
        &dir,
        &video,
        &[
            "--audio",
            audio.to_str().unwrap(),
            "--audio-map",
            "1:Surround",
        ],
    )
    .assert()
    .failure()
    .stdout(predicate::str::contains("Surround"));
}
