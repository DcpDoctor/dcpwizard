use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// A still sequence to compress at a bandwidth: the `encode` command and the
/// EncodeJ2k job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSequenceEncode {
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    /// 0 encodes at the default ratio
    pub bandwidth_mbps: u32,
    pub fps: u32,
}

/// Compress a still sequence into `<output_dir>/j2k` at the bandwidth's bytes a frame.
pub fn encode_image_sequence(
    encode: &ImageSequenceEncode,
    cancel: &Arc<AtomicBool>,
    on_progress: impl Fn(&postkit::pipeline::PipelineProgress),
) -> Result<postkit::pipeline::EncodeResult, String> {
    postkit::pipeline::run_encode_with_options(
        &encode.input_dir,
        &encode.output_dir,
        &image_sequence_encode_options(encode),
        cancel,
        &Arc::new(AtomicBool::new(false)),
        on_progress,
        |message| tracing::debug!("{message}"),
    )
}

fn image_sequence_encode_options(
    encode: &ImageSequenceEncode,
) -> postkit::pipeline::EncodeRunOptions {
    let fps = encode.fps;
    let dci_cap = postkit::j2k::dci_codestream_byte_cap(fps);
    let target_codestream_bytes = (encode.bandwidth_mbps > 0)
        .then(|| video_codestream_byte_cap(fps, encode.bandwidth_mbps, false));
    postkit::pipeline::EncodeRunOptions {
        compression_ratio: DEFAULT_COMPRESSION_RATIO,
        target_codestream_bytes,
        fps: postkit::encode::FrameRate::whole(fps),
        codestream_byte_cap: Some(
            target_codestream_bytes.map_or(dci_cap, |target| dci_cap.min(target)),
        ),
        ..Default::default()
    }
}

/// Compression ratio used when no target bandwidth is given: 10:1 is the
/// conventional visually-lossless DCI mastering ratio.
pub const DEFAULT_COMPRESSION_RATIO: f64 = 10.0;

/// Eyes sharing one stereoscopic (ST 429-10) picture track. DCI DCSS 4.3.1 caps
/// the picture bit rate for the whole track, both eyes together, so each eye
/// gets half the budget (libdcp halves max_cs_size the same way).
const STEREOSCOPIC_EYES: f64 = 2.0;

/// Below this a PSNR target asks for less than the 10:1 default ratio already
/// gives, so grok's quality allocation has nothing to do.
pub const MINIMUM_QUALITY_PSNR_DB: f64 = 20.0;
/// A 12-bit component tops out near this, so a higher target only spends bits
/// without changing the picture.
pub const MAXIMUM_QUALITY_PSNR_DB: f64 = 80.0;

/// The bytes one frame may take at `mbps` and `fps`, halved per eye in 3D.
pub fn video_codestream_byte_cap(fps: u32, mbps: u32, stereoscopic: bool) -> u64 {
    let fps = fps.max(1) as f64;
    let eyes = if stereoscopic { STEREOSCOPIC_EYES } else { 1.0 };
    (mbps as f64 * 1_000_000.0 / 8.0 / fps / eyes) as u64
}

/// Parse a `--source-colourspace` value into postkit's [`ColourSpace`].
///
/// [`ColourSpace`]: postkit::colour::ColourSpace
pub fn parse_source_colourspace(spec: &str) -> Result<postkit::colour::ColourSpace, String> {
    postkit::colour::parse_colour_space(spec).ok_or_else(|| {
        format!(
            "unknown source colour space '{}' (use rec709, p3, xyz, rec2020, aces, acescg or logc)",
            spec.trim().to_lowercase()
        )
    })
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
/// P3-D65, Rec.2020 and LogC go through postkit's own per-frame transform: that space's
/// curve, its matrix, and the compressor's transform off. ACES and ACEScg are
/// refused: they are scene-referred, so no matrix reaches X'Y'Z' from them and
/// approximating one would be silently wrong colour.
pub fn xyz_route(space: postkit::colour::ColourSpace) -> Result<XyzRoute, String> {
    use postkit::colour::ColourSpace;
    match space {
        ColourSpace::Rec709 => Ok(XyzRoute::CompressorTransform),
        ColourSpace::Xyz => Ok(XyzRoute::AlreadyXyz),
        ColourSpace::P3 | ColourSpace::P3D65 | ColourSpace::Rec2020 | ColourSpace::LogC => {
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
    pub fn frame_transform(
        self,
    ) -> Result<Option<Arc<postkit::colour::FrameColourTransform>>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bandwidth_becomes_the_bytes_one_frame_may_take() {
        assert_eq!(video_codestream_byte_cap(24, 250, false), 1_302_083);
        assert_eq!(video_codestream_byte_cap(48, 250, false), 651_041);
        assert_eq!(video_codestream_byte_cap(24, 5, false), 26_041);
        assert_eq!(video_codestream_byte_cap(24, 230, false), 1_197_916);
        // both eyes share the track's bit rate, so each gets half the bytes
        assert_eq!(video_codestream_byte_cap(24, 250, true), 651_041);
    }

    fn image_sequence(bandwidth_mbps: u32) -> ImageSequenceEncode {
        ImageSequenceEncode {
            input_dir: PathBuf::from("stills"),
            output_dir: PathBuf::from("out"),
            bandwidth_mbps,
            fps: 24,
        }
    }

    #[test]
    fn a_bandwidth_is_what_the_still_sequence_allocation_aims_at() {
        let options = image_sequence_encode_options(&image_sequence(230));
        assert_eq!(options.target_codestream_bytes, Some(1_197_916));
        assert_eq!(options.compression_ratio, DEFAULT_COMPRESSION_RATIO);
        assert_eq!(options.codestream_byte_cap, Some(1_197_916));
    }

    #[test]
    fn no_bandwidth_encodes_at_the_default_ratio_under_the_dci_cap() {
        let options = image_sequence_encode_options(&image_sequence(0));
        assert_eq!(options.target_codestream_bytes, None);
        assert_eq!(options.compression_ratio, DEFAULT_COMPRESSION_RATIO);
        assert_eq!(
            options.codestream_byte_cap,
            Some(postkit::j2k::dci_codestream_byte_cap(24))
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
