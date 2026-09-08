use assert_cmd::Command;
use predicates::prelude::*;
use std::sync::OnceLock;
use tempfile::TempDir;

fn cmd() -> Command {
    static CONFIG_DIRECTORY: OnceLock<TempDir> = OnceLock::new();
    let directory = CONFIG_DIRECTORY.get_or_init(|| TempDir::new().unwrap());
    let mut command = Command::cargo_bin("dcpwizard").unwrap();
    command.env("XDG_CONFIG_HOME", directory.path());
    command
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
        .stdout(predicate::str::contains("kdm"))
        .stdout(predicate::str::contains("preferences"))
        .stdout(predicate::str::contains("--license"))
        .stdout(predicate::str::contains("--registration-url"));
}

#[test]
fn a_registration_url_requires_a_license() {
    let directory = TempDir::new().unwrap();
    cmd()
        .env("XDG_CONFIG_HOME", directory.path())
        .args([
            "--gpu",
            "--registration-url",
            "https://example.com/register",
            "verify",
            ".",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--license"));
}

#[test]
fn the_gpu_preference_stays_on_the_cpu_when_no_plugin_loads() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("does_not_exist");
    cmd()
        .env("XDG_CONFIG_HOME", directory.path())
        .args(["preferences", "set", "gpu", "true"])
        .assert()
        .success();

    cmd()
        .env("XDG_CONFIG_HOME", directory.path())
        .env("GRK_NO_PLUGIN", "1")
        .args(["verify", missing.to_str().unwrap()])
        .assert()
        .failure()
        .stdout(predicate::str::contains("stays on the CPU"));

    cmd()
        .env("XDG_CONFIG_HOME", directory.path())
        .env("GRK_NO_PLUGIN", "1")
        .args(["--gpu", "verify", missing.to_str().unwrap()])
        .assert()
        .failure()
        .stdout(predicate::str::contains("did not initialise"))
        .stdout(predicate::str::contains("stays on the CPU").not());
}

#[test]
fn preferences_are_shared_between_invocations() {
    let directory = TempDir::new().unwrap();

    cmd()
        .env("XDG_CONFIG_HOME", directory.path())
        .args(["preferences", "set", "gpu-license", "test-license"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"gpuLicense\": \"test-license\""));

    cmd()
        .env("XDG_CONFIG_HOME", directory.path())
        .args(["preferences", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"gpuLicense\": \"test-license\""))
        .stdout(predicate::str::contains("\"version\": 2"));
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
fn an_unknown_accessibility_standard_is_refused_by_name() {
    let dir = TempDir::new().unwrap();

    cmd()
        .args([
            "accessibility",
            "--standard",
            "nonsense",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid value 'nonsense'")
                .and(predicate::str::contains("cvaa"))
                .and(predicate::str::contains("eaa"))
                .and(predicate::str::contains("aoda"))
                .and(predicate::str::contains("ofcom")),
        );
}

/// A DCP carrying a closed-caption asset, an HI mix channel and a VI-N
/// narration channel. Built once and shared, since a create runs ffmpeg and the
/// JPEG 2000 encoder.
fn dcp_with_the_accessibility_tracks() -> std::path::PathBuf {
    static BUILD: OnceLock<TempDir> = OnceLock::new();
    let build = BUILD.get_or_init(|| {
        let dir = TempDir::new().unwrap();
        let video = dir.path().join("source.mp4");
        write_test_video(&video, 2048, 1080);
        let audio = dir.path().join("source.wav");
        write_wav(&audio, 8, &vec![0; 8 * 12_000]);
        let captions = dir.path().join("captions.srt");
        write_subtitle(&captions, "00:00:00,000", "00:00:00,200", "hello");

        cmd()
            .args([
                "create",
                "--title",
                "T",
                "--video",
                video.to_str().unwrap(),
                "--audio",
                audio.to_str().unwrap(),
                "--ccap",
                captions.to_str().unwrap(),
                // 0-based, so the HI and VI-N channels land after the 5.1 bed
                "--hi-channel",
                "6",
                "--vi-channel",
                "7",
                "-o",
                dir.path().join("dcp").to_str().unwrap(),
            ])
            .assert()
            .success();
        dir
    });
    build.path().join("dcp")
}

/// The same picture with no sound and no captions, so every accessibility track
/// the probe reads off a composition is missing.
fn dcp_without_the_accessibility_tracks() -> std::path::PathBuf {
    static BUILD: OnceLock<TempDir> = OnceLock::new();
    let build = BUILD.get_or_init(|| {
        let dir = TempDir::new().unwrap();
        let video = dir.path().join("source.mp4");
        write_test_video(&video, 2048, 1080);
        cmd()
            .args([
                "create",
                "--title",
                "T",
                "--video",
                video.to_str().unwrap(),
                "-o",
                dir.path().join("dcp").to_str().unwrap(),
            ])
            .assert()
            .success();
        dir
    });
    build.path().join("dcp")
}

fn accessibility_check(standard: &str, dcp: &std::path::Path) -> assert_cmd::Command {
    let mut command = cmd();
    command.args([
        "accessibility",
        "--standard",
        standard,
        dcp.to_str().unwrap(),
    ]);
    command
}

#[test]
fn eaa_wants_captions_and_narration_on_a_real_dcp() {
    accessibility_check("eaa", &dcp_with_the_accessibility_tracks())
        .assert()
        .success()
        .stdout(predicate::str::contains("Standard:  Eaa"))
        .stdout(predicate::str::contains("Compliant: true"));

    accessibility_check("eaa", &dcp_without_the_accessibility_tracks())
        .assert()
        .failure()
        .stdout(predicate::str::contains("Compliant: false"))
        .stdout(predicate::str::contains(
            "[Error] EAA-CC-1 (ClosedCaptions): Closed caption asset required by EAA",
        ))
        .stdout(predicate::str::contains(
            "[Error] EAA-AD-1 (AudioDescription): Audio description channel required by EAA",
        ))
        .stdout(predicate::str::contains(
            "[Warning] EAA-HI-1 (HearingImpaired): Hearing impaired channel recommended by EAA",
        ));
}

#[test]
fn aoda_wants_captions_on_a_real_dcp() {
    accessibility_check("aoda", &dcp_with_the_accessibility_tracks())
        .assert()
        .success()
        .stdout(predicate::str::contains("Standard:  Aoda"))
        .stdout(predicate::str::contains("Compliant: true"));

    accessibility_check("aoda", &dcp_without_the_accessibility_tracks())
        .assert()
        .failure()
        .stdout(predicate::str::contains("Compliant: false"))
        .stdout(predicate::str::contains(
            "[Error] AODA-CC-1 (ClosedCaptions): Closed caption asset required by AODA",
        ))
        .stdout(predicate::str::contains(
            "[Warning] AODA-AD-1 (AudioDescription): Audio description channel recommended by AODA",
        ));
}

#[test]
fn ofcom_wants_captions_and_narration_on_a_real_dcp() {
    accessibility_check("ofcom", &dcp_with_the_accessibility_tracks())
        .assert()
        .success()
        .stdout(predicate::str::contains("Standard:  Ofcom"))
        .stdout(predicate::str::contains("Compliant: true"))
        .stdout(predicate::str::contains(
            "[Warning] OFCOM-SL-1 (SignLanguage): Sign language video recommended by OFCOM",
        ));

    accessibility_check("ofcom", &dcp_without_the_accessibility_tracks())
        .assert()
        .failure()
        .stdout(predicate::str::contains("Compliant: false"))
        .stdout(predicate::str::contains(
            "[Error] OFCOM-CC-1 (ClosedCaptions): Closed caption asset required by OFCOM",
        ))
        .stdout(predicate::str::contains(
            "[Error] OFCOM-AD-1 (AudioDescription): Audio description channel required by OFCOM",
        ));
}

#[test]
fn dv_inject_names_the_input_it_cannot_find() {
    let dir = TempDir::new().unwrap();
    let missing_input = dir.path().join("missing_input.hevc");

    cmd()
        .args([
            "dv-inject",
            "--input",
            missing_input.to_str().unwrap(),
            "--rpu",
            dir.path().join("rpu.bin").to_str().unwrap(),
            "--output",
            dir.path().join("out.hevc").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains(missing_input.to_str().unwrap()));
}

#[test]
fn dv_inject_names_the_rpu_it_cannot_find() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.hevc");
    std::fs::write(&input, b"not really hevc").unwrap();
    let missing_rpu = dir.path().join("missing_rpu.bin");

    cmd()
        .args([
            "dv-inject",
            "--input",
            input.to_str().unwrap(),
            "--rpu",
            missing_rpu.to_str().unwrap(),
            "--output",
            dir.path().join("out.hevc").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains(missing_rpu.to_str().unwrap())
                .and(predicate::str::contains("RPU file not found")),
        );
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

/// Three frames of colour bars as 8-bit PNG stills, a format postkit hands to
/// ffmpeg rather than reading itself.
fn write_test_stills(dir: &std::path::Path, width: u32, height: u32) {
    std::fs::create_dir_all(dir).unwrap();
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-f", "lavfi", "-i"])
        .arg(format!(
            "testsrc=size={width}x{height}:rate=24:duration=0.125"
        ))
        .arg(dir.join("frame_%03d.png"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("ffmpeg must be installed to build the test stills");
    assert!(status.success(), "ffmpeg failed to write the test stills");
}

#[test]
fn encode_compresses_a_still_sequence_into_a_j2k_directory() {
    let dir = TempDir::new().unwrap();
    let stills = dir.path().join("stills");
    write_test_stills(&stills, 2048, 1080);
    let out = dir.path().join("out");

    cmd()
        .args([
            "encode",
            "--input",
            stills.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--bandwidth",
            "250",
        ])
        .assert()
        .success();

    let mut codestreams: Vec<_> = std::fs::read_dir(out.join("j2k"))
        .expect("the codestreams land in <output>/j2k")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "j2c"))
        .collect();
    codestreams.sort();
    assert_eq!(codestreams.len(), 3, "one codestream per still");
    let decoded = postkit::grok_decoder::decode(std::fs::read(&codestreams[0]).unwrap(), 0)
        .expect("the first codestream decodes");
    assert_eq!((decoded.width, decoded.height), (2048, 1080));
    assert_eq!(
        decoded.precision, 12,
        "a DCI codestream carries 12-bit samples"
    );
    assert_eq!(decoded.components.len(), 3);
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
            "scale to 1998x1080, pad to 2048x1080 at (24,0)",
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

/// The scratch a build writes belongs to the build, not to the package, and a
/// package that was never written keeps it for `--resume`. The ASSETMAP is the
/// last thing `create_dcp` writes and its name is fixed, so a directory in its
/// place fails the package after every frame has been encoded.
#[test]
fn a_finished_dcp_holds_no_scratch_and_a_failed_one_keeps_it_for_a_resume() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("full.mp4");
    write_test_video(&video, 2048, 1080);

    let failed = dir.path().join("failed");
    std::fs::create_dir_all(failed.join("ASSETMAP.xml")).unwrap();
    cmd()
        .args([
            "create",
            "--title",
            "T",
            "--video",
            video.to_str().unwrap(),
            "-o",
            failed.to_str().unwrap(),
            "--twok",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("--resume reuses them"));
    assert!(
        dcpwizard_core::trim::frame_count(&failed.join("j2k")) > 0,
        "a failed package must keep the frames it encoded"
    );
    assert!(
        failed.join(".dcpwizard-encode.json").exists(),
        "a failed package must keep the state --resume checks"
    );

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
        !out.join("j2k").exists(),
        "the codestreams shipped in the DCP"
    );
    assert!(
        !out.join(".dcpwizard-encode.json").exists(),
        "the resume state shipped in the DCP"
    );
    assert!(
        std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("CPL_")),
        "a clean run must still produce a package"
    );
}

#[test]
fn create_writes_the_picture_mxf_while_it_encodes() {
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
        .success()
        .stdout(predicate::str::contains(
            "6 frames, written during the encode",
        ));
}

/// A bandwidth this far over DCI's 250 Mbit/s asks for more bytes a frame than
/// a DCP may carry.
const BANDWIDTH_OVER_THE_DCI_CAP_MBPS: u32 = 300;
/// The bytes a frame that bandwidth asks for at 24 fps.
const TARGET_OVER_THE_DCI_CAP: u64 = 1_562_500;
/// DCI DCSS 4.3.1 at 24 fps.
const DCI_CODESTREAM_BYTE_CAP: u64 = 1_302_083;
const NOISE_FRAMES: u32 = 3;

/// Noise from a fixed seed, which no wavelet compresses, so every codestream
/// runs to whatever budget the encoder is given.
fn write_noise_video(path: &std::path::Path, width: u32, height: u32, frames: u32) {
    let raw = path.with_extension("raw");
    let mut samples = Vec::with_capacity((width * height * 3 * frames) as usize);
    let mut state: u32 = 0x1234_5678;
    for _ in 0..width * height * 3 * frames {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        samples.push(state as u8);
    }
    std::fs::write(&raw, &samples).unwrap();
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo"])
        .args(["-pixel_format", "rgb24", "-video_size"])
        .arg(format!("{width}x{height}"))
        .args(["-framerate", "24", "-i"])
        .arg(&raw)
        .args(["-c:v", "ffv1"])
        .arg(path)
        .status()
        .expect("ffmpeg must be installed to build the test source");
    assert!(status.success(), "ffmpeg failed to write the noise source");
    std::fs::remove_file(&raw).unwrap();
}

#[test]
fn create_holds_the_dci_cap_when_the_bandwidth_asks_for_more() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("noise.mkv");
    write_noise_video(&video, 2048, 1080, NOISE_FRAMES);
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
            "--video-bit-rate",
            &BANDWIDTH_OVER_THE_DCI_CAP_MBPS.to_string(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Target: {TARGET_OVER_THE_DCI_CAP} bytes a frame \
             ({BANDWIDTH_OVER_THE_DCI_CAP_MBPS} Mbit/s), cap {DCI_CODESTREAM_BYTE_CAP} bytes"
        )));

    let picture = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("picture_"))
        })
        .expect("the package carries a picture MXF");
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&picture.to_string_lossy())
        .expect("open the picture MXF");
    let mut buf = vec![0u8; 16 * 1024 * 1024];
    for frame in 0..NOISE_FRAMES {
        let bytes = reader
            .read_frame(frame, &mut buf, None, None)
            .unwrap_or_else(|e| panic!("read frame {frame}: {e}")) as u64;
        assert!(
            bytes <= DCI_CODESTREAM_BYTE_CAP,
            "frame {frame} is {bytes} bytes, over the {DCI_CODESTREAM_BYTE_CAP} byte DCI cap"
        );
    }
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
        .stdout(predicate::str::contains("give only one of them"));
    create_with(
        &dir,
        &video,
        &["--twok", "--fill-crop", "--crop-left", "10"],
    )
    .assert()
    .failure()
    .stdout(predicate::str::contains("give only one of them"));
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

#[test]
fn create_refuses_a_rating_without_an_agency_separator() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);

    create_with(&dir, &video, &["--rating", "PG-13"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("AGENCY=LABEL"));
}

#[test]
fn create_refuses_an_isdcf_date_that_is_not_a_date() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);

    create_with(&dir, &video, &["--isdcf-date", "16-08-2026"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("YYYY-MM-DD"));
}

#[test]
fn create_refuses_an_unknown_territory_type() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);

    create_with(&dir, &video, &["--territory-type", "worldwide"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("territory-type"));
}

/// A two-cue SRT, the shape both `--subtitle` and `--burn-subtitle` take.
fn write_test_srt(path: &std::path::Path) {
    std::fs::write(
        path,
        "1\n00:00:00,000 --> 00:00:01,000\nfirst\n\n2\n00:00:02,000 --> 00:00:03,000\nsecond\n\n",
    )
    .unwrap();
}

#[test]
fn create_refuses_a_burn_colour_that_is_not_a_colour() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);
    let srt = dir.path().join("cues.srt");
    write_test_srt(&srt);

    create_with(
        &dir,
        &video,
        &[
            "--burn-subtitle",
            srt.to_str().unwrap(),
            "--burn-colour",
            "banana",
        ],
    )
    .assert()
    .failure()
    .stdout(predicate::str::contains("--burn-colour"));
}

#[test]
fn create_refuses_a_burn_appearance_with_nothing_to_burn() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);

    create_with(&dir, &video, &["--burn-effect", "outline"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("--burn-effect"))
        .stdout(predicate::str::contains("--burn-subtitle"));

    create_with(&dir, &video, &["--burn-margin", "12"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("--burn-margin"))
        .stdout(predicate::str::contains("--burn-subtitle"));
}

#[test]
fn create_refuses_a_burn_line_height_below_one() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);
    let srt = dir.path().join("cues.srt");
    write_test_srt(&srt);

    create_with(
        &dir,
        &video,
        &[
            "--burn-subtitle",
            srt.to_str().unwrap(),
            "--burn-line-height",
            "0.5",
        ],
    )
    .assert()
    .failure()
    .stdout(predicate::str::contains("line height"));
}

#[test]
fn create_refuses_a_subtitle_appearance_with_no_subtitle_track() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("hd.mp4");
    write_test_video(&video, 1920, 1080);

    create_with(&dir, &video, &["--subtitle-effect", "none"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("--subtitle-effect"))
        .stdout(predicate::str::contains("--subtitle"));
}

// ── create --check: the pre-build refusals and hints ─────────────────────────

/// One SRT cue, so a test can place the first subtitle where it wants it.
fn write_subtitle(path: &std::path::Path, start: &str, end: &str, text: &str) {
    std::fs::write(path, format!("1\n{start} --> {end}\n{text}\n")).unwrap();
}

/// Codestreams are compressed already, so nothing is fitted onto the container
/// and its active area can be wider than the frames a package would carry.
#[test]
fn check_refuses_a_container_larger_than_the_codestreams_it_would_declare() {
    let dir = TempDir::new().unwrap();
    let stills = dir.path().join("stills");
    write_test_stills(&stills, 1920, 1080);
    let encoded = dir.path().join("encoded");
    cmd()
        .args([
            "encode",
            "--input",
            stills.to_str().unwrap(),
            "--output",
            encoded.to_str().unwrap(),
            "--bandwidth",
            "250",
        ])
        .assert()
        .success();

    create_with(
        &dir,
        &encoded.join("j2k"),
        &["--check", "--container", "2k-full"],
    )
    .assert()
    .failure()
    .stdout(predicate::str::contains(
        "container 2048x1080 is larger than the 1920x1080",
    ));
}

#[test]
fn check_hints_a_first_subtitle_before_four_seconds() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("flat.mp4");
    write_test_video(&video, 1998, 1080);
    let subtitle = dir.path().join("subs.srt");
    write_subtitle(&subtitle, "00:00:01,000", "00:00:05,000", "hello");

    create_with(
        &dir,
        &video,
        &["--check", "--subtitle", subtitle.to_str().unwrap()],
    )
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "The first subtitle in subs.srt starts at 00:00:01.000",
    ));
}

#[test]
fn check_hints_a_frame_rate_not_every_projector_plays() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("flat.mp4");
    write_test_video(&video, 1998, 1080);

    create_with(&dir, &video, &["--check", "--frame-rate", "25"])
        .assert()
        .success()
        .stdout(predicate::str::contains("The DCP is 25 fps"))
        .stdout(predicate::str::contains(
            "24 fps is the rate to fall back to",
        ));
}

#[test]
fn a_clean_check_passes_with_no_hint_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("flat.mp4");
    write_test_video(&video, 1998, 1080);
    let out = dir.path().join("out");

    create_with(&dir, &video, &["--check", "--content-type", "SHR"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Pre-build check passed with 0 hint(s)",
        ))
        .stdout(predicate::str::contains("hint:").not());
    assert!(!out.exists(), "a check must write nothing under --output");
}

#[test]
fn tms_help_lists_the_package_and_the_config() {
    cmd()
        .args(["tms", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--tms-config"))
        .stdout(predicate::str::contains("PACKAGE"));
}

#[test]
fn tms_needs_a_package_directory() {
    cmd()
        .arg("tms")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "required arguments were not provided",
        ));
}

#[test]
fn tms_refuses_a_config_it_cannot_parse() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("tms.toml");
    std::fs::write(&config, "protocol = \"carrier-pigeon\"\n").unwrap();

    cmd()
        .args([
            "tms",
            dir.path().to_str().unwrap(),
            "--tms-config",
            config.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("invalid tms config"));
}

#[test]
fn create_reads_the_tms_config_before_it_encodes() {
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("flat.mp4");
    write_test_video(&video, 1998, 1080);
    let out = dir.path().join("out");

    create_with(
        &dir,
        &video,
        &[
            "--upload-to-tms",
            "--tms-config",
            dir.path().join("absent.toml").to_str().unwrap(),
        ],
    )
    .assert()
    .failure()
    .stdout(predicate::str::contains("cannot read tms config"));
    assert!(!out.exists(), "the encode must not have started");
}
