//! What `create` does to the source picture before it is compressed: per-side
//! crop, black-border detection, deinterlace, denoise, rotate, flip, and fitting
//! the result onto the raster the CPL will declare.
//!
//! The arithmetic, the ffmpeg filters and the crop resolution live in postkit.
//! This resolves the flags a caller was given into one
//! [`postkit::picture_processing::PictureProcessing`], so the CLI and the GUI
//! cannot disagree about what a crop means.

use serde::{Deserialize, Serialize};
use std::path::Path;

use postkit::picture_processing::{
    Crop, DEFAULT_AUTO_CROP_THRESHOLD, Fit, PicturePlan, PictureProcessing, Rotation, detect_crop,
    fill_crop, require_one_crop_decider,
};

/// What the caller asked for. Crops are source pixels, taken before any
/// rotation, which is the orientation the source is stored in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// The rasters the encode has to land on: `forced_raster` is the coded raster the
/// encoder writes, `container` is the aspect the picture is fitted into and the
/// active area the CPL declares. A named container is both of them.
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
    require_one_crop_decider(
        !options.crop.is_none(),
        options.auto_crop,
        options.fill_crop,
    )?;

    let fit_box = geometry.container.or(geometry.forced_raster);
    let crop = match (options.auto_crop, options.fill_crop, fit_box) {
        (true, _, _) => detect_crop(
            source,
            options.auto_crop_threshold,
            is_image_sequence,
            source_width,
            source_height,
        )?,
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
        assert_eq!(resolved.plan.pad_left, 24);
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
    fn a_named_container_is_the_coded_raster_the_source_is_letterboxed_into() {
        let geometry = EncodeGeometry {
            forced_raster: Some(TWO_K_SCOPE),
            container: Some(TWO_K_SCOPE),
        };
        let resolved = resolve_picture(
            &SourcePictureOptions::default(),
            Path::new("/source/near-scope.mov"),
            2048,
            872,
            &geometry,
            false,
        )
        .unwrap();
        assert_eq!((resolved.encode_width, resolved.encode_height), TWO_K_SCOPE);
        assert_eq!(
            (resolved.plan.scaled_width, resolved.plan.scaled_height),
            (2014, 858)
        );
        assert_eq!((resolved.plan.pad_left, resolved.plan.pad_top), (16, 0));
    }

    #[test]
    fn a_fill_crop_onto_a_named_container_neither_scales_nor_pads() {
        let resolved = resolve_picture(
            &SourcePictureOptions {
                fill_crop: true,
                ..SourcePictureOptions::default()
            },
            Path::new("/source/near-scope.mov"),
            2048,
            872,
            &EncodeGeometry {
                forced_raster: Some(TWO_K_SCOPE),
                container: Some(TWO_K_SCOPE),
            },
            false,
        )
        .unwrap();
        assert_eq!(
            resolved.processing.crop,
            Crop {
                left: 0,
                right: 0,
                top: 6,
                bottom: 8
            }
        );
        assert_eq!((resolved.encode_width, resolved.encode_height), TWO_K_SCOPE);
        let filters = resolved.plan.filters.join(",");
        assert!(
            !filters.contains("scale=") && !filters.contains("pad="),
            "the crop already lands on the container: {filters}"
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
                error.contains("give only one of them"),
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
        assert_eq!(resolved.plan.filters, vec!["yadif", "hqdn3d", "hflip"]);
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
}
