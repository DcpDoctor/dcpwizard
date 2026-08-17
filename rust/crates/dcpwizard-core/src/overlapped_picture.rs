//! Writing the picture MXF while the encode runs, instead of reading the whole
//! J2K directory back once the encode is over.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The picture MXF file name `create_dcp` writes and the CPL, PKL and ASSETMAP
/// declare. An overlapped wrap has to write to exactly this name, since nothing
/// renames the file afterwards.
pub fn picture_mxf_name(asset_uuid: &uuid::Uuid) -> String {
    format!("picture_{asset_uuid}.mxf")
}

/// A picture MXF the caller already wrote with [`encode_and_wrap_picture`],
/// handed to `create_dcp` as `DcpConfig::picture_mxf` in place of the wrap it
/// would otherwise do itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreWrappedPicture {
    /// The AssetUUID written into the MXF, which also names the file.
    pub asset_uuid: [u8; 16],
    /// Frames the MXF holds.
    pub duration: u64,
}

impl PreWrappedPicture {
    pub fn mxf_name(&self) -> String {
        picture_mxf_name(&uuid::Uuid::from_bytes(self.asset_uuid))
    }
}

/// What the source and the encode are, known only before the encode starts.
pub struct PictureSource {
    /// What postkit classified the picture input as.
    pub input_type: postkit::encode::InputType,
    /// The source is one image held for a run of frames.
    pub still_hold: bool,
    /// A head or tail trim drops picture frames after the encode.
    pub trims_picture: bool,
}

/// What the package does to its own picture essence. `create_dcp` reads it off
/// the config it was handed; a caller reads it off the job before the encode.
pub struct PackageShape {
    pub stereoscopic: bool,
    /// Black frames are prepended or appended to the program.
    pub pads: bool,
    pub splits_reels: bool,
    /// More than one CPL over shared essence.
    pub multiple_versions: bool,
    pub encrypts: bool,
}

impl PackageShape {
    /// What a config says about itself. `multiple_versions` is the packager's own
    /// answer, since the config looks the same whether one CPL is written over it
    /// or several.
    pub fn of(config: &crate::dcp::DcpConfig, multiple_versions: bool) -> Self {
        Self {
            stereoscopic: config.right_eye_dir.is_some(),
            pads: config.pad_head.is_some() || config.pad_tail.is_some(),
            splits_reels: config.reel_length_minutes > 0 || !config.reel_split_frames.is_empty(),
            multiple_versions,
            encrypts: config.encrypt,
        }
    }
}

/// Why this build's picture MXF cannot be written while the encode runs, or None
/// when it can.
///
/// postkit hands asdcplib one codestream at a time as the in-process encoder
/// finishes it, so the overlap holds only where the frames the encoder produces
/// are exactly the frames one picture MXF ships, in that order.
pub fn overlap_refusal(source: &PictureSource, package: &PackageShape) -> Option<&'static str> {
    package_refusal(package).or_else(|| source_refusal(source))
}

/// The half of [`overlap_refusal`] a packager handed an already-wrapped picture
/// can check without knowing what the source was.
pub fn package_refusal(package: &PackageShape) -> Option<&'static str> {
    if package.encrypts {
        return Some(
            "the picture content key is minted while the package is written, so an earlier wrap \
             has no key to encrypt the essence with",
        );
    }
    if package.stereoscopic {
        return Some("a 3D picture interleaves two eyes per edit unit from two encodes");
    }
    if package.pads {
        return Some(
            "head or tail padding puts black frames in the essence the encoder never made",
        );
    }
    if package.splits_reels {
        return Some("a split composition wraps one picture MXF per reel");
    }
    if package.multiple_versions {
        return Some("versions wrap their own picture over the reel ranges they share");
    }
    None
}

fn source_refusal(source: &PictureSource) -> Option<&'static str> {
    if source.still_hold {
        return Some(
            "a held still is encoded once and its codestream linked for the rest of the hold, so \
             nothing feeds the wrap frame by frame",
        );
    }
    if source.input_type != postkit::encode::InputType::Video {
        return Some(
            "only a video source is decoded frame by frame: a J2K sequence is never encoded, and \
             an image sequence can go straight to grk_compress",
        );
    }
    if source.trims_picture {
        return Some(
            "a head or tail trim drops frames after the encode, so the MXF would carry frames the \
             composition does not",
        );
    }
    None
}

/// Where the picture MXF goes when it is written as the encode runs. The file
/// name and the asset id are minted inside [`encode_and_wrap_picture`] so they
/// match what `create_dcp` would have written.
pub struct PictureWrapTarget {
    /// The DCP directory the MXF is written straight into, which is
    /// `DcpConfig::output_dir`.
    pub dcp_dir: PathBuf,
    pub fps: u32,
    /// DCI HDR Addendum: stamp ST 2084 / P3-D65 onto the picture MXF.
    pub hdr_dci: bool,
}

/// Encode the picture and write its MXF as the frames finish, returning what
/// `create_dcp` needs to declare it. The J2K codestreams stay behind in
/// `encode_dir` as they always did, since the CPL still reads the coded raster
/// off the first of them.
#[allow(clippy::too_many_arguments)]
pub fn encode_and_wrap_picture(
    video: &Path,
    encode_dir: &Path,
    options: &postkit::pipeline::EncodeRunOptions,
    target: PictureWrapTarget,
    cancel: &Arc<AtomicBool>,
    pause: &Arc<AtomicBool>,
    on_progress: impl Fn(&postkit::pipeline::PipelineProgress),
    on_log: impl Fn(&str),
) -> Result<(postkit::pipeline::EncodeResult, PreWrappedPicture), String> {
    std::fs::create_dir_all(&target.dcp_dir)
        .map_err(|e| format!("cannot create {}: {e}", target.dcp_dir.display()))?;
    let asset_uuid = uuid::Uuid::new_v4();
    let (encode, track) = postkit::pipeline::run_encode_and_wrap_picture(
        video,
        encode_dir,
        options,
        postkit::mxf_wrap::IncrementalWrapOptions {
            output: target.dcp_dir.join(picture_mxf_name(&asset_uuid)),
            standard: postkit::mxf_wrap::MxfStandard::AsDcp,
            fps_num: target.fps,
            fps_den: 1,
            encryption: None,
            hdr: target.hdr_dci.then(crate::mxf_wrap::dci_hdr_metadata),
            asset_uuid: Some(*asset_uuid.as_bytes()),
        },
        cancel,
        pause,
        on_progress,
        on_log,
    )?;
    Ok((
        encode,
        PreWrappedPicture {
            asset_uuid: *asset_uuid.as_bytes(),
            duration: track.duration,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_video() -> PictureSource {
        PictureSource {
            input_type: postkit::encode::InputType::Video,
            still_hold: false,
            trims_picture: false,
        }
    }

    fn plain_package() -> PackageShape {
        PackageShape {
            stereoscopic: false,
            pads: false,
            splits_reels: false,
            multiple_versions: false,
            encrypts: false,
        }
    }

    #[test]
    fn a_plain_video_into_a_single_reel_package_qualifies() {
        assert_eq!(overlap_refusal(&plain_video(), &plain_package()), None);
    }

    #[test]
    fn every_source_that_does_not_stream_each_packaged_frame_is_refused() {
        for source in [
            PictureSource {
                still_hold: true,
                ..plain_video()
            },
            PictureSource {
                input_type: postkit::encode::InputType::J2kSequence,
                ..plain_video()
            },
            PictureSource {
                input_type: postkit::encode::InputType::ImageSequence,
                ..plain_video()
            },
            PictureSource {
                trims_picture: true,
                ..plain_video()
            },
        ] {
            assert!(overlap_refusal(&source, &plain_package()).is_some());
        }
    }

    #[test]
    fn every_package_that_reshapes_the_picture_is_refused() {
        for package in [
            PackageShape {
                encrypts: true,
                ..plain_package()
            },
            PackageShape {
                stereoscopic: true,
                ..plain_package()
            },
            PackageShape {
                pads: true,
                ..plain_package()
            },
            PackageShape {
                splits_reels: true,
                ..plain_package()
            },
            PackageShape {
                multiple_versions: true,
                ..plain_package()
            },
        ] {
            assert!(package_refusal(&package).is_some());
            assert!(overlap_refusal(&plain_video(), &package).is_some());
        }
    }
}
