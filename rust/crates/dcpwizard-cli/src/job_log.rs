use std::io::Write;
use std::path::Path;

const LOG_NAME: &str = "dcpwizard.log";

// the three states the GUI's job log names the accelerator by, so one log reads
// like the other
pub fn accelerator_status(requested: bool, active: bool, error: Option<&str>) -> String {
    match (requested, active, error) {
        (false, _, _) => "off".to_string(),
        (true, true, _) => "requested, active".to_string(),
        (true, false, Some(error)) => format!("requested, inactive: {error}"),
        (true, false, None) => "requested, inactive".to_string(),
    }
}

pub struct JobLog(std::fs::File);

impl JobLog {
    pub fn create(output: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(output)
            .map_err(|e| format!("Cannot create the output folder {}: {e}", output.display()))?;
        let path = output.join(LOG_NAME);
        std::fs::File::create(&path)
            .map(Self)
            .map_err(|e| format!("Cannot create the job log {}: {e}", path.display()))
    }

    pub fn line(&mut self, text: &str) {
        let _ = writeln!(self.0, "{text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_names_what_was_asked_for_and_why_it_failed() {
        assert_eq!(accelerator_status(false, false, None), "off");
        assert_eq!(accelerator_status(true, true, None), "requested, active");
        assert_eq!(
            accelerator_status(true, false, Some("the plugin did not initialise")),
            "requested, inactive: the plugin did not initialise"
        );
        assert_eq!(accelerator_status(true, false, None), "requested, inactive");
    }

    #[test]
    fn the_log_is_created_under_an_output_folder_that_does_not_exist_yet() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("dcp");
        let mut log = JobLog::create(&output).expect("the log has to be created");
        log.line("Accelerator: off");
        assert_eq!(
            std::fs::read_to_string(output.join(LOG_NAME)).unwrap(),
            "Accelerator: off\n"
        );
    }
}
