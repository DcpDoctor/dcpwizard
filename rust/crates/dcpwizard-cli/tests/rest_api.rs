//! The REST API driven over real sockets against a real `dcpwizard daemon`.
//!
//! Every case runs the daemon and `serve` as child processes with a
//! `DCPWIZARD_DAEMON_ADDR` and a `DCPWIZARD_JOBS_FILE` of their own, so the
//! cases do not share the fixed default daemon port and can run in parallel.

use dcpwizard_core::dcp::DcpConfig;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const API_KEY: &str = "correct-horse-battery-staple";
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const JOB_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 24;

/// A port the OS handed out and nothing holds any more. Two harnesses racing for
/// the same number is possible but has not been seen.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn binary() -> PathBuf {
    assert_cmd::cargo::cargo_bin("dcpwizard")
}

struct Harness {
    directory: TempDir,
    daemon: Option<Child>,
    server: Child,
    api: SocketAddr,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
        if let Some(daemon) = &mut self.daemon {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
    }
}

impl Harness {
    fn start(api_key: Option<&str>, with_daemon: bool) -> Self {
        let directory = TempDir::new().unwrap();
        let daemon_address = format!("127.0.0.1:{}", free_port());
        let api = format!("127.0.0.1:{}", free_port())
            .parse::<SocketAddr>()
            .unwrap();

        let child = |arguments: &[&str]| {
            let mut command = Command::new(binary());
            command
                .args(arguments)
                .env("DCPWIZARD_DAEMON_ADDR", &daemon_address)
                .env("DCPWIZARD_JOBS_FILE", directory.path().join("jobs.jsonl"))
                .env("XDG_CONFIG_HOME", directory.path())
                .env("XDG_DATA_HOME", directory.path());
            command.spawn().expect("spawn dcpwizard")
        };

        let daemon = with_daemon.then(|| child(&["daemon"]));
        if daemon.is_some() {
            wait_until(|| TcpStream::connect(&daemon_address).is_ok(), "the daemon");
        }

        let api_address = api.to_string();
        let mut arguments = vec!["serve", "--bind", &api_address];
        if let Some(key) = api_key {
            arguments.push("--api-key");
            arguments.push(key);
        }
        let server = child(&arguments);
        wait_until(|| health_is_ok(api), "the REST API to answer /health");

        Self {
            directory,
            daemon,
            server,
            api,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn get(&self, path: &str, headers: &str) -> String {
        send(
            self.api,
            &format!(
                "GET {path} HTTP/1.1\r\nHost: localhost\r\n{headers}Connection: close\r\n\r\n"
            ),
        )
    }

    fn post(&self, path: &str, body: &str) -> String {
        send(
            self.api,
            &format!(
                "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        )
    }

    /// Poll `GET /jobs` until the job is Completed or Failed, then return its
    /// record.
    fn wait_for_job(&self, id: &str) -> serde_json::Value {
        let deadline = Instant::now() + JOB_TIMEOUT;
        loop {
            let jobs = self.jobs();
            let job = jobs
                .iter()
                .find(|job| job["id"] == id)
                .unwrap_or_else(|| panic!("GET /jobs never listed {id}: {jobs:?}"))
                .clone();
            let state = job["state"].as_str().unwrap_or_default().to_string();
            if state == "Completed" || state == "Failed" {
                return job;
            }
            assert!(
                Instant::now() < deadline,
                "job {id} stayed {state} for {JOB_TIMEOUT:?}"
            );
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn jobs(&self) -> Vec<serde_json::Value> {
        let response = self.get("/jobs", "");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        serde_json::from_str(body_of(&response)).expect("a JSON job list")
    }
}

fn wait_until(mut ready: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + READY_TIMEOUT;
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "waited {READY_TIMEOUT:?} for {what}"
        );
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn health_is_ok(address: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect(address) else {
        return false;
    };
    if stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).is_ok() && response.starts_with("HTTP/1.1 200 OK")
}

fn send(address: SocketAddr, raw: &str) -> String {
    let mut stream = TcpStream::connect(address).expect("connect to the REST API");
    stream.write_all(raw.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn body_of(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("")
}

fn job_id_of(response: &str) -> String {
    let parsed: serde_json::Value =
        serde_json::from_str(body_of(response)).unwrap_or_else(|e| panic!("{e}: {response}"));
    parsed["job_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no job_id in {response}"))
        .to_string()
}

/// 24 black 2K frames and a matching stereo WAV, the smallest package the create
/// path builds.
fn make_source(directory: &Path) -> (PathBuf, PathBuf) {
    let j2k = directory.join("j2k");
    std::fs::create_dir_all(&j2k).unwrap();
    let seed = j2k.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(WIDTH, HEIGHT, FPS, &seed).expect("encode a frame");
    for index in 0..FRAMES {
        std::fs::copy(&seed, j2k.join(format!("frame_{index:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();

    let audio = directory.join("audio.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&audio, spec).unwrap();
    let samples = FRAMES as u32 * (48_000 / FPS);
    for _ in 0..samples * u32::from(spec.channels) {
        writer.write_sample(0i32).unwrap();
    }
    writer.finalize().unwrap();

    (j2k, audio)
}

fn base_config(output: PathBuf, j2k: PathBuf, audio: PathBuf) -> DcpConfig {
    DcpConfig {
        title: "Rest Api".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: output,
        j2k_dir: Some(j2k),
        audio_path: Some(audio),
        ..Default::default()
    }
}

fn has_assetmap(directory: &Path) -> bool {
    ["ASSETMAP", "ASSETMAP.xml"]
        .iter()
        .any(|name| directory.join(name).exists())
}

#[test]
fn the_job_routes_503_and_name_the_daemon_when_it_is_down() {
    let harness = Harness::start(None, false);

    let jobs = harness.get("/jobs", "");
    assert!(jobs.starts_with("HTTP/1.1 503"), "{jobs}");
    assert!(
        jobs.contains("dcpwizard daemon"),
        "the 503 must say what to start: {jobs}"
    );

    let config = base_config(
        harness.path("out"),
        harness.path("j2k"),
        harness.path("audio.wav"),
    );
    let create = harness.post("/create", &serde_json::to_string(&config).unwrap());
    assert!(create.starts_with("HTTP/1.1 503"), "{create}");
    assert!(
        create.contains("dcpwizard daemon"),
        "the 503 must say what to start: {create}"
    );

    // the daemon answers nothing, but the API itself is up
    let status = harness.get("/daemon-status", "");
    assert!(status.starts_with("HTTP/1.1 200 OK"), "{status}");
    assert!(status.contains(r#""daemon_running":false"#), "{status}");
}

#[test]
fn a_key_is_required_on_the_job_routes_and_not_on_health() {
    let harness = Harness::start(Some(API_KEY), true);

    assert!(
        harness.get("/health", "").starts_with("HTTP/1.1 200 OK"),
        "/health is the exempt path"
    );

    let refused = harness.get("/jobs", "");
    assert!(
        refused.starts_with("HTTP/1.1 401"),
        "no key must not reach the daemon: {refused}"
    );

    for header in [
        format!("X-Api-Key: {API_KEY}\r\n"),
        format!("Authorization: Bearer {API_KEY}\r\n"),
    ] {
        let response = harness.get("/jobs", &header);
        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "{header}: {response}"
        );
    }
}

#[test]
fn a_config_over_eight_kibibytes_reaches_the_daemon_whole() {
    let harness = Harness::start(None, true);

    let mut config = base_config(
        harness.path("out"),
        harness.path("j2k"),
        harness.path("audio.wav"),
    );
    config.title = "P".repeat(9000);
    let body = serde_json::to_string(&config).unwrap();
    assert!(body.len() > 8192, "the body must exceed one 8 KiB read");

    let response = harness.post("/create", &body);
    assert!(
        response.starts_with("HTTP/1.1 202"),
        "a body split across reads must not be truncated into a 400: {response}"
    );

    let id = job_id_of(&response);
    let job = harness
        .jobs()
        .into_iter()
        .find(|job| job["id"] == id)
        .expect("the submitted job");
    assert_eq!(
        job["params"].as_str().unwrap().len(),
        body.len(),
        "the daemon must hold the whole config the client sent"
    );
}

#[test]
fn a_posted_config_builds_a_dcp_the_verify_route_then_passes() {
    let harness = Harness::start(None, true);
    let (j2k, audio) = make_source(harness.directory.path());
    let output = harness.path("out");
    let config = base_config(output.clone(), j2k, audio);

    let create = harness.post("/create", &serde_json::to_string(&config).unwrap());
    assert!(create.starts_with("HTTP/1.1 202"), "{create}");
    let create_id = job_id_of(&create);

    let job = harness.wait_for_job(&create_id);
    assert_eq!(
        job["state"], "Completed",
        "the create job failed: {}",
        job["message"]
    );
    assert!(has_assetmap(&output), "no ASSETMAP in {}", output.display());

    let verify = harness.post("/verify", output.to_str().unwrap());
    assert!(verify.starts_with("HTTP/1.1 202"), "{verify}");
    let verify_job = harness.wait_for_job(&job_id_of(&verify));
    assert_eq!(
        verify_job["state"], "Completed",
        "the verify job failed: {}",
        verify_job["message"]
    );

    let metrics = harness.get("/metrics", "");
    assert!(metrics.starts_with("HTTP/1.1 200 OK"), "{metrics}");
    assert!(
        metrics.contains("Content-Type: text/plain; version=0.0.4"),
        "{metrics}"
    );
    assert!(metrics.contains("dcpwizard_jobs_total 2"), "{metrics}");
    assert!(metrics.contains("dcpwizard_jobs_completed 2"), "{metrics}");
    assert!(metrics.contains("dcpwizard_daemon_running 1"), "{metrics}");
}

#[test]
fn serve_answers_health_on_the_address_it_was_given() {
    let bind = format!("127.0.0.1:{}", free_port());
    let directory = TempDir::new().unwrap();
    let mut child = Command::new(binary())
        .args(["serve", "--bind", &bind, "--api-key", "k"])
        .env("XDG_CONFIG_HOME", directory.path())
        .env("XDG_DATA_HOME", directory.path())
        .spawn()
        .expect("spawn dcpwizard serve");

    let address: SocketAddr = bind.parse().unwrap();
    wait_until(|| health_is_ok(address), "serve to answer /health");
    assert!(
        send(
            address,
            "GET /jobs HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .starts_with("HTTP/1.1 401"),
        "--api-key must guard the job routes"
    );

    child.kill().unwrap();
    child.wait().unwrap();
}
