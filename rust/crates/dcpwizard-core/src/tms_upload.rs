//! where dcpwizard keeps its TMS config. the upload itself is `postkit::tms`,
//! which every app in the family shares; only the config file is ours, because
//! its path carries our name.

use std::path::{Path, PathBuf};

pub use postkit::tms::{TmsConfig, upload_package};

/// where `tms` and `create --upload-to-tms` look for the config when no path is
/// given. same place the README points `--smtp-config` at.
pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("dcpwizard")
        .join("tms.toml")
}

pub fn load_config(path: &Path) -> Result<TmsConfig, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read tms config {}: {e}", path.display()))?;
    parse_config(&text)
}

fn parse_config(text: &str) -> Result<TmsConfig, String> {
    let config: TmsConfig = toml::from_str(text).map_err(|e| format!("invalid tms config: {e}"))?;
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_config_path_is_a_tms_toml_beside_the_other_config() {
        let path = default_config_path();
        assert!(path.ends_with("dcpwizard/tms.toml"), "{}", path.display());
    }

    #[test]
    fn a_config_parses_and_a_field_it_leaves_empty_is_refused() {
        let config = parse_config(
            r#"
            protocol = "sftp"
            host = "tms.cinema.test"
            path = "/dcp"
            user = "projectionist"
            password = "hunter2"
            "#,
        )
        .unwrap();
        assert_eq!(config.port(), 22);
        assert!(!format!("{config:?}").contains("hunter2"));

        let err = parse_config(
            r#"
            protocol = "sftp"
            host = "  "
            path = "/dcp"
            user = "projectionist"
            password = "hunter2"
            "#,
        )
        .unwrap_err();
        assert!(err.contains("needs a host"), "{err}");

        let err = parse_config(
            r#"
            protocol = "carrier-pigeon"
            host = "tms.cinema.test"
            path = "/dcp"
            user = "u"
            password = "p"
            "#,
        )
        .unwrap_err();
        assert!(err.contains("invalid tms config"), "{err}");
    }
}
