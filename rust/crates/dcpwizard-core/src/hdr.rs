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
    Hlg,
    PqP3D65,
    DolbyVision,
}

impl HdrSourceFormat {
    pub fn parse(name: &str) -> Option<Self> {
        if name.trim().to_lowercase() == DOLBY_VISION_NAME {
            return Some(Self::DolbyVision);
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
        HdrSourceFormat::Hdr10 => (HdrSource::Hdr10, None),
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
    let (transfer, primaries) = colour_tags(video)?;
    match (transfer.as_str(), primaries.as_str()) {
        (PQ_TRANSFER_TAG, BT2020_PRIMARIES_TAG) => Ok(HdrSourceFormat::Hdr10),
        (PQ_TRANSFER_TAG, P3D65_PRIMARIES_TAG) => Ok(HdrSourceFormat::PqP3D65),
        (HLG_TRANSFER_TAG, _) => Ok(HdrSourceFormat::Hlg),
        _ => Err(format!(
            "{} is tagged with the {transfer} transfer and {primaries} primaries, which names no \
             HDR grade --hdr-dci can transform: name the master with \
             --hdr-source <hdr10|hlg|pq-p3d65|dolby-vision>",
            video.display()
        )),
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
