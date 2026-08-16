pub mod aaf_import;
pub mod accessibility;
pub mod assemble;
pub mod assetmap;
pub mod audio_adjust;
pub mod audio_map;
pub mod audio_route;
pub mod burnin;
pub mod cert_fetch;
pub mod certificate;
pub mod cinema;
pub mod combine;
pub mod conform;
pub mod copy_drive;
pub mod cpl;
pub mod cpl_annotation;
pub mod dashboard;
pub mod dcdm;
pub mod dcp;
pub mod decrypt;
pub mod disk;
pub mod dolby_vision;
pub mod edit;
pub mod edl_import;
pub mod email;
pub mod encode;
pub mod encode_qol;
pub mod encrypt;
pub mod export;
pub mod flm;
pub mod free_space;
pub mod grok;
pub mod hash;
pub mod hdr;
pub mod hfr;
pub mod import;
pub mod info;
pub mod ingest;
pub mod ingest_package;
pub mod isdcf_name;
pub mod j2k_transcode;
pub mod job_queue;
pub mod kdm;
pub mod kdm_log;
pub mod kdm_template;
pub mod loudness;
pub mod markers;
pub mod mca;
pub mod multi_cpl;
pub mod mxf_wrap;
pub mod otioz_import;
pub mod package_signature;
pub mod pad;
pub mod pkl;
pub mod preview;
pub mod probe;
pub mod profiles;
pub mod qc;
pub mod reel;
pub mod report;
pub mod rest_api;
pub mod shell_completion;
pub mod sign_language;
pub mod source_picture;
pub mod still;
pub mod store;
pub mod subtitle;
pub mod subtitle_edit;
pub mod subtitle_extract;
pub mod subtitle_retime;
#[cfg(test)]
pub(crate) mod test_wav;
pub mod trailer;
pub mod transcode;
pub mod trim;
pub mod verify;
pub mod version_tracker;
pub mod versions;
pub mod vf;
pub mod watch;
pub mod watermark;
pub mod webhook;

use serde::{Deserialize, Serialize};

/// DCP standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Standard {
    #[default]
    Smpte,
    Interop,
}

/// Resolution preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Resolution {
    #[default]
    TwoK,
    FourK,
}

impl Resolution {
    pub fn width(&self) -> u32 {
        match self {
            Resolution::TwoK => 2048,
            Resolution::FourK => 4096,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Resolution::TwoK => 1080,
            Resolution::FourK => 2160,
        }
    }
}

/// DCP content type (SMPTE 429-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContentType {
    /// Feature
    #[default]
    Feature,
    /// Short
    Short,
    /// Trailer
    Trailer,
    /// Test
    Test,
    /// Transitional (pre-show)
    Transitional,
    /// Rating
    Rating,
    /// Teaser
    Teaser,
    /// Policy
    Policy,
    /// Public service announcement
    PublicServiceAnnouncement,
    /// Advertisement
    Advertisement,
    /// Episode (episodic content)
    Episode,
}

impl ContentType {
    /// Parse from common abbreviation string.
    pub fn from_abbrev(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "FTR" => Some(Self::Feature),
            "SHR" => Some(Self::Short),
            "TLR" => Some(Self::Trailer),
            "TST" => Some(Self::Test),
            "XSN" => Some(Self::Transitional),
            "RTG" => Some(Self::Rating),
            "TSR" => Some(Self::Teaser),
            "POL" => Some(Self::Policy),
            "PSA" => Some(Self::PublicServiceAnnouncement),
            "ADV" => Some(Self::Advertisement),
            "EPS" => Some(Self::Episode),
            _ => None,
        }
    }

    /// SMPTE content kind string for CPL.
    pub fn as_cpl_kind(&self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Short => "short",
            Self::Trailer => "trailer",
            Self::Test => "test",
            Self::Transitional => "transitional",
            Self::Rating => "rating",
            Self::Teaser => "teaser",
            Self::Policy => "policy",
            Self::PublicServiceAnnouncement => "psa",
            Self::Advertisement => "advertisement",
            Self::Episode => "episode",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_type_from_abbrev() {
        assert_eq!(ContentType::from_abbrev("FTR"), Some(ContentType::Feature));
        assert_eq!(ContentType::from_abbrev("SHR"), Some(ContentType::Short));
        assert_eq!(ContentType::from_abbrev("TLR"), Some(ContentType::Trailer));
        assert_eq!(ContentType::from_abbrev("TST"), Some(ContentType::Test));
        assert_eq!(
            ContentType::from_abbrev("XSN"),
            Some(ContentType::Transitional)
        );
        assert_eq!(ContentType::from_abbrev("RTG"), Some(ContentType::Rating));
        assert_eq!(ContentType::from_abbrev("TSR"), Some(ContentType::Teaser));
        assert_eq!(ContentType::from_abbrev("POL"), Some(ContentType::Policy));
        assert_eq!(
            ContentType::from_abbrev("PSA"),
            Some(ContentType::PublicServiceAnnouncement)
        );
        assert_eq!(
            ContentType::from_abbrev("ADV"),
            Some(ContentType::Advertisement)
        );
    }

    #[test]
    fn test_content_type_case_insensitive() {
        assert_eq!(ContentType::from_abbrev("ftr"), Some(ContentType::Feature));
        assert_eq!(ContentType::from_abbrev("Tlr"), Some(ContentType::Trailer));
    }

    #[test]
    fn test_content_type_invalid() {
        assert_eq!(ContentType::from_abbrev("XYZ"), None);
        assert_eq!(ContentType::from_abbrev(""), None);
    }

    #[test]
    fn test_content_type_cpl_kind() {
        assert_eq!(ContentType::Feature.as_cpl_kind(), "feature");
        assert_eq!(ContentType::Trailer.as_cpl_kind(), "trailer");
        assert_eq!(ContentType::PublicServiceAnnouncement.as_cpl_kind(), "psa");
    }

    #[test]
    fn test_resolution_dimensions() {
        assert_eq!(Resolution::TwoK.width(), 2048);
        assert_eq!(Resolution::TwoK.height(), 1080);
        assert_eq!(Resolution::FourK.width(), 4096);
        assert_eq!(Resolution::FourK.height(), 2160);
    }
}
