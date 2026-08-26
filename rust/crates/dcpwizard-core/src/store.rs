//! where the kdm distribution data files live: the xdg data dir and the
//! default paths under it.

use std::path::PathBuf;

/// base data dir for dcpwizard state (~/.local/share/dcpwizard on linux).
pub fn data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("dcpwizard")
}

pub fn default_db_path() -> PathBuf {
    data_dir().join("cinemas.json")
}

pub fn default_history_path() -> PathBuf {
    data_dir().join("kdm-history.jsonl")
}

pub fn default_templates_path() -> PathBuf {
    data_dir().join("kdm-templates.json")
}

/// where the daemon keeps its job queue. `DCPWIZARD_JOBS_FILE` points a second
/// daemon, or a test, at a file of its own.
pub fn jobs_path() -> PathBuf {
    match std::env::var("DCPWIZARD_JOBS_FILE") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => data_dir().join("jobs.jsonl"),
    }
}
