//! Job queue with a TCP/IPC daemon.
//!
//! Distinct from [`postkit::job_queue`], which is an in-memory queue with job
//! dependencies but no daemon. This one adds cross-process IPC and job types
//! bound to dcpwizard's own pipeline (create/verify/export/import DCP), so it
//! stays local.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};

/// What a job that was still Running when the queue was loaded again is failed
/// with. Nothing can pick a half-run job back up.
pub const INTERRUPTED_MESSAGE: &str = "the daemon stopped while this job was running";

/// What a job is failed with when its worker thread ended without sending a
/// result back.
pub const WORKER_LOST_MESSAGE: &str = "the job thread stopped without reporting a result";

/// Job type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobType {
    CreateDcp,
    VerifyDcp,
    ExportDcp,
    ImportVideo,
    EncodeJ2k,
    WrapMxf,
    CopyToDrive,
}

/// Job state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JobState {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// A queued job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub job_type: JobType,
    pub state: JobState,
    pub progress_percent: u32,
    pub message: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub params: String,
}

/// IPC request sent from CLI client to daemon.
#[derive(Debug, Serialize, Deserialize)]
pub enum IpcRequest {
    List,
    Submit { job_type: JobType, params: String },
    Cancel { id: String },
    Status { id: String },
}

/// IPC response sent from daemon to CLI client.
#[derive(Debug, Serialize, Deserialize)]
pub enum IpcResponse {
    Jobs(Vec<Job>),
    Submitted { id: String },
    Cancelled(bool),
    JobStatus(Option<Job>),
    Error(String),
}

/// Thread-safe job queue, backed by a JSONL file so a crash or a reboot does not
/// lose what is queued.
#[derive(Clone)]
pub struct JobQueue {
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    running: Arc<Mutex<bool>>,
    /// cooperative-cancel flag per running job; the job loop and the running
    /// operation both watch it so a cancel stops in-flight work between stages.
    cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    /// one JSON line per job record, appended on submit and on every state
    /// change. the last record for an id is the job.
    jobs_file: Arc<PathBuf>,
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue {
    pub fn new() -> Self {
        Self::with_jobs_file(crate::store::jobs_path())
    }

    pub fn with_jobs_file(jobs_file: PathBuf) -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            running: Arc::new(Mutex::new(false)),
            cancel_flags: Arc::new(Mutex::new(HashMap::new())),
            jobs_file: Arc::new(jobs_file),
        }
    }

    /// Read the jobs file back into the queue and rewrite it with one line per
    /// job. A job left Running is failed with [`INTERRUPTED_MESSAGE`]. Returns
    /// how many lines could not be read.
    pub fn load_jobs_file(&self) -> usize {
        let path = self.jobs_file.as_path();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
            Err(e) => {
                tracing::error!("could not read {}: {e}", path.display());
                return 0;
            }
        };

        let mut loaded: HashMap<String, Job> = HashMap::new();
        let mut skipped = 0;
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Job>(line) {
                Ok(mut job) => {
                    if job.state == JobState::Running {
                        job.state = JobState::Failed;
                        job.message = INTERRUPTED_MESSAGE.to_string();
                    }
                    loaded.insert(job.id.clone(), job);
                }
                Err(e) => {
                    skipped += 1;
                    tracing::error!(
                        "{} line {}: not a job record: {e}",
                        path.display(),
                        index + 1
                    );
                }
            }
        }
        tracing::info!("loaded {} jobs from {}", loaded.len(), path.display());
        if skipped > 0 {
            tracing::error!("skipped {skipped} unreadable lines in {}", path.display());
        }

        let mut compacted: Vec<&Job> = loaded.values().collect();
        compacted.sort_by_key(|job| job.created_at);
        write_jobs_file(path, &compacted);

        if let Ok(mut jobs) = self.jobs.lock() {
            *jobs = loaded;
        }
        skipped
    }

    fn record(&self, job: &Job) {
        if let Err(e) = append_job_record(&self.jobs_file, job) {
            tracing::error!(
                "could not record job {} in {}: {e}",
                job.id,
                self.jobs_file.display()
            );
        }
    }

    /// Submit a new job to the queue.
    pub fn submit(&self, job_type: JobType, params: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = current_epoch_secs();

        let job = Job {
            id: id.clone(),
            job_type,
            state: JobState::Pending,
            progress_percent: 0,
            message: String::new(),
            created_at: now,
            updated_at: now,
            params: params.to_string(),
        };

        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.insert(id.clone(), job.clone());
        }
        self.record(&job);

        tracing::info!("Submitted job {id}");
        id
    }

    /// Cancel a job by ID. A pending job never starts; a running job is asked to
    /// stop via its cancel flag and the job loop finalises it as Cancelled.
    pub fn cancel(&self, id: &str) -> bool {
        let cancelled = {
            let mut cancelled = None;
            if let Ok(mut jobs) = self.jobs.lock()
                && let Some(job) = jobs.get_mut(id)
                && (job.state == JobState::Pending || job.state == JobState::Running)
            {
                job.state = JobState::Cancelled;
                job.updated_at = current_epoch_secs();
                cancelled = Some(job.clone());
            }
            cancelled
        };

        let Some(job) = cancelled else {
            return false;
        };
        self.record(&job);
        // signal a running operation to bail between stages
        if let Ok(flags) = self.cancel_flags.lock()
            && let Some(flag) = flags.get(id)
        {
            flag.store(true, Ordering::Relaxed);
        }
        tracing::info!("Cancelled job {id}");
        true
    }

    /// Get a job by ID.
    pub fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().ok()?.get(id).cloned()
    }

    /// List all jobs.
    pub fn list(&self) -> Vec<Job> {
        match self.jobs.lock() {
            Ok(jobs) => {
                let mut result: Vec<Job> = jobs.values().cloned().collect();
                result.sort_by_key(|j| std::cmp::Reverse(j.created_at));
                result
            }
            Err(_) => Vec::new(),
        }
    }

    /// Update a job's state and progress.
    pub fn update_job(&self, id: &str, state: JobState, progress: u32, message: &str) {
        let updated = {
            let mut updated = None;
            if let Ok(mut jobs) = self.jobs.lock()
                && let Some(job) = jobs.get_mut(id)
            {
                job.state = state;
                job.progress_percent = progress;
                job.message = message.to_string();
                job.updated_at = current_epoch_secs();
                updated = Some(job.clone());
            }
            updated
        };
        if let Some(job) = updated {
            self.record(&job);
        }
    }
}

/// Append one job record as a JSON line, creating the file and its parent dir.
fn append_job_record(path: &Path, job: &Job) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let mut line = serde_json::to_string(job).map_err(|e| format!("serialize job: {e}"))?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    file.write_all(line.as_bytes())
        .map_err(|e| format!("cannot append: {e}"))
}

/// Replace the file with one line per job.
fn write_jobs_file(path: &Path, jobs: &[&Job]) {
    let mut text = String::new();
    for job in jobs {
        match serde_json::to_string(job) {
            Ok(line) => {
                text.push_str(&line);
                text.push('\n');
            }
            Err(e) => tracing::error!("could not serialize job {}: {e}", job.id),
        }
    }
    if let Err(e) = std::fs::write(path, text) {
        tracing::error!("could not rewrite {}: {e}", path.display());
    }
}

/// Start the job queue processor in a background thread.
pub fn start_job_queue(queue: &JobQueue) {
    if let Ok(mut running) = queue.running.lock() {
        if *running {
            tracing::warn!("Job queue is already running");
            return;
        }
        *running = true;
    }

    let queue_clone = queue.clone();
    std::thread::spawn(move || {
        tracing::info!("Job queue processor started");
        loop {
            let is_running = queue_clone.running.lock().map(|r| *r).unwrap_or(false);
            if !is_running {
                break;
            }

            // Find next pending job
            let next_job = {
                let jobs = match queue_clone.jobs.lock() {
                    Ok(j) => j,
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        continue;
                    }
                };
                jobs.values()
                    .filter(|j| j.state == JobState::Pending)
                    .min_by_key(|j| j.created_at)
                    .cloned()
            };

            if let Some(job) = next_job {
                queue_clone.update_job(&job.id, JobState::Running, 0, "Processing...");
                tracing::info!("Processing job {} ({:?})", job.id, job.job_type);

                // run the job on its own thread so the loop can watch the cancel
                // flag and finalise the job even if the operation is still running
                let cancel = Arc::new(AtomicBool::new(false));
                if let Ok(mut flags) = queue_clone.cancel_flags.lock() {
                    flags.insert(job.id.clone(), cancel.clone());
                }
                let control = JobControl {
                    queue: queue_clone.clone(),
                    job_id: job.id.clone(),
                    cancel: cancel.clone(),
                };
                let (tx, rx) = mpsc::channel();
                let worker_job = job.clone();
                std::thread::spawn(move || {
                    let outcome = process_job(&worker_job, &control);
                    let _ = tx.send(outcome);
                });

                let outcome = loop {
                    match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                        Ok(outcome) => break Some(outcome),
                        Err(RecvTimeoutError::Timeout) => {
                            if cancel.load(Ordering::Relaxed) {
                                break None; // cancelled; detach the worker
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break None,
                    }
                };

                if let Ok(mut flags) = queue_clone.cancel_flags.lock() {
                    flags.remove(&job.id);
                }

                if cancel.load(Ordering::Relaxed) {
                    queue_clone.update_job(&job.id, JobState::Cancelled, 0, "Cancelled");
                } else {
                    match outcome {
                        Some(Ok(())) => queue_clone.update_job(
                            &job.id,
                            JobState::Completed,
                            100,
                            "Completed successfully",
                        ),
                        Some(Err(cause)) => {
                            tracing::error!("job {} failed: {cause}", job.id);
                            queue_clone.update_job(&job.id, JobState::Failed, 0, &cause)
                        }
                        None => queue_clone.update_job(
                            &job.id,
                            JobState::Failed,
                            0,
                            WORKER_LOST_MESSAGE,
                        ),
                    }
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        tracing::info!("Job queue processor stopped");
    });
}

/// Stop the job queue processor.
pub fn stop_job_queue(queue: &JobQueue) {
    if let Ok(mut running) = queue.running.lock() {
        *running = false;
    }
    tracing::info!("Job queue stop requested");
}

/// Progress + cancel bridge handed to a running operation; forwards stage
/// updates to the job's queue entry and exposes the cooperative cancel flag.
struct JobControl {
    queue: JobQueue,
    job_id: String,
    cancel: Arc<AtomicBool>,
}

impl crate::dcp::ProgressSink for JobControl {
    fn stage(&self, percent: u32, message: &str) {
        self.queue
            .update_job(&self.job_id, JobState::Running, percent, message);
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

fn parse_params<T: serde::de::DeserializeOwned>(params: &str, job_type: &str) -> Result<T, String> {
    serde_json::from_str(params).map_err(|e| format!("invalid {job_type} params: {e}"))
}

/// An operation that reports only an exit code leaves the cause in the daemon
/// log, so say which operation failed and with what.
fn from_exit_code(code: i32, operation: &str) -> Result<(), String> {
    if code == 0 {
        return Ok(());
    }
    Err(format!(
        "{operation} failed with code {code}, the daemon log holds the cause"
    ))
}

fn process_job(job: &Job, control: &JobControl) -> Result<(), String> {
    match job.job_type {
        JobType::CreateDcp => {
            let config = parse_params::<crate::dcp::DcpConfig>(&job.params, "CreateDcp")?;
            crate::dcp::create_dcp_with_progress(&config, control)
        }
        JobType::VerifyDcp => {
            let path = std::path::PathBuf::from(&job.params);
            let result = crate::verify::verify_dcp(&path);
            if result.valid {
                return Ok(());
            }
            let cause = result.errors.join("; ");
            Err(if cause.is_empty() {
                "the DCP did not verify".to_string()
            } else {
                cause
            })
        }
        JobType::ExportDcp => {
            let config = parse_params::<crate::export::ExportConfig>(&job.params, "ExportDcp")?;
            from_exit_code(crate::export::export_dcp(&config), "exporting the DCP")
        }
        JobType::ImportVideo => {
            let config = parse_params::<crate::import::ImportConfig>(&job.params, "ImportVideo")?;
            from_exit_code(crate::import::import_video(&config), "importing the video")
        }
        JobType::EncodeJ2k => {
            let encode =
                parse_params::<crate::encode::ImageSequenceEncode>(&job.params, "EncodeJ2k")?;
            let report = |progress: &postkit::pipeline::PipelineProgress| {
                crate::dcp::ProgressSink::stage(
                    control,
                    progress.percent as u32,
                    &format!("{}/{} frames", progress.frame, progress.total_frames),
                );
            };
            crate::encode::encode_image_sequence(&encode, &control.cancel, report).map(|_| ())
        }
        JobType::WrapMxf => {
            let config = parse_params::<crate::mxf_wrap::MxfWrapConfig>(&job.params, "WrapMxf")?;
            from_exit_code(crate::mxf_wrap::wrap_mxf(&config), "wrapping the MXF")
        }
        JobType::CopyToDrive => {
            // params is JSON {"source": "...", "target": "..."}
            let map = parse_params::<HashMap<String, String>>(&job.params, "CopyToDrive")?;
            let src = std::path::Path::new(map.get("source").map(|s| s.as_str()).unwrap_or(""));
            let dst = std::path::Path::new(map.get("target").map(|s| s.as_str()).unwrap_or(""));
            from_exit_code(
                crate::copy_drive::copy_to_drive(src, dst),
                "copying to the drive",
            )
        }
    }
}

fn current_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Get the daemon address.
/// Uses TCP localhost on a fixed port for cross-platform compatibility.
pub fn daemon_addr() -> String {
    std::env::var("DCPWIZARD_DAEMON_ADDR").unwrap_or_else(|_| "127.0.0.1:9457".to_string())
}

/// Start the daemon IPC listener.
/// Binds a TCP listener on localhost and processes client requests.
/// This blocks the current thread.
pub fn start_daemon_ipc(queue: &JobQueue) -> i32 {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let addr = daemon_addr();

    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to bind {addr}: {e}");
            return -1;
        }
    };

    tracing::info!("Daemon listening on {addr}");

    queue.load_jobs_file();

    // Start the job processor thread
    start_job_queue(queue);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let queue = queue.clone();
                std::thread::spawn(move || {
                    let reader = BufReader::new(match stream.try_clone() {
                        Ok(s) => s,
                        Err(_) => return,
                    });

                    for line in reader.lines() {
                        let line = match line {
                            Ok(l) => l,
                            Err(_) => break,
                        };

                        let request: IpcRequest = match serde_json::from_str(&line) {
                            Ok(r) => r,
                            Err(e) => {
                                let resp = IpcResponse::Error(format!("invalid request: {e}"));
                                let _ = writeln!(
                                    stream,
                                    "{}",
                                    serde_json::to_string(&resp).unwrap_or_default()
                                );
                                continue;
                            }
                        };

                        let response = match request {
                            IpcRequest::List => IpcResponse::Jobs(queue.list()),
                            IpcRequest::Submit { job_type, params } => {
                                let id = queue.submit(job_type, &params);
                                IpcResponse::Submitted { id }
                            }
                            IpcRequest::Cancel { id } => IpcResponse::Cancelled(queue.cancel(&id)),
                            IpcRequest::Status { id } => IpcResponse::JobStatus(queue.get(&id)),
                        };

                        let json = serde_json::to_string(&response).unwrap_or_default();
                        if writeln!(stream, "{json}").is_err() {
                            break;
                        }
                    }
                });
            }
            Err(e) => {
                tracing::error!("accept error: {e}");
            }
        }
    }

    0
}

/// Send an IPC request to the running daemon and return the response.
pub fn send_ipc_request(request: &IpcRequest) -> Result<IpcResponse, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    let addr = daemon_addr();
    let mut stream = TcpStream::connect(&addr)
        .map_err(|e| format!("cannot connect to daemon at {addr}: {e} (is the daemon running?)"))?;

    let json = serde_json::to_string(request).map_err(|e| format!("serialize error: {e}"))?;
    writeln!(stream, "{json}").map_err(|e| format!("write error: {e}"))?;

    let reader = BufReader::new(stream);
    let line = reader
        .lines()
        .next()
        .ok_or_else(|| "no response from daemon".to_string())?
        .map_err(|e| format!("read error: {e}"))?;

    serde_json::from_str(&line).map_err(|e| format!("invalid response: {e}"))
}

/// Check if the daemon is running by attempting a connection.
pub fn is_daemon_running() -> bool {
    use std::net::TcpStream;
    let addr = daemon_addr();
    TcpStream::connect_timeout(
        &addr
            .parse()
            .unwrap_or_else(|_| "127.0.0.1:9457".parse().unwrap()),
        std::time::Duration::from_millis(500),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_keeps_pending_and_fails_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("jobs.jsonl");

        let queue = JobQueue::with_jobs_file(path.clone());
        let pending = queue.submit(JobType::VerifyDcp, "/dcp/one");
        let running = queue.submit(JobType::VerifyDcp, "/dcp/two");
        queue.update_job(&running, JobState::Running, 40, "Processing...");

        let reloaded = JobQueue::with_jobs_file(path.clone());
        assert_eq!(reloaded.load_jobs_file(), 0);

        assert_eq!(reloaded.get(&pending).unwrap().state, JobState::Pending);
        let interrupted = reloaded.get(&running).unwrap();
        assert_eq!(interrupted.state, JobState::Failed);
        assert_eq!(interrupted.message, INTERRUPTED_MESSAGE);

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
    }

    /// How long the queue processor gets to pick up and finish one job.
    const FAILURE_POLL_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

    #[test]
    fn a_failed_job_carries_the_runners_own_error_text() {
        let dir = tempfile::tempdir().unwrap();
        let missing_dcp = dir.path().join("no_such_dcp");

        let queue = JobQueue::with_jobs_file(dir.path().join("jobs.jsonl"));
        let id = queue.submit(JobType::VerifyDcp, missing_dcp.to_str().unwrap());
        start_job_queue(&queue);

        let deadline = std::time::Instant::now() + FAILURE_POLL_LIMIT;
        let failed = loop {
            let job = queue.get(&id).expect("the submitted job");
            if job.state == JobState::Failed {
                break job;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "job stayed {:?} for {FAILURE_POLL_LIMIT:?}",
                job.state
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        stop_job_queue(&queue);

        assert!(
            failed.message.contains(missing_dcp.to_str().unwrap()),
            "message hid the cause: {}",
            failed.message
        );
    }

    #[test]
    fn a_failed_create_carries_the_missing_source_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing_j2k = dir.path().join("no_such_frames");
        let config = crate::dcp::DcpConfig {
            title: "Test Film".into(),
            output_dir: dir.path().join("out"),
            j2k_dir: Some(missing_j2k.clone()),
            frame_rate_num: 24,
            frame_rate_den: 1,
            ..Default::default()
        };
        let params = serde_json::to_string(&config).unwrap();

        let queue = JobQueue::with_jobs_file(dir.path().join("jobs.jsonl"));
        let id = queue.submit(JobType::CreateDcp, &params);
        start_job_queue(&queue);

        let deadline = std::time::Instant::now() + FAILURE_POLL_LIMIT;
        let failed = loop {
            let job = queue.get(&id).expect("the submitted job");
            if job.state == JobState::Failed {
                break job;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "job stayed {:?} for {FAILURE_POLL_LIMIT:?}",
                job.state
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        stop_job_queue(&queue);

        assert!(
            failed.message.contains(missing_j2k.to_str().unwrap()),
            "message hid the cause: {}",
            failed.message
        );
    }

    #[test]
    fn corrupt_line_is_skipped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.jsonl");

        let queue = JobQueue::with_jobs_file(path.clone());
        let id = queue.submit(JobType::VerifyDcp, "/dcp/one");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("{not json}\n");
        std::fs::write(&path, text).unwrap();

        let reloaded = JobQueue::with_jobs_file(path.clone());
        assert_eq!(reloaded.load_jobs_file(), 1);
        assert_eq!(reloaded.list().len(), 1);
        assert_eq!(reloaded.get(&id).unwrap().state, JobState::Pending);
    }
}
