use dcpwizard_core::multi_cpl::{self, CplEntry, TimelineEntry};
use std::path::Path;

#[tauri::command]
pub async fn list_cpls(dcp_dir: String) -> Result<Vec<CplEntry>, String> {
    let path = Path::new(&dcp_dir);
    if !path.exists() {
        return Err("DCP directory not found".into());
    }
    Ok(multi_cpl::list_cpls(path))
}

#[tauri::command]
pub async fn get_timeline(cpl_path: String) -> Result<Vec<TimelineEntry>, String> {
    let path = Path::new(&cpl_path);
    if !path.exists() {
        return Err("CPL file not found".into());
    }
    Ok(multi_cpl::get_timeline(path))
}
