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
}
