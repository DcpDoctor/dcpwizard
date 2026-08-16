//! What `create` does to the source picture before it is compressed: per-side
//! crop, black-border detection, deinterlace, denoise, rotate, flip, and fitting
//! the result onto the raster the CPL will declare.
//!
//! The arithmetic and the ffmpeg filters live in postkit. This resolves the
//! flags a caller was given into one [`postkit::picture_processing::PictureProcessing`],
//! so the CLI and the GUI cannot disagree about what a crop means.

use std::path::Path;

use postkit::encode::DecodeSource;
use postkit::picture_processing::{
    Crop, Fit, PicturePlan, PictureProcessing, Rotation, detect_black_borders,
};

/// Black level auto-crop reads as border, as a fraction of full scale. This is
/// DCP-o-matic's default.
pub const DEFAULT_AUTO_CROP_THRESHOLD: f32 = 0.1;

/// Frames spread over the content that auto-crop measures. Enough that one dark
/// shot cannot crop away picture the rest of the reel has.
const AUTO_CROP_SAMPLE_COUNT: u32 = 8;

/// Frame rate written into the concat list auto-crop hands ffmpeg for an image
/// sequence. It only spreads the samples over the list, so any rate serves.
const IMAGE_LIST_FRAME_RATE: u32 = 24;

const ROTATION_NONE: &str = "none";
const ROTATION_CLOCKWISE_90: &str = "90";
const ROTATION_HALF: &str = "180";
const ROTATION_COUNTER_CLOCKWISE_90: &str = "270";

const FLIP_NONE: &str = "none";
const FLIP_HORIZONTAL: &str = "horizontal";
const FLIP_VERTICAL: &str = "vertical";
const FLIP_BOTH: &str = "both";

/// What the caller asked for. Crops are source pixels, taken before any
/// rotation, which is the orientation the source is stored in.
#[derive(Debug, Clone, PartialEq)]
pub struct SourcePictureOptions {
    pub crop: Crop,
    pub auto_crop: bool,
    pub auto_crop_threshold: f32,
    pub fill_crop: bool,
    pub deinterlace: bool,
    pub denoise: bool,
    pub rotation: Rotation,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
}

impl Default for SourcePictureOptions {
    fn default() -> Self {
        SourcePictureOptions {
            crop: Crop::default(),
            auto_crop: false,
            auto_crop_threshold: DEFAULT_AUTO_CROP_THRESHOLD,
            fill_crop: false,
            deinterlace: false,
            denoise: false,
            rotation: Rotation::None,
            flip_horizontal: false,
            flip_vertical: false,
        }
    }
}

impl SourcePictureOptions {
    /// Whether nothing at all was asked for, so the source reaches the encoder
    /// exactly as it decodes.
    pub fn is_identity(&self) -> bool {
        self.crop.is_none()
            && !self.auto_crop
            && !self.fill_crop
            && !self.deinterlace
            && !self.denoise
            && self.rotation == Rotation::None
            && !self.flip_horizontal
            && !self.flip_vertical
    }
}

/// The rasters the encode has to land on: the one `--twok`/`--fourk` forces, and
/// the active area a container declares inside it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EncodeGeometry {
    pub forced_raster: Option<(u32, u32)>,
    pub container: Option<(u32, u32)>,
}

/// The processing a source resolves to, and the raster the encoder then reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPicture {
    pub processing: PictureProcessing,
    pub plan: PicturePlan,
    pub encode_width: u32,
    pub encode_height: u32,
}

/// Resolve the picture flags against a source of `source_width`x`source_height`.
///
/// The fit box is the container's active area when one was given, else the
/// forced raster. A forced raster is what turns the fit on: the picture is
/// scaled into the box and centred on the raster with black around it. Without
/// one, nothing is scaled and the encode raster is whatever the crop and the
/// rotation leave.
pub fn resolve_picture(
    options: &SourcePictureOptions,
    source: &Path,
    source_width: u32,
    source_height: u32,
    geometry: &EncodeGeometry,
    is_image_sequence: bool,
) -> Result<ResolvedPicture, String> {
    if options.auto_crop && options.fill_crop {
        return Err(
            "--auto-crop cuts off the source's own black borders and --fill-crop cuts \
                    it to the container aspect: pass one or the other"
                .to_string(),
        );
    }
    if !options.crop.is_none() && options.auto_crop {
        return Err(
            "--crop-left/--crop-right/--crop-top/--crop-bottom and --auto-crop both \
                    decide what to cut off: pass one or the other"
                .to_string(),
        );
    }
    if !options.crop.is_none() && options.fill_crop {
        return Err(
            "--crop-left/--crop-right/--crop-top/--crop-bottom and --fill-crop both \
                    decide what to cut off: pass one or the other"
                .to_string(),
        );
    }

    let fit_box = geometry.container.or(geometry.forced_raster);
    let crop = match (options.auto_crop, options.fill_crop, fit_box) {
        (true, _, _) => detect_crop(source, options.auto_crop_threshold, is_image_sequence)?,
        (_, true, Some(box_size)) => {
            fill_crop(source_width, source_height, box_size, options.rotation)
        }
        (_, true, None) => {
            return Err(
                "--fill-crop cuts the source to the aspect it has to fill, and nothing \
                        here names that aspect: pass --container or --twok/--fourk"
                    .to_string(),
            );
        }
        _ => options.crop,
    };

    let fit = match (geometry.forced_raster, fit_box) {
        (Some((raster_width, raster_height)), Some((box_width, box_height))) => Some(Fit {
            box_width,
            box_height,
            raster_width,
            raster_height,
        }),
        _ => None,
    };

    let processing = PictureProcessing {
        deinterlace: options.deinterlace,
        denoise: options.denoise,
        crop,
        rotation: options.rotation,
        flip_horizontal: options.flip_horizontal,
        flip_vertical: options.flip_vertical,
        fit,
    };
    let plan = processing.plan(source_width, source_height)?;
    Ok(ResolvedPicture {
        encode_width: plan.output_width,
        encode_height: plan.output_height,
        processing,
        plan,
    })
}

/// Refuse picture processing on picture that is already compressed: every filter
/// runs while ffmpeg decodes, and a codestream directory never decodes.
pub fn check_precompressed_picture(options: &SourcePictureOptions) -> Result<(), String> {
    if options.is_identity() {
        return Ok(());
    }
    Err(
        "J2K input is already compressed, so there are no frames to crop, rotate or fit: \
         encode from the source picture instead"
            .to_string(),
    )
}

/// Parse a clockwise rotation: `none`, `90`, `180` or `270`.
pub fn parse_rotation(spec: &str) -> Result<Rotation, String> {
    match spec.trim().to_lowercase().as_str() {
        "" | ROTATION_NONE => Ok(Rotation::None),
        ROTATION_CLOCKWISE_90 => Ok(Rotation::Clockwise90),
        ROTATION_HALF => Ok(Rotation::Half),
        ROTATION_COUNTER_CLOCKWISE_90 => Ok(Rotation::CounterClockwise90),
        other => Err(format!(
            "unknown rotation '{other}' (use {ROTATION_CLOCKWISE_90}, {ROTATION_HALF} or \
             {ROTATION_COUNTER_CLOCKWISE_90} degrees clockwise)"
        )),
    }
}

/// Parse a flip into (horizontal, vertical): `none`, `horizontal`, `vertical`
/// or `both`.
pub fn parse_flip(spec: &str) -> Result<(bool, bool), String> {
    match spec.trim().to_lowercase().as_str() {
        "" | FLIP_NONE => Ok((false, false)),
        FLIP_HORIZONTAL => Ok((true, false)),
        FLIP_VERTICAL => Ok((false, true)),
        FLIP_BOTH => Ok((true, true)),
        other => Err(format!(
            "unknown flip '{other}' (use {FLIP_HORIZONTAL}, {FLIP_VERTICAL} or {FLIP_BOTH})"
        )),
    }
}

/// The centred crop that brings the source to the fit box's aspect. A quarter
/// turn happens after the crop, so the box's aspect is wanted the other way
/// round.
fn fill_crop(
    source_width: u32,
    source_height: u32,
    (box_width, box_height): (u32, u32),
    rotation: Rotation,
) -> Crop {
    let (aspect_width, aspect_height) = match rotation {
        Rotation::Clockwise90 | Rotation::CounterClockwise90 => (box_height, box_width),
        Rotation::None | Rotation::Half => (box_width, box_height),
    };
    Crop::to_aspect(source_width, source_height, aspect_width, aspect_height)
}

/// Measure the black borders around the content. An image sequence has no
/// container ffmpeg can seek, so it is measured through a concat list, the same
/// way the encode decodes one.
fn detect_crop(source: &Path, threshold: f32, is_image_sequence: bool) -> Result<Crop, String> {
    if !is_image_sequence {
        return detect_black_borders(
            source,
            DecodeSource::Video,
            threshold,
            AUTO_CROP_SAMPLE_COUNT,
        );
    }
    let directory = if source.is_dir() {
        source.to_path_buf()
    } else {
        source.parent().unwrap_or(source).to_path_buf()
    };
    let frames = postkit::encode::find_source_frames(&directory)
        .map_err(|e| format!("cannot list {}: {e}", directory.display()))?;
    if frames.is_empty() {
        return Err(format!("no images in {}", directory.display()));
    }
    let list_dir = tempfile::tempdir()
        .map_err(|e| format!("cannot create a working directory for auto-crop: {e}"))?;
    let list = list_dir.path().join("frames.ffconcat");
    postkit::encode::write_image_concat_list(&frames, IMAGE_LIST_FRAME_RATE, &list)?;
    detect_black_borders(
        &list,
        DecodeSource::ImageList,
        threshold,
        AUTO_CROP_SAMPLE_COUNT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD_WIDTH: u32 = 1920;
    const HD_HEIGHT: u32 = 1080;
    const TWO_K_RASTER: (u32, u32) = (2048, 1080);
    const TWO_K_SCOPE: (u32, u32) = (2048, 858);

    fn resolve(
        options: &SourcePictureOptions,
        geometry: &EncodeGeometry,
    ) -> Result<ResolvedPicture, String> {
        resolve_picture(
            options,
            Path::new("/source/never-read.mov"),
            HD_WIDTH,
            HD_HEIGHT,
            geometry,
            false,
        )
    }

    #[test]
    fn nothing_asked_for_leaves_the_source_at_its_own_raster() {
        let resolved =
            resolve(&SourcePictureOptions::default(), &EncodeGeometry::default()).unwrap();
        assert!(resolved.processing.is_identity());
        assert!(resolved.plan.is_identity());
        assert_eq!(
            (resolved.encode_width, resolved.encode_height),
            (HD_WIDTH, HD_HEIGHT)
        );
    }

    #[test]
    fn a_forced_raster_fits_the_source_onto_it_instead_of_refusing_the_size() {
        // the case that used to be refused: a flat master with --twok
        let resolved = resolve_picture(
            &SourcePictureOptions::default(),
            Path::new("/source/flat.mov"),
            1998,
            1080,
            &EncodeGeometry {
                forced_raster: Some(TWO_K_RASTER),
                container: None,
            },
            false,
        )
        .unwrap();
        assert_eq!(
            (resolved.encode_width, resolved.encode_height),
            TWO_K_RASTER
        );
        assert_eq!(
            (resolved.plan.scaled_width, resolved.plan.scaled_height),
            (1998, 1080),
            "nothing grows past the box, so the flat picture keeps its size"
        );
        assert_eq!(resolved.plan.pad_left, 25);
    }

    #[test]
    fn a_fill_crop_reaches_the_container_full_frame_on_the_forced_raster() {
        let resolved = resolve(
            &SourcePictureOptions {
                fill_crop: true,
                ..SourcePictureOptions::default()
            },
            &EncodeGeometry {
                forced_raster: Some(TWO_K_RASTER),
                container: Some(TWO_K_SCOPE),
            },
        )
        .unwrap();
        assert_eq!(
            resolved.processing.crop,
            Crop {
                left: 0,
                right: 0,
                top: 138,
                bottom: 138
            }
        );
        assert_eq!(
            (resolved.encode_width, resolved.encode_height),
            TWO_K_RASTER,
            "the encode raster is the forced raster, the container is masked out of it"
        );
        assert_eq!(
            (resolved.plan.scaled_width, resolved.plan.scaled_height),
            (2048, 856)
        );
    }

    #[test]
    fn a_quarter_turn_takes_the_fill_crop_aspect_the_other_way_round() {
        let upright = resolve(
            &SourcePictureOptions {
                fill_crop: true,
                rotation: Rotation::Clockwise90,
                ..SourcePictureOptions::default()
            },
            &EncodeGeometry {
                forced_raster: Some(TWO_K_RASTER),
                container: Some(TWO_K_SCOPE),
            },
        )
        .unwrap();
        // a 2.39:1 frame after a quarter turn means a 1:2.39 crop before it, so
        // the sides go rather than the top and bottom
        assert_eq!(upright.processing.crop.top, 0);
        assert_eq!(upright.processing.crop.bottom, 0);
        assert!(upright.processing.crop.left > 0);
        assert_eq!(
            (upright.plan.rotated_width, upright.plan.rotated_height),
            (1080, 452)
        );
    }

    #[test]
    fn a_container_without_a_forced_raster_crops_but_never_scales() {
        let resolved = resolve(
            &SourcePictureOptions {
                fill_crop: true,
                ..SourcePictureOptions::default()
            },
            &EncodeGeometry {
                forced_raster: None,
                container: Some(TWO_K_SCOPE),
            },
        )
        .unwrap();
        assert!(resolved.processing.fit.is_none());
        assert_eq!((resolved.encode_width, resolved.encode_height), (1920, 804));
    }

    #[test]
    fn a_fill_crop_with_no_aspect_to_fill_is_refused() {
        let error = resolve(
            &SourcePictureOptions {
                fill_crop: true,
                ..SourcePictureOptions::default()
            },
            &EncodeGeometry::default(),
        )
        .unwrap_err();
        assert!(error.contains("--container"), "{error}");
        assert!(error.contains("--twok"), "{error}");
    }

    #[test]
    fn the_three_ways_of_choosing_a_crop_refuse_each_other() {
        let manual = Crop {
            left: 4,
            right: 4,
            top: 0,
            bottom: 0,
        };
        let geometry = EncodeGeometry {
            forced_raster: Some(TWO_K_RASTER),
            container: None,
        };
        for (label, options) in [
            (
                "auto and fill",
                SourcePictureOptions {
                    auto_crop: true,
                    fill_crop: true,
                    ..SourcePictureOptions::default()
                },
            ),
            (
                "manual and auto",
                SourcePictureOptions {
                    crop: manual,
                    auto_crop: true,
                    ..SourcePictureOptions::default()
                },
            ),
            (
                "manual and fill",
                SourcePictureOptions {
                    crop: manual,
                    fill_crop: true,
                    ..SourcePictureOptions::default()
                },
            ),
        ] {
            let error = resolve(&options, &geometry).unwrap_err();
            assert!(
                error.contains("one or the other"),
                "{label} must be refused by name: {error}"
            );
        }
    }

    #[test]
    fn a_crop_that_eats_the_whole_source_fails_loud() {
        let error = resolve(
            &SourcePictureOptions {
                crop: Crop {
                    left: 960,
                    right: 960,
                    top: 0,
                    bottom: 0,
                },
                ..SourcePictureOptions::default()
            },
            &EncodeGeometry::default(),
        )
        .unwrap_err();
        assert!(error.contains("leaves nothing"), "{error}");
    }

    #[test]
    fn the_filters_carry_every_step_that_was_asked_for() {
        let resolved = resolve(
            &SourcePictureOptions {
                deinterlace: true,
                denoise: true,
                flip_horizontal: true,
                ..SourcePictureOptions::default()
            },
            &EncodeGeometry::default(),
        )
        .unwrap();
        assert_eq!(
            resolved.plan.filters,
            vec!["yadif", "hqdn3d", "format=gbrp16le", "hflip"]
        );
    }

    #[test]
    fn already_compressed_picture_takes_no_processing() {
        assert!(check_precompressed_picture(&SourcePictureOptions::default()).is_ok());
        let error = check_precompressed_picture(&SourcePictureOptions {
            deinterlace: true,
            ..SourcePictureOptions::default()
        })
        .unwrap_err();
        assert!(error.contains("already compressed"), "{error}");
    }

    #[test]
    fn every_rotation_and_flip_has_a_spelling_and_a_typo_does_not() {
        assert_eq!(parse_rotation("none").unwrap(), Rotation::None);
        assert_eq!(parse_rotation("90").unwrap(), Rotation::Clockwise90);
        assert_eq!(parse_rotation("180").unwrap(), Rotation::Half);
        assert_eq!(parse_rotation("270").unwrap(), Rotation::CounterClockwise90);
        assert!(parse_rotation("45").is_err());

        assert_eq!(parse_flip("none").unwrap(), (false, false));
        assert_eq!(parse_flip("horizontal").unwrap(), (true, false));
        assert_eq!(parse_flip("vertical").unwrap(), (false, true));
        assert_eq!(parse_flip("both").unwrap(), (true, true));
        assert!(parse_flip("sideways").is_err());
    }
}
