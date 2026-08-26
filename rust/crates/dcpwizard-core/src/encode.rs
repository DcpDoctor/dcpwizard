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

/// Below this a PSNR target asks for less than the 10:1 default ratio already
/// gives, so grok's quality allocation has nothing to do.
pub const MINIMUM_QUALITY_PSNR_DB: f64 = 20.0;
/// A 12-bit component tops out near this, so a higher target only spends bits
/// without changing the picture.
pub const MAXIMUM_QUALITY_PSNR_DB: f64 = 80.0;

/// The bytes one frame may take at `mbps` and `fps`. Under a PSNR target this is
/// the ceiling the encoder holds to instead of a ratio, and a 3D encode splits
/// it between the eyes exactly as [`video_compression_ratio`] splits the ratio.
pub fn video_codestream_byte_cap(fps: u32, mbps: u32, stereoscopic: bool) -> u64 {
    let fps = fps.max(1) as f64;
    let eyes = if stereoscopic { STEREOSCOPIC_EYES } else { 1.0 };
    (mbps as f64 * 1_000_000.0 / 8.0 / fps / eyes) as u64
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

/// How a source carrying `space` reaches DCI X'Y'Z' inside the encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XyzRoute {
    /// the compressor runs its own Rec.709 RGB to X'Y'Z' transform
    CompressorTransform,
    /// postkit converts every frame with this space's matrix, and the
    /// compressor's own transform stays off
    FrameTransform(postkit::colour::ColourSpace),
    /// the source already carries X'Y'Z'
    AlreadyXyz,
}

/// The route a source carrying `space` takes to X'Y'Z' inside the encode.
///
/// Rec.709 is the compressor's own transform, X'Y'Z' is already there, and P3,
/// Rec.2020 and LogC go through postkit's own per-frame transform: that space's
/// curve, its matrix, and the compressor's transform off. ACES and ACEScg are
/// refused: they are scene-referred, so no matrix reaches X'Y'Z' from them and
/// approximating one would be silently wrong colour.
pub fn xyz_route(space: postkit::colour::ColourSpace) -> Result<XyzRoute, String> {
    use postkit::colour::ColourSpace;
    match space {
        ColourSpace::Rec709 => Ok(XyzRoute::CompressorTransform),
        ColourSpace::Xyz => Ok(XyzRoute::AlreadyXyz),
        ColourSpace::P3 | ColourSpace::Rec2020 | ColourSpace::LogC => {
            Ok(XyzRoute::FrameTransform(space))
        }
        ColourSpace::Aces | ColourSpace::AcesCg => Err(format!(
            "--source-colourspace {space:?} is scene-referred: no matrix reaches X'Y'Z' from \
             it, so it needs a rendering transform. Pass --hdr-to-dci-lut a 3D LUT that lands \
             on X'Y'Z', or convert the source first with `dcpwizard colour --target xyz --lut \
             <LUT>` and pass --source-colourspace xyz"
        )),
    }
}

impl XyzRoute {
    /// Whether the compressor runs its own X'Y'Z' transform.
    pub fn compressor_transform(self) -> bool {
        matches!(self, XyzRoute::CompressorTransform)
    }

    /// postkit's per-frame transform, built once for a whole encode.
    pub fn frame_transform(self) -> Result<Option<Arc<postkit::colour::DcdmTransform>>, String> {
        self.source_colour().frame_transform()
    }

    /// How the frames reach the encoder, for the pipeline's own colour routing.
    pub fn source_colour(self) -> postkit::encode::SourceColour {
        use postkit::encode::SourceColour;
        match self {
            XyzRoute::CompressorTransform => SourceColour::DisplayRgb,
            XyzRoute::FrameTransform(space) => SourceColour::DisplayRgbIn(space),
            // postkit spells "compress untransformed" AlreadyPq; an X'Y'Z'
            // source is that same route without the PQ label
            XyzRoute::AlreadyXyz => SourceColour::AlreadyPq,
        }
    }
}

/// Refuse a source colour space on picture that is already compressed: no
/// transform runs there, so anything but the Rec.709 default would be ignored.
pub fn check_precompressed_colourspace(space: postkit::colour::ColourSpace) -> Result<(), String> {
    if space == postkit::colour::ColourSpace::Rec709 {
        return Ok(());
    }
    Err(format!(
        "--source-colourspace {space:?} cannot be honoured for J2K input: the picture is \
         already encoded, so no colour transform can run over it. Encode from the source \
         picture instead"
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
        &Arc::new(grok_encoder::PhaseClocks::default()),
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
    fn a_bandwidth_becomes_the_bytes_one_frame_may_take() {
        assert_eq!(video_codestream_byte_cap(24, 250, false), 1_302_083);
        assert_eq!(video_codestream_byte_cap(48, 250, false), 651_041);
        assert_eq!(video_codestream_byte_cap(24, 5, false), 26_041);
        // both eyes share the track's bit rate, so each gets half the bytes
        assert_eq!(video_codestream_byte_cap(24, 250, true), 651_041);
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
            xyz_route(ColourSpace::Rec709)
                .unwrap()
                .compressor_transform(),
            "rec709 must leave the compressor doing the conversion, as it always did"
        );
        assert_eq!(
            xyz_route(ColourSpace::Xyz).unwrap(),
            XyzRoute::AlreadyXyz,
            "an X'Y'Z' source must be compressed untransformed"
        );
    }

    #[test]
    fn the_wide_gamut_spaces_convert_through_postkit_and_not_the_compressor() {
        use postkit::colour::ColourSpace;
        for space in [ColourSpace::P3, ColourSpace::Rec2020, ColourSpace::LogC] {
            let route = xyz_route(space).unwrap();
            assert_eq!(route, XyzRoute::FrameTransform(space));
            assert!(
                !route.compressor_transform(),
                "{space:?} is converted here, so the compressor must not convert it again"
            );
            assert!(
                route.frame_transform().unwrap().is_some(),
                "{space:?} must carry a transform into the encode"
            );
            assert_eq!(
                route.source_colour(),
                postkit::encode::SourceColour::DisplayRgbIn(space)
            );
        }
    }

    #[test]
    fn a_scene_referred_space_is_refused_rather_than_approximated() {
        use postkit::colour::ColourSpace;
        for space in [ColourSpace::Aces, ColourSpace::AcesCg] {
            let err = xyz_route(space).unwrap_err();
            assert!(err.contains(&format!("{space:?}")), "{err}");
            assert!(
                err.contains("3D LUT"),
                "{space:?} must name the LUT pass: {err}"
            );
            assert!(
                err.contains("--hdr-to-dci-lut"),
                "{space:?} must name the flag that takes the LUT: {err}"
            );
        }
    }

    /// postkit's `DcdmTransform` decodes LogC3 ahead of the matrix, so LogC is
    /// encoded rather than refused.
    #[test]
    fn logc_carries_the_logc3_decode_into_the_encode() {
        use postkit::colour::ColourSpace;
        assert_eq!(
            xyz_route(ColourSpace::LogC).unwrap(),
            XyzRoute::FrameTransform(ColourSpace::LogC)
        );
        assert!(postkit::colour::DcdmTransform::to_xyz(ColourSpace::LogC).is_ok());
    }

    #[test]
    fn precompressed_picture_takes_the_default_colour_space_and_nothing_else() {
        use postkit::colour::ColourSpace;
        assert!(check_precompressed_colourspace(ColourSpace::Rec709).is_ok());
        for space in [
            ColourSpace::P3,
            ColourSpace::Rec2020,
            ColourSpace::Xyz,
            ColourSpace::LogC,
        ] {
            let err = check_precompressed_colourspace(space).unwrap_err();
            assert!(
                err.contains("already encoded"),
                "{space:?} must say why it cannot be honoured: {err}"
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
