//! A trimmed video source is encoded as the kept window and nothing else: the
//! encoder writes exactly the frames the package ships, so no codestream is
//! compressed and then relinked away.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use postkit::grok_encoder::{self, CompressParams, EncodeProgress};

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FPS: u32 = 24;
const SOURCE_FRAMES: u64 = 12;
const TRIM_START: u64 = 3;
const TRIM_END: u64 = 3;
const KEPT: u64 = SOURCE_FRAMES - TRIM_START - TRIM_END;

fn make_clip(path: &Path) {
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=s={WIDTH}x{HEIGHT}:r={FPS}"),
            "-frames:v",
            &SOURCE_FRAMES.to_string(),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .status()
        .expect("run ffmpeg");
    assert!(ok.success() && path.exists(), "ffmpeg wrote no source clip");
}

fn codestream_name(index: u64) -> String {
    format!("frame_{index:08}.j2c")
}

fn encode(
    clip: &Path,
    out_dir: &Path,
    frames: u64,
    window: Option<postkit::encode::FrameRange>,
) -> u64 {
    let params = CompressParams {
        compression_ratio: 10.0,
        edit_rate: postkit::encode::FrameRate::whole(FPS),
        apply_xyz_transform: true,
        ..CompressParams::default()
    };
    grok_encoder::initialize(0);
    let result = grok_encoder::encode_video_pipeline_resumable(
        clip,
        out_dir,
        &params,
        frames,
        WIDTH,
        HEIGHT,
        &Arc::new(AtomicBool::new(false)),
        false,
        None,
        window,
        |_progress: EncodeProgress| {},
    );
    assert!(result.success, "encode failed: {}", result.error);
    result.frames_encoded
}

#[test]
fn a_trimmed_video_encodes_the_kept_window_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("source.mp4");
    make_clip(&clip);

    // the shared decision both front ends read: a video is windowed, so neither
    // relinks the codestreams afterwards
    let window = dcpwizard_core::trim::encode_window(&clip, TRIM_START, KEPT)
        .expect("a video source takes an encode window");
    assert_eq!(window.first_frame, TRIM_START);
    assert_eq!(window.frame_count, KEPT);

    let windowed_dir = dir.path().join("windowed");
    assert_eq!(encode(&clip, &windowed_dir, KEPT, Some(window)), KEPT);
    assert_eq!(
        dcpwizard_core::trim::frame_count(&windowed_dir),
        KEPT,
        "the encoder wrote one codestream per kept frame"
    );
    assert!(
        !dir.path().join("j2k_trimmed").exists() && !windowed_dir.join("j2k_trimmed").exists(),
        "a windowed encode needs no trimmed copy of the picture"
    );

    let whole_dir = dir.path().join("whole");
    assert_eq!(
        encode(&clip, &whole_dir, SOURCE_FRAMES, None),
        SOURCE_FRAMES
    );

    // frame N of the window is the frame a full encode wrote at first_frame + N,
    // which is what proves the window landed where it was asked for
    for index in 0..KEPT {
        let source_index = index + TRIM_START;
        assert_eq!(
            std::fs::read(windowed_dir.join(codestream_name(index))).unwrap(),
            std::fs::read(whole_dir.join(codestream_name(source_index))).unwrap(),
            "window frame {index} is not source frame {source_index}"
        );
    }
}
