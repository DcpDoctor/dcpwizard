//! DCI HDR Addendum (v1.2.1) signaling.
//!
//! The addendum (s7) requires the picture MXF's Generic Picture Essence
//! Descriptor to carry TransferCharacteristic = ST 2084 (the UL below) plus a CPL
//! ExtensionMetadata EOTF="ST 2084" claim. The descriptor side is written by
//! `mxf_wrap::wrap_j2k_hdr_files`, so this module holds the numbers both the CLI
//! and the GUI validate against before an HDR encode starts: the raised
//! per-codestream byte cap and the bitrate ceiling it comes from.

use postkit::colour::HdrSource;
use postkit::dolby_vision::DolbyVisionSummary;
use postkit::encode::SourceColour;
use std::path::Path;

/// ST 2084 (PQ) TransferCharacteristic UL (DCI HDR Addendum s7; asdcplib
/// TransferCharacteristic_SMPTEST2084).
pub const ST2084_TRANSFER_UL: [u8; 16] = [
    0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x0d, 0x04, 0x01, 0x01, 0x01, 0x01, 0x0a, 0x00, 0x00,
];

/// DCI HDR Addendum monoscopic per-codestream byte cap: floor(56,250,000 / R)
/// bytes per frame at edit rate R fps (= 450 Mbit/s). Stereoscopic halves it.
pub fn hdr_codestream_byte_cap(edit_rate: u32) -> u64 {
    56_250_000 / edit_rate.max(1) as u64
}

/// The DCI HDR bitrate ceiling in Mbit/s (constant 450 across edit rates: the
/// per-frame byte cap times fps times 8 bits is always 450 Mbit/s).
pub const HDR_MAX_MBPS: u32 = 450;

const DOLBY_VISION_NAME: &str = "dolby-vision";
const HDR10_PLUS_NAME: &str = "hdr10plus";

// ffprobe's name for the ST 2094-40 dynamic metadata an HDR10+ master carries
const HDR10_PLUS_SIDE_DATA: &str = "SMPTE2094-40";

// ffprobe's spellings of the colour tags an HDR master carries
const PQ_TRANSFER_TAG: &str = "smpte2084";
const HLG_TRANSFER_TAG: &str = "arib-std-b67";
const BT2020_PRIMARIES_TAG: &str = "bt2020";
const P3D65_PRIMARIES_TAG: &str = "smpte432";

// ST 2086 mastering display luminance is carried in 0.0001 cd/m² steps
const MASTERING_DISPLAY_LUMINANCE_STEPS_PER_NIT: f32 = 10_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrSourceFormat {
    Hdr10,
    Hdr10Plus,
    Hlg,
    PqP3D65,
    DolbyVision,
}

impl HdrSourceFormat {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            DOLBY_VISION_NAME => return Some(Self::DolbyVision),
            HDR10_PLUS_NAME => return Some(Self::Hdr10Plus),
            _ => {}
        }
        match HdrSource::parse(name)? {
            HdrSource::Hdr10 => Some(Self::Hdr10),
            HdrSource::Hlg => Some(Self::Hlg),
            HdrSource::PqP3D65 => Some(Self::PqP3D65),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Hdr10 => "hdr10",
            Self::Hdr10Plus => HDR10_PLUS_NAME,
            Self::Hlg => "hlg",
            Self::PqP3D65 => "pq-p3d65",
            Self::DolbyVision => DOLBY_VISION_NAME,
        }
    }
}

// a --hdr-dci master with no --hdr-source is read from its own colour tags
pub fn plan_hdr_dcdm(
    video: &Path,
    format: Option<HdrSourceFormat>,
    peak_nits: Option<f32>,
) -> Result<SourceColour, String> {
    let format = match format {
        Some(format) => format,
        None => detect_hdr_source_format(video)?,
    };
    let (source, dolby_vision) = match format {
        // HDR10+ adds per scene metadata over an HDR10 grade, and the addendum
        // transform reads that grade
        HdrSourceFormat::Hdr10 | HdrSourceFormat::Hdr10Plus => (HdrSource::Hdr10, None),
        HdrSourceFormat::Hlg => (HdrSource::Hlg, None),
        HdrSourceFormat::PqP3D65 => (HdrSource::PqP3D65, None),
        HdrSourceFormat::DolbyVision => {
            let summary = read_dolby_vision_master(video)?;
            (hdr_source_of_dolby_vision(&summary)?, Some(summary))
        }
    };
    let source_peak_nits = resolve_peak_nits(peak_nits, dolby_vision.as_ref(), video);
    tracing::info!(
        "DCI HDR: {} master, {source:?} grade rolled off from {source_peak_nits} cd/m²",
        format.name()
    );
    Ok(SourceColour::HdrDcdm {
        source,
        source_peak_nits,
    })
}

// profile 8.1's base layer is an HDR10 grade the addendum transform reads
pub fn hdr_source_of_dolby_vision(summary: &DolbyVisionSummary) -> Result<HdrSource, String> {
    postkit::dolby_vision::refuse_undecodable_dolby_vision(summary)?;
    Ok(HdrSource::Hdr10)
}

fn read_dolby_vision_master(video: &Path) -> Result<DolbyVisionSummary, String> {
    postkit::dolby_vision::read_dolby_vision(video)?.ok_or_else(|| {
        format!(
            "--hdr-source dolby-vision, but {} carries no Dolby Vision RPU",
            video.display()
        )
    })
}

pub fn detect_hdr_source_format(video: &Path) -> Result<HdrSourceFormat, String> {
    if dolby_vision_rpu_present(video) {
        return Ok(HdrSourceFormat::DolbyVision);
    }
    if hdr10_plus_metadata_present(video) {
        return Ok(HdrSourceFormat::Hdr10Plus);
    }
    let (transfer, primaries) = colour_tags(video)?;
    match (transfer.as_str(), primaries.as_str()) {
        (PQ_TRANSFER_TAG, BT2020_PRIMARIES_TAG) => Ok(HdrSourceFormat::Hdr10),
        (PQ_TRANSFER_TAG, P3D65_PRIMARIES_TAG) => Ok(HdrSourceFormat::PqP3D65),
        (HLG_TRANSFER_TAG, _) => Ok(HdrSourceFormat::Hlg),
        _ => Err(format!(
            "{} is tagged with the {transfer} transfer and {primaries} primaries, which names no \
             HDR grade --hdr-dci can transform: name the master with \
             --hdr-source <hdr10|hdr10plus|hlg|pq-p3d65|dolby-vision>",
            video.display()
        )),
    }
}

fn hdr10_plus_metadata_present(video: &Path) -> bool {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_frames",
            "-read_intervals",
            "%+#1",
            "-show_entries",
            "frame=side_data_list",
            "-of",
            "json",
        ])
        .arg(video)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).contains(HDR10_PLUS_SIDE_DATA)
        }
        Ok(output) => {
            tracing::warn!(
                "ffprobe could not read the dynamic metadata of {}: {}",
                video.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            false
        }
        Err(e) => {
            tracing::warn!("failed to run ffprobe: {e}");
            false
        }
    }
}

// a master too big for postkit's RPU read cap still has its colour tags to go on
fn dolby_vision_rpu_present(video: &Path) -> bool {
    match postkit::dolby_vision::read_dolby_vision(video) {
        Ok(summary) => summary.is_some(),
        Err(e) => {
            tracing::warn!(
                "reading the Dolby Vision RPU of {} failed: {e}",
                video.display()
            );
            false
        }
    }
}

fn colour_tags(video: &Path) -> Result<(String, String), String> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=color_transfer,color_primaries",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(video)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffprobe could not read the colour tags of {}: {}",
            video.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut transfer = String::from("unknown");
    let mut primaries = String::from("unknown");
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let field = match key.trim() {
            "color_transfer" => &mut transfer,
            "color_primaries" => &mut primaries,
            _ => continue,
        };
        *field = value.trim().to_lowercase();
    }
    Ok((transfer, primaries))
}

fn resolve_peak_nits(
    explicit: Option<f32>,
    dolby_vision: Option<&DolbyVisionSummary>,
    video: &Path,
) -> f32 {
    if let Some(nits) = explicit {
        return nits;
    }
    let probe = postkit::dolby_vision::read_hdr10_metadata(video);
    let content_light = dolby_vision
        .and_then(|summary| summary.max_content_light_level_nits)
        .or_else(|| positive_nits(f32::from(probe.max_cll)));
    let mastering_display = dolby_vision
        .and_then(|summary| summary.mastering_display_max_nits)
        .or_else(|| {
            positive_nits(probe.max_luminance as f32 / MASTERING_DISPLAY_LUMINANCE_STEPS_PER_NIT)
        });
    content_light.or(mastering_display).unwrap_or_else(|| {
        tracing::warn!(
            "{} names neither a MaxCLL nor a mastering display maximum, so the DCI HDR roll-off \
             starts from {} cd/m²: pass --hdr-peak-nits to name the grade's own peak",
            video.display(),
            HdrSource::DEFAULT_PEAK_NITS
        );
        HdrSource::DEFAULT_PEAK_NITS
    })
}

fn positive_nits(nits: f32) -> Option<f32> {
    (nits > 0.0).then_some(nits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_cap_is_floor_of_450mbit() {
        assert_eq!(hdr_codestream_byte_cap(24), 2_343_750);
        assert_eq!(hdr_codestream_byte_cap(25), 2_250_000);
        assert_eq!(hdr_codestream_byte_cap(48), 1_171_875);
        // cap * fps * 8 bits stays at the 450 Mbit/s ceiling
        assert_eq!(hdr_codestream_byte_cap(24) * 24 * 8 / 1_000_000, 450);
    }

    #[test]
    fn every_hdr_source_flag_value_parses_and_a_typo_does_not() {
        for format in [
            HdrSourceFormat::Hdr10,
            HdrSourceFormat::Hlg,
            HdrSourceFormat::PqP3D65,
            HdrSourceFormat::DolbyVision,
        ] {
            assert_eq!(HdrSourceFormat::parse(format.name()), Some(format));
        }
        assert_eq!(
            HdrSourceFormat::parse(" Dolby-Vision "),
            Some(HdrSourceFormat::DolbyVision)
        );
        for name in ["", "hdr", "dolbyvision", "rec709"] {
            assert_eq!(HdrSourceFormat::parse(name), None, "{name}");
        }
    }

    fn dolby_vision_summary(profile: u8) -> DolbyVisionSummary {
        DolbyVisionSummary {
            profile,
            frames: 2,
            shots: 1,
            max_content_light_level_nits: None,
            max_frame_average_light_level_nits: None,
            peak_luminance_nits: 1000.0,
            mastering_display_max_nits: None,
            mastering_display_min_nits: None,
        }
    }

    #[test]
    fn a_profile_81_master_transforms_as_hdr10_and_profile_5_is_refused_by_name() {
        assert_eq!(
            hdr_source_of_dolby_vision(&dolby_vision_summary(8)).unwrap(),
            HdrSource::Hdr10
        );
        let refusal = hdr_source_of_dolby_vision(&dolby_vision_summary(5)).unwrap_err();
        assert!(refusal.contains("profile 5"), "{refusal}");
    }

    // the matroska muxer writes the tags the frames carry, not the output options
    fn tagged_clip(dir: &Path, name: &str, setparams: Option<&str>) -> std::path::PathBuf {
        let clip = dir.join(name);
        let mut command = std::process::Command::new("ffmpeg");
        command
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("color=c=black:s=64x64:r=24:d=0.084");
        if let Some(filter) = setparams {
            command.args(["-vf", filter]);
        }
        let status = command
            .args(["-pix_fmt", "yuv420p10le", "-c:v", "ffv1"])
            .arg(&clip)
            .status()
            .expect("ffmpeg has to run");
        assert!(
            status.success(),
            "ffmpeg could not write {}",
            clip.display()
        );
        clip
    }

    #[test]
    fn the_masters_own_tags_name_the_grade_and_an_sdr_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for (name, setparams, format) in [
            (
                "hdr10.mkv",
                "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc",
                HdrSourceFormat::Hdr10,
            ),
            (
                "hlg.mkv",
                "setparams=color_primaries=bt2020:color_trc=arib-std-b67:colorspace=bt2020nc",
                HdrSourceFormat::Hlg,
            ),
            (
                "pq_p3d65.mkv",
                "setparams=color_primaries=smpte432:color_trc=smpte2084",
                HdrSourceFormat::PqP3D65,
            ),
        ] {
            let clip = tagged_clip(dir.path(), name, Some(setparams));
            assert_eq!(detect_hdr_source_format(&clip).unwrap(), format, "{name}");
        }

        let sdr = tagged_clip(
            dir.path(),
            "sdr.mkv",
            Some("setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709"),
        );
        let refusal = detect_hdr_source_format(&sdr).unwrap_err();
        assert!(refusal.contains("--hdr-source"), "{refusal}");
    }

    const FIXTURE_WIDTH: u32 = 320;
    const FIXTURE_HEIGHT: u32 = 180;
    const FIXTURE_FPS: u32 = 25;
    const HDR10_PLUS_FRAMES: usize = 2;
    // 12 bit PQ code for 600 cd/m², the level 1 peak the fixture RPUs carry
    const PQ_CODE_600_NITS: u16 = 2851;

    fn plain_hevc(directory: &Path, name: &str, frames: usize) -> std::path::PathBuf {
        let output = directory.join(name);
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c=gray:s={FIXTURE_WIDTH}x{FIXTURE_HEIGHT}:r={FIXTURE_FPS}"),
                "-frames:v",
                &frames.to_string(),
                "-pix_fmt",
                "yuv420p10le",
                "-c:v",
                "libx265",
                "-x265-params",
                "log-level=none",
                "-f",
                "hevc",
            ])
            .arg(&output)
            .output()
            .expect("ffmpeg has to run");
        assert!(
            made.status.success(),
            "ffmpeg could not write {name}: {}",
            String::from_utf8_lossy(&made.stderr)
        );
        output
    }

    // one scene of ST 2094-40 metadata, interleaved into the stream as SEI NALs
    fn hdr10_plus_fixture(directory: &Path) -> std::path::PathBuf {
        let frame = |sequence_frame_index: usize| {
            serde_json::json!({
                "BezierCurveData": {
                    "Anchors": [102, 205, 307, 410, 512, 614, 717, 819, 922],
                    "KneePointX": 0,
                    "KneePointY": 0
                },
                "LuminanceParameters": {
                    "AverageRGB": 1024,
                    "LuminanceDistributions": {
                        "DistributionIndex": [1, 5, 10, 25, 50, 75, 90, 95, 99],
                        "DistributionValues": [0, 0, 100, 3, 4, 5, 6, 7, 8]
                    },
                    "MaxScl": [17000, 18000, 19000]
                },
                "NumberOfWindows": 1,
                "TargetedSystemDisplayMaximumLuminance": 500,
                "SceneFrameIndex": sequence_frame_index,
                "SceneId": 0,
                "SequenceFrameIndex": sequence_frame_index
            })
        };
        let metadata = serde_json::json!({
            "JSONInfo": { "HDR10plusProfile": "B", "Version": "1.0" },
            "SceneInfo": (0..HDR10_PLUS_FRAMES).map(frame).collect::<Vec<_>>(),
            "SceneInfoSummary": {
                "SceneFirstFrameIndex": [0],
                "SceneFrameNumbers": [HDR10_PLUS_FRAMES]
            },
            "ToolInfo": { "Tool": "dcpwizard test fixture", "Version": "1.0" }
        });
        let json = directory.join("hdr10plus.json");
        std::fs::write(&json, serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();

        let base_layer = plain_hevc(directory, "base.hevc", HDR10_PLUS_FRAMES);
        let injected = directory.join("hdr10plus.hevc");
        let made = std::process::Command::new("hdr10plus_tool")
            .arg("inject")
            .args(["-i".as_ref(), base_layer.as_os_str()])
            .args(["-j".as_ref(), json.as_os_str()])
            .args(["-o".as_ref(), injected.as_os_str()])
            .output()
            .expect("hdr10plus_tool has to be installed");
        assert!(
            made.status.success(),
            "hdr10plus_tool could not inject: {}",
            String::from_utf8_lossy(&made.stderr)
        );
        injected
    }

    #[test]
    fn a_dolby_vision_master_and_an_hdr10_plus_one_are_named_by_their_metadata() {
        let dir = tempfile::tempdir().unwrap();

        let dolby_vision = postkit::dolby_vision::write_dolby_vision_fixture(
            dir.path(),
            "profile81.hevc",
            postkit::dolby_vision::DolbyVisionFixtureProfile::Profile81,
            None,
            Some(PQ_CODE_600_NITS),
        )
        .expect("the profile 8.1 fixture has to be written");
        assert_eq!(
            detect_hdr_source_format(&dolby_vision).unwrap(),
            HdrSourceFormat::DolbyVision
        );

        let hdr10_plus = hdr10_plus_fixture(dir.path());
        assert_eq!(
            detect_hdr_source_format(&hdr10_plus).unwrap(),
            HdrSourceFormat::Hdr10Plus,
            "the ST 2094-40 metadata names the grade where the base layer carries no colour tags"
        );
    }

    // an HDR10+ master delivers as its HDR10 base layer, the grade the addendum transform reads
    #[test]
    fn an_hdr10_plus_master_plans_as_hdr10() {
        let dir = tempfile::tempdir().unwrap();
        let hdr10_plus = hdr10_plus_fixture(dir.path());
        let planned = plan_hdr_dcdm(&hdr10_plus, None, Some(1000.0)).expect("plan the master");
        assert_eq!(
            planned,
            SourceColour::HdrDcdm {
                source: HdrSource::Hdr10,
                source_peak_nits: 1000.0,
            }
        );
    }

    #[test]
    fn the_peak_luminance_prefers_the_flag_then_maxcll_then_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let untagged = tagged_clip(dir.path(), "untagged.mkv", None);
        assert_eq!(resolve_peak_nits(Some(4000.0), None, &untagged), 4000.0);
        assert_eq!(
            resolve_peak_nits(None, None, &untagged),
            HdrSource::DEFAULT_PEAK_NITS
        );

        let mut summary = dolby_vision_summary(8);
        summary.mastering_display_max_nits = Some(4000.0);
        assert_eq!(resolve_peak_nits(None, Some(&summary), &untagged), 4000.0);
        // MaxCLL is the grade's own peak
        summary.max_content_light_level_nits = Some(1200.0);
        assert_eq!(resolve_peak_nits(None, Some(&summary), &untagged), 1200.0);
    }
}
