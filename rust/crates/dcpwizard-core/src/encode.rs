use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use postkit::grok;
use postkit::grok_encoder::{self, CompressParams, RawFrame};

/// Encode configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EncodeConfig {
    pub bandwidth_mbps: u32,
    pub threads: u32,
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
}

/// Convert a target J2K bandwidth (Mbit/s) to a grok compression ratio for the
/// given picture size and frame rate. Raw DCI codestream is width*height*36
/// bits/frame (12-bit XYZ, 3 components); ratio = raw_bits / target_bits.
pub fn bandwidth_to_ratio(width: u32, height: u32, fps: u32, mbps: u32) -> f64 {
    let fps = fps.max(1) as f64;
    let mbps = (mbps as f64).max(1.0);
    let raw_bits = width as f64 * height as f64 * 36.0;
    let target_bits = (mbps * 1_000_000.0) / fps;
    (raw_bits / target_bits).max(1.0)
}

/// Compression ratio used when no target bandwidth is given: 10:1 is the
/// conventional visually-lossless DCI mastering ratio.
pub const DEFAULT_COMPRESSION_RATIO: f64 = 10.0;

/// Eyes sharing one stereoscopic (ST 429-10) picture track. DCI DCSS 4.3.1 caps
/// the picture bit rate for the whole track, both eyes together, so each eye
/// gets half the budget (libdcp halves max_cs_size the same way).
const STEREOSCOPIC_EYES: f64 = 2.0;

/// Compression ratio for a video encode at `width`x`height`/`fps`. A higher
/// ratio is a smaller codestream, so halving a 3D encode's per-eye budget means
/// doubling its ratio: both eyes are encoded with these parameters and the cap
/// covers their sum.
pub fn video_compression_ratio(
    width: u32,
    height: u32,
    fps: u32,
    mbps: Option<u32>,
    stereoscopic: bool,
) -> f64 {
    let ratio = match mbps {
        Some(mbps) => bandwidth_to_ratio(width, height, fps, mbps),
        None => DEFAULT_COMPRESSION_RATIO,
    };
    if stereoscopic {
        ratio * STEREOSCOPIC_EYES
    } else {
        ratio
    }
}

/// Parse a `--source-colourspace` value into postkit's [`ColourSpace`].
///
/// [`ColourSpace`]: postkit::colour::ColourSpace
pub fn parse_source_colourspace(spec: &str) -> Result<postkit::colour::ColourSpace, String> {
    use postkit::colour::ColourSpace;
    match spec.trim().to_lowercase().as_str() {
        "rec709" | "bt709" => Ok(ColourSpace::Rec709),
        "p3" => Ok(ColourSpace::P3),
        "xyz" => Ok(ColourSpace::Xyz),
        "rec2020" | "bt2020" => Ok(ColourSpace::Rec2020),
        "aces" => Ok(ColourSpace::Aces),
        "acescg" => Ok(ColourSpace::AcesCg),
        "logc" => Ok(ColourSpace::LogC),
        other => Err(format!(
            "unknown source colour space '{other}' (use rec709, p3, xyz, rec2020, aces, acescg or logc)"
        )),
    }
}

/// Whether the compressor has to run its Rec.709 RGB to DCI X'Y'Z' transform for
/// a source carrying `space`.
///
/// Only two spaces reach X'Y'Z' from inside the encode: Rec.709, which the
/// compressor converts itself, and X'Y'Z', which is already there. Every other
/// space needs a real transform first, which `dcpwizard colour --target xyz`
/// (P3, Rec.2020) or a 3D LUT (the log and ACES spaces) does as its own pass, so
/// asking for one here is refused rather than approximated.
pub fn applies_xyz_transform(space: postkit::colour::ColourSpace) -> Result<bool, String> {
    use postkit::colour::ColourSpace;
    match space {
        ColourSpace::Rec709 => Ok(true),
        ColourSpace::Xyz => Ok(false),
        other => Err(format!(
            "--source-colourspace {other:?} has no transform inside the encode: convert the source \
             to X'Y'Z' first (`dcpwizard colour --source <space> --target xyz`, or a 3D LUT via \
             --hdr-to-dci-lut) and then pass --source-colourspace xyz"
        )),
    }
}

/// Refuse a video encode whose source raster is not the raster the encoder will
/// read. The pipeline decodes with ffmpeg and slices its raw output into
/// width*height frames with no scaling filter, so any other source size is read
/// misaligned and the CPL then declares a size the J2K frames do not have.
///
/// TODO: fit the source automatically instead (scale preserving aspect, pad to
/// the container with black); that needs the scale/pad filter in postkit's
/// ffmpeg invocation.
pub fn check_encode_raster(
    source_width: u32,
    source_height: u32,
    encode_width: u32,
    encode_height: u32,
) -> Result<(), String> {
    if (source_width, source_height) == (encode_width, encode_height) {
        return Ok(());
    }
    Err(format!(
        "source is {source_width}x{source_height} but the encode raster is \
         {encode_width}x{encode_height}; the encoder reads the source raster and does not \
         scale. Fit the source first, e.g. `dcpwizard convert -i <source> -o <fitted> \
         --target <container> --method letterbox`, or drop --twok/--fourk to encode at \
         {source_width}x{source_height}"
    ))
}

/// Encode image sequence to JPEG 2000 using in-process Grok FFI pipeline.
pub fn encode_j2k(config: &EncodeConfig) -> i32 {
    let mut frames: Vec<PathBuf> = std::fs::read_dir(&config.input_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "tif" | "tiff" | "dpx" | "exr" | "png" | "bmp"))
        })
        .collect();
    frames.sort();

    if frames.is_empty() {
        tracing::error!("No image frames found in {}", config.input_dir.display());
        return -1;
    }

    let ratio = if config.bandwidth_mbps > 0 {
        // Convert target Mbps to compression ratio
        // DCI 2K 24fps: uncompressed ≈ 2048*1080*3*12*24 bits/sec ≈ 2.28 Gbps
        // ratio = uncompressed_bps / target_bps
        let uncompressed_mbps = 2048.0 * 1080.0 * 3.0 * 12.0 * 24.0 / 1_000_000.0;
        uncompressed_mbps / config.bandwidth_mbps as f64
    } else {
        DEFAULT_COMPRESSION_RATIO
    };

    let params = CompressParams {
        compression_ratio: ratio,
        ..CompressParams::default()
    };

    let total_frames = frames.len() as u64;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut frame_iter = frames.into_iter().enumerate();

    grok_encoder::initialize(0);

    let result = grok_encoder::encode_pipeline(
        &config.output_dir,
        &params,
        total_frames,
        &cancel,
        || {
            let (idx, path) = frame_iter.next()?;
            match grok::load_tiff(&path) {
                Ok(tf) => Some(RawFrame::Planar {
                    components: tf.components,
                    width: tf.width,
                    height: tf.height,
                    precision: tf.precision,
                    index: idx as u64,
                }),
                Err(e) => {
                    tracing::error!("Failed to load {}: {e}", path.display());
                    None
                }
            }
        },
        |progress| {
            if progress.total_frames > 0 {
                tracing::info!(
                    "Encoding: {}/{} frames ({:.1} fps)",
                    progress.frames_encoded,
                    progress.total_frames,
                    progress.fps,
                );
            }
        },
    );

    if !result.success {
        tracing::error!("Encode failed: {}", result.error);
        return -1;
    }

    tracing::info!(
        "Encoded {} frames to J2K (ratio {:.1}:1)",
        result.frames_encoded,
        ratio,
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_scales_inversely_with_bandwidth() {
        // 2K 24fps: raw = 2048*1080*36 = ~79.6 Mbit/frame -> ~1911 Mbit/s raw.
        // At 250 Mbps target the ratio is ~7.6:1; halving the bandwidth doubles it.
        let r250 = bandwidth_to_ratio(2048, 1080, 24, 250);
        let r125 = bandwidth_to_ratio(2048, 1080, 24, 125);
        assert!((r250 - 7.64).abs() < 0.1, "got {r250}");
        assert!((r125 / r250 - 2.0).abs() < 0.01);
    }

    #[test]
    fn ratio_never_below_one() {
        // absurdly high bandwidth would give sub-1 ratio; clamp keeps it lossless-ish
        assert_eq!(bandwidth_to_ratio(2048, 1080, 24, 100_000), 1.0);
    }

    #[test]
    fn ratio_accounts_for_resolution_and_fps() {
        // 4K has 4x the pixels, so at the same bandwidth the ratio is 4x higher
        let two_k = bandwidth_to_ratio(2048, 1080, 24, 250);
        let four_k = bandwidth_to_ratio(4096, 2160, 24, 250);
        assert!((four_k / two_k - 4.0).abs() < 0.01);
        // doubling fps halves per-frame budget, doubling the ratio
        let hfr = bandwidth_to_ratio(2048, 1080, 48, 250);
        assert!((hfr / two_k - 2.0).abs() < 0.01);
    }

    #[test]
    fn a_source_that_is_not_the_encode_raster_is_refused() {
        // 1998x1080 flat master with --twok: the frames are not 2048 wide, so
        // reading them as 2048x1080 shears every frame
        let err = check_encode_raster(1998, 1080, 2048, 1080).unwrap_err();
        assert!(err.contains("1998x1080"), "must name the source: {err}");
        assert!(
            err.contains("2048x1080"),
            "must name the encode raster: {err}"
        );
    }

    #[test]
    fn a_source_that_matches_the_encode_raster_passes() {
        assert!(check_encode_raster(2048, 1080, 2048, 1080).is_ok());
        assert!(check_encode_raster(4096, 2160, 4096, 2160).is_ok());
    }

    #[test]
    fn stereoscopic_splits_the_bit_rate_between_the_eyes() {
        // both eyes are encoded with these parameters, so a 3D encode at the
        // requested 250 Mbps must give each eye a 125 Mbps budget: the ratio is
        // exactly the one a 2D encode at half the bandwidth would get.
        let flat = video_compression_ratio(2048, 1080, 24, Some(250), false);
        let per_eye = video_compression_ratio(2048, 1080, 24, Some(250), true);
        assert!(
            (per_eye / flat - 2.0).abs() < 1e-9,
            "3D must halve the per-eye budget: flat {flat}, per eye {per_eye}"
        );
        assert!((per_eye - bandwidth_to_ratio(2048, 1080, 24, 125)).abs() < 1e-9);
    }

    #[test]
    fn stereoscopic_halves_the_default_ratio_budget_too() {
        assert_eq!(
            video_compression_ratio(2048, 1080, 24, None, false),
            DEFAULT_COMPRESSION_RATIO
        );
        assert_eq!(
            video_compression_ratio(2048, 1080, 24, None, true),
            DEFAULT_COMPRESSION_RATIO * 2.0
        );
    }

    // the default must not move a single output byte: rec709 is what every
    // encode did before --source-colourspace existed, and it is the compressor's
    // own X'Y'Z' transform that does it
    #[test]
    fn rec709_is_the_default_and_keeps_the_encoder_transform() {
        use postkit::colour::ColourSpace;
        assert_eq!(
            parse_source_colourspace("rec709").unwrap(),
            ColourSpace::Rec709
        );
        assert!(
            applies_xyz_transform(ColourSpace::Rec709).unwrap(),
            "rec709 must leave the compressor doing the conversion, as it always did"
        );
        assert!(
            !applies_xyz_transform(ColourSpace::Xyz).unwrap(),
            "an X'Y'Z' source must be compressed untransformed"
        );
    }

    #[test]
    fn a_space_with_no_transform_here_is_refused_rather_than_approximated() {
        use postkit::colour::ColourSpace;
        for space in [
            ColourSpace::P3,
            ColourSpace::Rec2020,
            ColourSpace::Aces,
            ColourSpace::AcesCg,
            ColourSpace::LogC,
        ] {
            let err = applies_xyz_transform(space).unwrap_err();
            assert!(
                err.contains("colour --source"),
                "{space:?} must name the conversion pass: {err}"
            );
        }
    }

    #[test]
    fn every_postkit_space_has_a_spelling_and_a_typo_does_not() {
        use postkit::colour::ColourSpace;
        for (spelling, space) in [
            ("p3", ColourSpace::P3),
            ("xyz", ColourSpace::Xyz),
            ("rec2020", ColourSpace::Rec2020),
            ("aces", ColourSpace::Aces),
            ("acescg", ColourSpace::AcesCg),
            ("logc", ColourSpace::LogC),
        ] {
            assert_eq!(parse_source_colourspace(spelling).unwrap(), space);
        }
        assert!(parse_source_colourspace("rec601").is_err());
    }
}
