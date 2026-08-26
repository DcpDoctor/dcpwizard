//! The Jobs panel queue on disk: one JSON line per job record, appended when a
//! job is queued and on every state change after it, so closing the window or
//! losing the process does not lose what was queued.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::pipeline::JobConfig;

/// What a job that was still running when the process ended is failed with.
/// What a job left running when the window closed reports on the next launch.
pub const INTERRUPTED_MESSAGE: &str = "the app closed while this job was running";

/// Where the Jobs panel keeps its queue.
pub fn jobs_path() -> PathBuf {
    dcpwizard_core::store::data_dir().join("gui-jobs.jsonl")
}

/// The states the queue moves a job through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredJobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

/// One line of the jobs file.
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredJob {
    pub state: StoredJobState,
    pub message: String,
    pub config: JobConfig,
}

/// Append one record as a JSON line, creating the file and its parent dir.
fn append_record(path: &Path, record: &StoredJob) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let mut line = serde_json::to_string(record).map_err(|e| format!("serialize job: {e}"))?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    file.write_all(line.as_bytes())
        .map_err(|e| format!("cannot append: {e}"))
}

/// Record a job at the state it has just reached.
pub fn record(path: &Path, state: StoredJobState, message: &str, config: &JobConfig) {
    let stored = StoredJob {
        state,
        message: message.to_string(),
        config: config.clone(),
    };
    if let Err(e) = append_record(path, &stored) {
        tracing_error(&format!(
            "could not record job {} in {}: {e}",
            config.id,
            path.display()
        ));
    }
}

/// The GUI has no tracing subscriber, so an error goes where the job log goes.
fn tracing_error(message: &str) {
    eprintln!("[jobs] {message}");
}

/// What the jobs file held: the last record per job id, ordered by id, with a
/// job left running failed, plus how many lines could not be read.
pub struct LoadedJobs {
    pub jobs: Vec<StoredJob>,
    pub skipped: usize,
}

/// Read the jobs file and rewrite it with one line per job.
pub fn load(path: &Path) -> LoadedJobs {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return LoadedJobs {
                jobs: Vec::new(),
                skipped: 0,
            };
        }
        Err(e) => {
            tracing_error(&format!("could not read {}: {e}", path.display()));
            return LoadedJobs {
                jobs: Vec::new(),
                skipped: 0,
            };
        }
    };

    let mut jobs: Vec<StoredJob> = Vec::new();
    let mut skipped = 0;
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<StoredJob>(line) {
            Ok(mut stored) => {
                if stored.state == StoredJobState::Running {
                    stored.state = StoredJobState::Failed;
                    stored.message = INTERRUPTED_MESSAGE.to_string();
                }
                match jobs
                    .iter()
                    .position(|job| job.config.id == stored.config.id)
                {
                    Some(at) => jobs[at] = stored,
                    None => jobs.push(stored),
                }
            }
            Err(e) => {
                skipped += 1;
                tracing_error(&format!(
                    "{} line {}: not a job record: {e}",
                    path.display(),
                    index + 1
                ));
            }
        }
    }
    if skipped > 0 {
        tracing_error(&format!(
            "skipped {skipped} unreadable lines in {}",
            path.display()
        ));
    }

    jobs.sort_by_key(|job| job.config.id);
    write_all(path, &jobs);
    LoadedJobs { jobs, skipped }
}

/// Replace the file with one line per job.
fn write_all(path: &Path, jobs: &[StoredJob]) {
    let mut text = String::new();
    for job in jobs {
        match serde_json::to_string(job) {
            Ok(line) => {
                text.push_str(&line);
                text.push('\n');
            }
            Err(e) => tracing_error(&format!("could not serialize job {}: {e}", job.config.id)),
        }
    }
    if let Err(e) = std::fs::write(path, text) {
        tracing_error(&format!("could not rewrite {}: {e}", path.display()));
    }
}
