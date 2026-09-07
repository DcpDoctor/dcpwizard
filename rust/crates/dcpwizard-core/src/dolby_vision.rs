//! Dolby Vision and HDR metadata handling for DCP/IMF workflows.
//!
//! Delegates to [`postkit::dolby_vision`] for HDR detection, metadata injection,
//! and format conversion.

pub use postkit::dolby_vision::{
    DolbyVisionOptions, DolbyVisionProfile, Hdr10Metadata, HdrMetadataOptions, HdrType,
    convert_hdr, detect_hdr_type, read_hdr10_metadata,
};

use std::path::Path;

/// Where dovi_tool comes from, named in the refusal when it is not installed.
const DOVI_TOOL_URL: &str = "https://github.com/quietvoid/dovi_tool";

fn require_file(label: &str, path: &Path) -> Result<(), String> {
    if path.is_file() {
        return Ok(());
    }
    Err(format!("{label} not found: {}", path.display()))
}

fn require_on_path(binary: &str) -> Result<(), String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path).any(|dir| dir.join(binary).is_file());
    if found {
        return Ok(());
    }
    Err(format!(
        "{binary} is not installed or not on PATH, get it from {DOVI_TOOL_URL}"
    ))
}

/// Inject a Dolby Vision RPU, refusing before the tool runs when an input or
/// dovi_tool itself is missing.
pub fn inject_dolby_vision(options: &DolbyVisionOptions) -> i32 {
    let checks = require_file("input file", &options.input)
        .and_then(|()| require_file("RPU file", &options.rpu_file))
        .and_then(|()| require_on_path("dovi_tool"));
    if let Err(e) = checks {
        tracing::error!("{e}");
        return 1;
    }
    postkit::dolby_vision::inject_dolby_vision(options)
}

/// Inject HDR10 static metadata, refusing before ffmpeg runs when the input is
/// missing.
pub fn inject_hdr10_metadata(options: &HdrMetadataOptions) -> i32 {
    if let Err(e) = require_file("input file", &options.input) {
        tracing::error!("{e}");
        return 1;
    }
    postkit::dolby_vision::inject_hdr10_metadata(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hdr_type_default() {
        assert_eq!(HdrType::default(), HdrType::Sdr);
    }

    #[test]
    fn test_dolby_vision_profile_default() {
        assert_eq!(DolbyVisionProfile::default(), DolbyVisionProfile::Profile81);
    }

    #[test]
    fn test_hdr10_metadata_default() {
        let meta = Hdr10Metadata::default();
        assert_eq!(meta.max_luminance, 0);
        assert_eq!(meta.max_cll, 0);
        assert_eq!(meta.max_fall, 0);
    }

    #[test]
    fn test_detect_hdr_missing_file() {
        let hdr = detect_hdr_type(std::path::Path::new("/nonexistent.mxf"));
        assert_eq!(hdr, HdrType::Sdr);
    }
}
