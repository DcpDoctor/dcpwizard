//! DCP creation profiles (cinema 2K/4K, trailer, advertisement).
//!
//! [`postkit::profiles`] describes delivery-platform encoding targets (Netflix,
//! Apple, etc.); these are DCP packaging presets consumed by `create --profile`,
//! a different concept, so they stay local.

use serde::{Deserialize, Serialize};

/// Encode target in Mbit/s for a full-quality DCP. Under DCI's 250 on purpose:
/// rate allocation lands a frame either side of the target, so 250 fails the
/// peak bitrate check.
pub const FULL_QUALITY_MBPS: u32 = 230;

/// Encode target in Mbit/s for content that does not need the full rate.
const REDUCED_QUALITY_MBPS: u32 = 200;

/// DCI base frame rate, and the only rate these presets ship at.
const STANDARD_FRAME_RATE: u32 = 24;

/// DCI audio sample rate.
const AUDIO_SAMPLE_RATE: u32 = 48_000;

/// A DCP creation profile with preset settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub description: String,
    pub standard: String,
    pub resolution_width: u32,
    pub resolution_height: u32,
    pub frame_rate: u32,
    pub bitrate_mbps: u32,
    pub audio_channels: u32,
    pub audio_sample_rate: u32,
    pub content_kind: String,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "cinema_2k".into(),
            description: "Standard 2K cinema DCP".into(),
            standard: "SMPTE".into(),
            resolution_width: 2048,
            resolution_height: 1080,
            frame_rate: STANDARD_FRAME_RATE,
            bitrate_mbps: FULL_QUALITY_MBPS,
            audio_channels: 6,
            audio_sample_rate: AUDIO_SAMPLE_RATE,
            content_kind: "feature".into(),
        }
    }
}

/// Get a profile by name.
pub fn get_profile(name: &str) -> Option<Profile> {
    let profiles = all_profiles();
    profiles.into_iter().find(|p| p.name == name)
}

/// Return all built-in profiles.
pub fn all_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "cinema_2k".into(),
            description: "Standard 2K cinema DCP (Flat/Scope)".into(),
            standard: "SMPTE".into(),
            resolution_width: 2048,
            resolution_height: 1080,
            frame_rate: STANDARD_FRAME_RATE,
            bitrate_mbps: FULL_QUALITY_MBPS,
            audio_channels: 6,
            audio_sample_rate: AUDIO_SAMPLE_RATE,
            content_kind: "feature".into(),
        },
        Profile {
            name: "cinema_4k".into(),
            description: "4K cinema DCP".into(),
            standard: "SMPTE".into(),
            resolution_width: 4096,
            resolution_height: 2160,
            frame_rate: STANDARD_FRAME_RATE,
            bitrate_mbps: 500,
            audio_channels: 6,
            audio_sample_rate: AUDIO_SAMPLE_RATE,
            content_kind: "feature".into(),
        },
        Profile {
            name: "trailer".into(),
            description: "Cinema trailer DCP".into(),
            standard: "SMPTE".into(),
            resolution_width: 2048,
            resolution_height: 858,
            frame_rate: STANDARD_FRAME_RATE,
            bitrate_mbps: FULL_QUALITY_MBPS,
            audio_channels: 6,
            audio_sample_rate: AUDIO_SAMPLE_RATE,
            content_kind: "trailer".into(),
        },
        Profile {
            name: "advertisement".into(),
            description: "Cinema advertisement DCP".into(),
            standard: "SMPTE".into(),
            resolution_width: 2048,
            resolution_height: 1080,
            frame_rate: STANDARD_FRAME_RATE,
            bitrate_mbps: REDUCED_QUALITY_MBPS,
            audio_channels: 2,
            audio_sample_rate: AUDIO_SAMPLE_RATE,
            content_kind: "advertisement".into(),
        },
    ]
}
