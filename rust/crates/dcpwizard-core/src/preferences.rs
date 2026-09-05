use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub const CURRENT_PREFERENCES_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub version: u32,
    #[serde(alias = "default_standard")]
    pub standard: String,
    #[serde(alias = "default_resolution")]
    pub resolution: String,
    #[serde(alias = "default_frame_rate")]
    pub framerate: u32,
    pub encrypt: bool,
    pub stereo3d: bool,
    pub validate: bool,
    #[serde(alias = "creator_name")]
    pub creator: String,
    #[serde(alias = "isdcf_facility_code")]
    pub facility: String,
    #[serde(alias = "default_bandwidth_mbps")]
    pub bandwidth: u32,
    pub gpu: bool,
    pub gpu_license: String,
    pub gpu_registration_url: String,
    #[serde(alias = "signing_certificate_path")]
    pub signing_cert: String,
    #[serde(alias = "signing_key_path")]
    pub signing_key: String,
    #[serde(alias = "default_output_dir")]
    pub output_dir: String,
    pub isdcf_naming: bool,
    #[serde(alias = "default_channel_config")]
    pub channels: String,
    pub show_hints_before_build: bool,
    #[serde(flatten)]
    pub additional: BTreeMap<String, serde_json::Value>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: CURRENT_PREFERENCES_VERSION,
            standard: "SMPTE".to_string(),
            resolution: "2K".to_string(),
            framerate: 24,
            encrypt: false,
            stereo3d: false,
            validate: true,
            creator: String::new(),
            facility: String::new(),
            bandwidth: 230,
            gpu: false,
            gpu_license: String::new(),
            gpu_registration_url: String::new(),
            signing_cert: String::new(),
            signing_key: String::new(),
            output_dir: String::new(),
            isdcf_naming: false,
            channels: "5.1".to_string(),
            show_hints_before_build: true,
            additional: BTreeMap::new(),
        }
    }
}

pub fn preferences_path() -> PathBuf {
    postkit::preferences::config_dir("dcpwizard").join("preferences.json")
}

pub fn load_preferences() -> io::Result<Preferences> {
    Ok(load_preferences_if_present()?.unwrap_or_default())
}

pub fn load_preferences_if_present() -> io::Result<Option<Preferences>> {
    load_preferences_from(&preferences_path())
}

pub fn load_preferences_from(path: &Path) -> io::Result<Option<Preferences>> {
    let Some(contents) = postkit::preferences::read_preferences_file(path)? else {
        return Ok(None);
    };
    let stored_version = postkit::preferences::prefs_version(&contents);
    let mut preferences: Preferences = serde_json::from_str(&contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    if stored_version < CURRENT_PREFERENCES_VERSION {
        preferences.version = CURRENT_PREFERENCES_VERSION;
        save_preferences_to(&preferences, path)?;
    }

    Ok(Some(preferences))
}

pub fn save_preferences(preferences: &Preferences) -> io::Result<()> {
    save_preferences_to(preferences, &preferences_path())
}

pub fn save_preferences_to(preferences: &Preferences, path: &Path) -> io::Result<()> {
    if preferences.version > CURRENT_PREFERENCES_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "preferences version {} is newer than supported version {}",
                preferences.version, CURRENT_PREFERENCES_VERSION
            ),
        ));
    }

    let mut current = preferences.clone();
    current.version = CURRENT_PREFERENCES_VERSION;
    let contents = serde_json::to_string_pretty(&current).map_err(io::Error::other)?;
    postkit::preferences::write_preferences_file(path, &contents)
}

pub fn reset_preferences() -> io::Result<Preferences> {
    let preferences = Preferences::default();
    save_preferences(&preferences)?;
    Ok(preferences)
}

pub fn set_preference(name: &str, value: &str) -> Result<Preferences, String> {
    let preferences = load_preferences().map_err(|error| error.to_string())?;
    let preferences = postkit::preferences::set_json_preference(&preferences, name, value)?;
    save_preferences(&preferences).map_err(|error| error.to_string())?;
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn older_file_adds_defaults_and_updates_version() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        let contents =
            r#"{"version":1,"creator_name":"Studio","default_bandwidth_mbps":180,"theme":"dark"}"#;
        postkit::preferences::write_preferences_file(&path, contents).unwrap();

        let preferences = load_preferences_from(&path).unwrap().unwrap();

        assert_eq!(preferences.creator, "Studio");
        assert_eq!(preferences.bandwidth, 180);
        assert_eq!(preferences.additional["theme"], "dark");
        assert_eq!(preferences.version, CURRENT_PREFERENCES_VERSION);
        let saved = postkit::preferences::read_preferences_file(&path)
            .unwrap()
            .unwrap();
        assert_eq!(
            postkit::preferences::prefs_version(&saved),
            CURRENT_PREFERENCES_VERSION
        );
        assert!(saved.contains("gpuRegistrationUrl"));
        assert!(saved.contains("theme"));
    }

    #[test]
    fn newer_file_is_not_rewritten() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        let contents = r#"{"version":99,"creator":"Future","futureField":true}"#;
        postkit::preferences::write_preferences_file(&path, contents).unwrap();

        let preferences = load_preferences_from(&path).unwrap().unwrap();

        assert_eq!(preferences.version, 99);
        assert_eq!(
            postkit::preferences::read_preferences_file(&path)
                .unwrap()
                .unwrap(),
            contents
        );
        assert!(save_preferences_to(&preferences, &path).is_err());
    }
}
