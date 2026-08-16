//! The ISDCF content title for a `DcpConfig`: the one place the packaging
//! configuration is mapped onto [`crate::isdcf_name`]'s input, shared by the CLI
//! and the GUI so both name a package identically.

use crate::dcp::DcpConfig;
use crate::isdcf_name::{
    DEFAULT_CONTAINER_SIZE, IsdcfDate, IsdcfNameInput, SoundtrackChannel, TerritoryType, TextKind,
    isdcf_name,
};
use chrono::Datelike;

/// Composition version number when the config carries none, as Bv2.1 requires
/// the element present.
const DEFAULT_VERSION_NUMBER: u32 = 1;

/// The parts of the name that are not packaging configuration: who mastered it,
/// what kind of version it is, and when.
#[derive(Debug, Clone, Default)]
pub struct IsdcfNamingOptions {
    pub studio: Option<String>,
    pub temp_version: bool,
    pub pre_release: bool,
    pub red_band: bool,
    pub two_d_version_of_three_d: bool,
    pub territory_type: TerritoryType,
    /// Creation date in the name. None is today, UTC.
    pub date: Option<IsdcfDate>,
    pub version_file: bool,
}

/// The packaged soundtrack as the name counts it.
#[derive(Debug, Clone, Default)]
pub struct SoundtrackSummary {
    pub channels: Vec<SoundtrackChannel>,
    pub has_hearing_impaired: bool,
    pub has_visually_impaired: bool,
}

/// The soundtrack the packaged channel count carries, read the way the CPL's
/// `main_sound_configuration` reads it: HI and VI sit outside the main layout,
/// and a count with no canonical DCP layout carries no channels at all.
pub fn soundtrack_summary(
    channel_count: usize,
    hi_channel: Option<u32>,
    vi_channel: Option<u32>,
) -> SoundtrackSummary {
    let accessibility = hi_channel.is_some() as usize + vi_channel.is_some() as usize;
    let main_count = channel_count.saturating_sub(accessibility);
    let channels = match main_count {
        2 => vec![SoundtrackChannel::Left, SoundtrackChannel::Right],
        6 | 16 => vec![
            SoundtrackChannel::Left,
            SoundtrackChannel::Right,
            SoundtrackChannel::Centre,
            SoundtrackChannel::Lfe,
            SoundtrackChannel::LeftSurround,
            SoundtrackChannel::RightSurround,
        ],
        8 => vec![
            SoundtrackChannel::Left,
            SoundtrackChannel::Right,
            SoundtrackChannel::Centre,
            SoundtrackChannel::Lfe,
            SoundtrackChannel::LeftSurround,
            SoundtrackChannel::RightSurround,
            SoundtrackChannel::BackSurroundLeft,
            SoundtrackChannel::BackSurroundRight,
        ],
        _ => Vec::new(),
    };
    SoundtrackSummary {
        channels,
        has_hearing_impaired: hi_channel.is_some(),
        has_visually_impaired: vi_channel.is_some(),
    }
}

/// The ISDCF content title for a package. `burnt_in_subtitle` says the subtitles
/// are drawn into the picture, which the name spells in lower case.
pub fn isdcf_title(
    config: &DcpConfig,
    options: &IsdcfNamingOptions,
    sound: &SoundtrackSummary,
    burnt_in_subtitle: bool,
) -> String {
    let (open_text_languages, open_text_burnt_in) = open_text(config, burnt_in_subtitle);
    let closed_text_languages = match config.ccap_path {
        Some(_) => vec![config.ccap_language.clone()],
        None => Vec::new(),
    };

    let input = IsdcfNameInput {
        title: config.title.clone(),
        content_type: config.content_type,
        version_number: config.version_number.unwrap_or(DEFAULT_VERSION_NUMBER),
        content_versions: config.content_versions.clone(),
        temp_version: options.temp_version,
        pre_release: options.pre_release,
        red_band: options.red_band,
        chain: config.chain.clone(),
        three_d: config.stereo_3d,
        two_d_version_of_three_d: options.two_d_version_of_three_d,
        luminance: config.luminance.clone(),
        frame_rate: rounded_frame_rate(config.frame_rate_num, config.frame_rate_den),
        container_size: container_size(config),
        // the CPL declares the stored area as the active one, so the name has no
        // interior aspect to spell
        active_picture_size: None,
        audio_language: config.audio_language.clone(),
        open_text_languages,
        open_text_kind: TextKind::Subtitle,
        open_text_burnt_in,
        closed_text_languages,
        closed_text_kind: TextKind::Caption,
        territory_type: options.territory_type,
        release_territory: config.release_territory.clone(),
        ratings: config.ratings.clone(),
        soundtrack_channels: sound.channels.clone(),
        has_hearing_impaired: sound.has_hearing_impaired,
        has_visually_impaired: sound.has_visually_impaired,
        has_atmos: config.atmos_path.is_some(),
        resolution: config.resolution,
        studio: options.studio.clone(),
        date: Some(options.date.unwrap_or_else(today)),
        facility: config.facility.clone(),
        standard: config.standard,
        version_file: options.version_file,
    };

    isdcf_name(&input)
}

/// The container whose aspect the name spells. Without one the CPL declares the
/// coded raster as the active area, so the raster is the container.
fn container_size(config: &DcpConfig) -> (u32, u32) {
    if config.container_width > 0 && config.container_height > 0 {
        return (config.container_width, config.container_height);
    }
    config
        .j2k_dir
        .as_deref()
        .and_then(|dir| crate::cpl::picture_geometry(dir, 0, 0).ok())
        .map(|geometry| (geometry.stored_width, geometry.stored_height))
        .unwrap_or(DEFAULT_CONTAINER_SIZE)
}

/// The open text languages and whether they are burnt in. A registered subtitle
/// track wins over a burn, since the name spells the track the package carries.
fn open_text(config: &DcpConfig, burnt_in_subtitle: bool) -> (Vec<String>, bool) {
    if config.subtitle_path.is_some() {
        return (vec![config.subtitle_language.clone()], false);
    }
    if burnt_in_subtitle {
        return (vec![config.subtitle_language.clone()], true);
    }
    (Vec::new(), false)
}

fn rounded_frame_rate(numerator: u32, denominator: u32) -> u32 {
    if denominator == 0 {
        return numerator;
    }
    (numerator as f64 / denominator as f64).round() as u32
}

fn today() -> IsdcfDate {
    let now = chrono::Utc::now().date_naive();
    IsdcfDate {
        year: now.year() as u32,
        month: now.month(),
        day: now.day(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isdcf_name::Rating;
    use crate::{ContentType, Resolution, Standard};
    use std::path::PathBuf;

    const DATE: IsdcfDate = IsdcfDate {
        year: 2026,
        month: 8,
        day: 16,
    };

    fn config() -> DcpConfig {
        DcpConfig {
            title: "My Film".into(),
            standard: Standard::Smpte,
            resolution: Resolution::TwoK,
            content_type: ContentType::Test,
            frame_rate_num: 24,
            frame_rate_den: 1,
            container_width: 1998,
            container_height: 1080,
            audio_language: Some("en".into()),
            facility: Some("PPF".into()),
            ..Default::default()
        }
    }

    fn options() -> IsdcfNamingOptions {
        IsdcfNamingOptions {
            date: Some(DATE),
            ..Default::default()
        }
    }

    fn stereo() -> SoundtrackSummary {
        soundtrack_summary(2, None, None)
    }

    #[test]
    fn a_stereo_package_is_named_from_its_configuration() {
        assert_eq!(
            isdcf_title(&config(), &options(), &stereo(), false),
            "MyFilm_TST-1_F_EN-XX_20_2K_20260816_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn the_channel_ladder_follows_the_packaged_count() {
        let ladder = [(2, "_20"), (6, "_51"), (16, "_51"), (8, "_71"), (3, "_MOS")];
        for (channel_count, expected) in ladder {
            let sound = soundtrack_summary(channel_count, None, None);
            let name = isdcf_title(&config(), &options(), &sound, false);
            assert!(
                name.contains(expected),
                "{channel_count} channels must name {expected}, got {name}"
            );
        }
    }

    #[test]
    fn accessibility_channels_sit_outside_the_main_layout() {
        let sound = soundtrack_summary(8, Some(6), Some(7));
        assert_eq!(sound.channels.len(), 6, "eight channels less HI and VI");
        let name = isdcf_title(&config(), &options(), &sound, false);
        assert!(name.contains("_51-HI-VI"), "{name}");
    }

    #[test]
    fn an_open_subtitle_is_upper_case_and_a_burnt_in_one_lower() {
        let open = DcpConfig {
            subtitle_path: Some(PathBuf::from("subs.srt")),
            subtitle_language: "fr".into(),
            ..config()
        };
        assert!(
            isdcf_title(&open, &options(), &stereo(), false).contains("_EN-FR_"),
            "an open subtitle track is spelled in upper case"
        );

        let burnt = DcpConfig {
            subtitle_language: "fr".into(),
            ..config()
        };
        assert!(
            isdcf_title(&burnt, &options(), &stereo(), true).contains("_EN-fr_"),
            "burnt-in subtitles are spelled in lower case"
        );
    }

    #[test]
    fn a_closed_caption_track_is_marked_ccap() {
        let captioned = DcpConfig {
            ccap_path: Some(PathBuf::from("captions.srt")),
            ccap_language: "de".into(),
            ..config()
        };
        assert!(
            isdcf_title(&captioned, &options(), &stereo(), false).contains("_EN-DE-CCAP_"),
            "a closed-caption track is marked CCAP"
        );
    }

    #[test]
    fn a_stereoscopic_package_is_marked_3d_twice() {
        let three_d = DcpConfig {
            stereo_3d: true,
            ..config()
        };
        let name = isdcf_title(&three_d, &options(), &stereo(), false);
        assert!(name.contains("_TST-1-3D_"), "{name}");
        assert!(name.ends_with("_SMPTE-3D_OV"), "{name}");
    }

    #[test]
    fn no_date_names_today() {
        let options = IsdcfNamingOptions::default();
        let name = isdcf_title(&config(), &options, &stereo(), false);
        let today = today();
        assert!(
            name.contains(&format!(
                "_{:04}{:02}{:02}_",
                today.year, today.month, today.day
            )),
            "{name}"
        );
    }

    #[test]
    fn no_container_takes_the_aspect_from_the_coded_raster() {
        let dir = tempfile::tempdir().unwrap();
        let frames = dir.path().join("frames");
        std::fs::create_dir_all(&frames).unwrap();
        crate::pad::generate_black_frame(2048, 858, 24, &frames.join("frame_00000.j2c"))
            .expect("encode frame");

        let config = DcpConfig {
            container_width: 0,
            container_height: 0,
            j2k_dir: Some(frames),
            ..config()
        };
        let name = isdcf_title(&config, &options(), &stereo(), false);
        assert!(name.contains("_S_"), "2048x858 frames are scope: {name}");
    }

    #[test]
    fn no_container_and_no_frames_fall_back_to_flat() {
        let config = DcpConfig {
            container_width: 0,
            container_height: 0,
            ..config()
        };
        let name = isdcf_title(&config, &options(), &stereo(), false);
        assert!(name.contains("_F_"), "{name}");
    }

    #[test]
    fn a_version_file_is_named_vf() {
        let options = IsdcfNamingOptions {
            version_file: true,
            ..options()
        };
        assert!(isdcf_title(&config(), &options, &stereo(), false).ends_with("_VF"));
    }

    /// The mapping is pinned against the name built by hand from the same facts.
    #[test]
    fn the_mapping_matches_a_hand_built_name() {
        let config = DcpConfig {
            title: "My Nice Film".into(),
            content_type: ContentType::Feature,
            version_number: Some(2),
            content_versions: vec!["Final Cut".into()],
            release_territory: Some("GB".into()),
            ratings: vec![Rating {
                agency: "http://www.bbfc.co.uk/BBFCRatings".into(),
                label: "PG".into(),
            }],
            chain: Some("MyChain".into()),
            atmos_path: Some(PathBuf::from("atmos.mxf")),
            ccap_path: Some(PathBuf::from("captions.srt")),
            ccap_language: "fr".into(),
            frame_rate_num: 48,
            frame_rate_den: 1,
            container_width: 2048,
            container_height: 858,
            resolution: Resolution::FourK,
            ..config()
        };
        let options = IsdcfNamingOptions {
            studio: Some("Disney".into()),
            temp_version: true,
            ..options()
        };
        let sound = soundtrack_summary(6, None, None);

        let expected = isdcf_name(&IsdcfNameInput {
            title: "My Nice Film".into(),
            content_type: ContentType::Feature,
            version_number: 2,
            content_versions: vec!["Final Cut".into()],
            temp_version: true,
            chain: Some("MyChain".into()),
            frame_rate: 48,
            container_size: (2048, 858),
            audio_language: Some("en".into()),
            closed_text_languages: vec!["fr".into()],
            closed_text_kind: TextKind::Caption,
            release_territory: Some("GB".into()),
            ratings: vec![Rating {
                agency: "http://www.bbfc.co.uk/BBFCRatings".into(),
                label: "PG".into(),
            }],
            soundtrack_channels: sound.channels.clone(),
            has_atmos: true,
            resolution: Resolution::FourK,
            studio: Some("Disney".into()),
            date: Some(DATE),
            facility: Some("PPF".into()),
            standard: Standard::Smpte,
            ..Default::default()
        });

        assert_eq!(isdcf_title(&config, &options, &sound, false), expected);
        assert_eq!(
            expected,
            "MyNiceFilm_FTR-2-Temp-MyChain-48_S_EN-FR-CCAP_GB-PG_51-IAB_4K_DISN_20260816_PPF_SMPTE_OV"
        );
    }
}
