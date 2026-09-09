use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const POLL_INTERVAL_SECONDS: &str = "1";
const BUILD_TIMEOUT: Duration = Duration::from_secs(180);
const FAILURE_TIMEOUT: Duration = Duration::from_secs(60);
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_millis(100);
const GROWING_APPEND_INTERVAL: Duration = Duration::from_millis(300);
const GROWING_APPEND_COUNT: usize = 10;
const GROWING_APPEND_BYTES: usize = 4096;

struct RecordedWebhooks {
    url: String,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl RecordedWebhooks {
    fn wait_for_one(&self, event_type: &str) -> Vec<String> {
        let deadline = Instant::now() + WEBHOOK_TIMEOUT;
        loop {
            let bodies = self.bodies.lock().unwrap().clone();
            let matching: Vec<String> = bodies
                .into_iter()
                .filter(|body| body.contains(event_type))
                .collect();
            if !matching.is_empty() || Instant::now() >= deadline {
                return matching;
            }
            std::thread::sleep(CHECK_INTERVAL);
        }
    }
}

fn answer_request(stream: TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line == "\r\n" {
            break;
        }
        let lowercase = line.to_ascii_lowercase();
        if let Some(value) = lowercase.strip_prefix("content-length:") {
            content_length = value.trim().parse().ok()?;
        }
    }

    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).ok()?;
    let mut stream = reader.into_inner();
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .ok()?;
    stream.flush().ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

fn start_webhook_listener() -> RecordedWebhooks {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&bodies);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            if let Some(body) = answer_request(stream) {
                recorded.lock().unwrap().push(body);
            }
        }
    });

    RecordedWebhooks { url, bodies }
}

struct Watcher {
    child: Child,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_watcher(
    watch_dir: &Path,
    output_dir: &Path,
    webhooks: &RecordedWebhooks,
    config_home: &Path,
) -> Watcher {
    let child = Command::new(env!("CARGO_BIN_EXE_dcpwizard"))
        .env("XDG_CONFIG_HOME", config_home)
        .args([
            "watch",
            watch_dir.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--interval",
            POLL_INTERVAL_SECONDS,
            "--webhook-url",
            &webhooks.url,
        ])
        .spawn()
        .unwrap();
    Watcher { child }
}

fn run_ffmpeg(arguments: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(arguments)
        .status()
        .expect("ffmpeg is required by the watch folder tests");
    assert!(status.success(), "ffmpeg failed: {arguments:?}");
}

fn make_test_video(directory: &Path, file_name: &str) -> PathBuf {
    let path = directory.join(file_name);
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=2048x1080:rate=24",
        "-frames:v",
        "12",
        "-pix_fmt",
        "yuv420p",
        path.to_str().unwrap(),
    ]);
    path
}

fn make_silent_wav(directory: &Path, file_name: &str) -> PathBuf {
    let path = directory.join(file_name);
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "anullsrc=r=48000:cl=stereo",
        "-t",
        "0.5",
        // DCI sound is 24-bit PCM, and anullsrc defaults to 16
        "-c:a",
        "pcm_s24le",
        path.to_str().unwrap(),
    ]);
    path
}

fn wait_for(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if path.exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(CHECK_INTERVAL);
    }
}

fn file_named(directory: &Path, prefix: &str, extension: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(directory).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(prefix) && name.ends_with(extension) {
            return Some(entry.path());
        }
    }
    None
}

#[test]
fn a_master_that_lands_becomes_a_dcp() {
    let watch_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let webhooks = start_webhook_listener();
    let mut watcher = start_watcher(
        watch_dir.path(),
        output_dir.path(),
        &webhooks,
        config_home.path(),
    );

    let video = make_test_video(scratch.path(), "feature.mp4");
    let audio = make_silent_wav(scratch.path(), "feature.wav");
    std::fs::rename(&audio, watch_dir.path().join("feature.wav")).unwrap();
    std::fs::rename(&video, watch_dir.path().join("feature.mp4")).unwrap();

    let package_dir = output_dir.path().join("feature");
    assert!(
        wait_for(&package_dir.join("ASSETMAP.xml"), BUILD_TIMEOUT),
        "no ASSETMAP.xml under {}",
        package_dir.display()
    );
    assert!(
        wait_for(
            &watch_dir.path().join("done").join("feature.mp4"),
            BUILD_TIMEOUT
        ),
        "the master was not moved into done/"
    );

    let cpl = file_named(&package_dir, "CPL_", ".xml").expect("no CPL in the package");
    let cpl_text = std::fs::read_to_string(&cpl).unwrap();
    assert!(
        cpl_text.contains("<ContentTitleText>feature</ContentTitleText>"),
        "the CPL does not carry the file stem as its title"
    );
    assert!(
        file_named(&package_dir, "sound_", ".mxf").is_some(),
        "the sidecar wav was not packaged as sound"
    );
    assert!(
        watch_dir.path().join("done").join("feature.wav").exists(),
        "the sidecar wav was not moved into done/"
    );

    let log = std::fs::read_to_string(output_dir.path().join("feature.log")).unwrap();
    assert!(!log.is_empty(), "the job log is empty");

    let created = webhooks.wait_for_one("dcp.created");
    assert_eq!(
        created.len(),
        1,
        "expected one dcp.created body, got {created:?}"
    );

    watcher.child.kill().unwrap();
    let status = watcher.child.wait().unwrap();
    assert!(!status.success(), "the watcher was not killed");
    assert!(
        watcher.child.try_wait().unwrap().is_some(),
        "the watcher is still running"
    );
}

#[test]
fn a_master_create_refuses_lands_in_failed() {
    let watch_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let webhooks = start_webhook_listener();
    let _watcher = start_watcher(
        watch_dir.path(),
        output_dir.path(),
        &webhooks,
        config_home.path(),
    );

    let broken = scratch.path().join("broken.mov");
    std::fs::write(&broken, [0u8; 100]).unwrap();
    std::fs::rename(&broken, watch_dir.path().join("broken.mov")).unwrap();

    assert!(
        wait_for(
            &watch_dir.path().join("failed").join("broken.mov"),
            FAILURE_TIMEOUT
        ),
        "the master was not moved into failed/"
    );

    let log = std::fs::read_to_string(output_dir.path().join("broken.log")).unwrap();
    assert!(
        log.contains("ffprobe could not inspect"),
        "the job log does not name the failure: {log}"
    );

    let failed = webhooks.wait_for_one("dcp.failed");
    assert_eq!(
        failed.len(),
        1,
        "expected one dcp.failed body, got {failed:?}"
    );
}

#[test]
fn a_growing_file_is_not_built_until_it_stops_changing() {
    let watch_dir = TempDir::new().unwrap();
    let output_dir = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let webhooks = start_webhook_listener();
    let finished = make_test_video(scratch.path(), "finished.mp4");
    let _watcher = start_watcher(
        watch_dir.path(),
        output_dir.path(),
        &webhooks,
        config_home.path(),
    );

    let growing_path = watch_dir.path().join("slow.mp4");
    let mut growing = std::fs::File::create(&growing_path).unwrap();
    for _ in 0..GROWING_APPEND_COUNT {
        growing.write_all(&[0u8; GROWING_APPEND_BYTES]).unwrap();
        growing.flush().unwrap();
        std::thread::sleep(GROWING_APPEND_INTERVAL);
    }
    drop(growing);

    let log_path = output_dir.path().join("slow.log");
    assert!(
        !log_path.exists(),
        "the watcher started a build while the file was still growing"
    );

    std::fs::rename(&finished, &growing_path).unwrap();

    let package_dir = output_dir.path().join("slow");
    assert!(
        wait_for(&package_dir.join("ASSETMAP.xml"), BUILD_TIMEOUT),
        "no ASSETMAP.xml once the file stopped changing"
    );
    assert!(
        wait_for(
            &watch_dir.path().join("done").join("slow.mp4"),
            BUILD_TIMEOUT
        ),
        "the master was not moved into done/"
    );
}
