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
