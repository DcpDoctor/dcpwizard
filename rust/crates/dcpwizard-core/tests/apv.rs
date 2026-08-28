//! APV (dom#3159) input-side support: ffmpeg decodes an APV source, so it flows
//! through the video → J2K → DCP pipeline to a valid DCP. An undecodable source
//! fails loud.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::probe::{ensure_video_decodable, ffmpeg_has_decoder, source_video_codec};
use postkit::grok_encoder::{CompressParams, encode_video_pipeline};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const W: u32 = 2048;
const H: u32 = 1080;
const FPS: u32 = 24;

/// Synthesize a real APV clip with the local ffmpeg (raw .apv container).
fn make_apv(path: &Path, frames: u32) {
    let dur = format!("{}", frames as f32 / FPS as f32);
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=duration={dur}:size={W}x{H}:rate={FPS}"),
            "-c:v",
            "apv",
            "-pix_fmt",
            "yuv422p10le",
        ])
        .arg(path)
        .output()
        .expect("run ffmpeg");
    assert!(
        ok.status.success() && path.exists(),
        "{}
  stdout: {}
  stderr: {}",
        "ffmpeg wrote no APV clip",
        String::from_utf8_lossy(&ok.stdout).trim(),
        String::from_utf8_lossy(&ok.stderr).trim(),
    );
}

#[test]
fn apv_source_becomes_a_valid_dcp() {
    assert!(ffmpeg_has_decoder("apv"), "this ffmpeg has no apv decoder");
    let dir = tempfile::tempdir().unwrap();
    let apv = dir.path().join("clip.apv");
    make_apv(&apv, 6);

    // the decoder gate admits APV
    assert_eq!(source_video_codec(&apv).unwrap(), "apv");
    assert!(ensure_video_decodable(&apv).is_ok());

    // decode APV → J2K via the real pipeline
    let j2k = dir.path().join("j2k");
    let params = CompressParams {
        edit_rate: postkit::encode::FrameRate::whole(FPS),
        ..CompressParams::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let result = encode_video_pipeline(&apv, &j2k, &params, 0, W, H, &cancel, |_p| {});
    assert!(
        result.success,
        "APV encode pipeline failed: {}",
        result.error
    );
    assert!(result.frames_encoded >= 1, "no frames from APV: {result:?}");

    // wrap the decoded frames into a DCP and validate
    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "APV Test".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(j2k),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "create must succeed");
    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(
        result.errors.is_empty(),
        "verify errors: {:?}",
        result.errors
    );
}

#[test]
fn unsupported_codec_fails_loud() {
    // a codec ffmpeg lacks is reported absent by the decoder gate
    assert!(!ffmpeg_has_decoder("not_a_real_codec_zzz"));

    // an undecodable source fails loud rather than silently
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("bogus.apv");
    std::fs::write(&bogus, vec![0u8; 4096]).unwrap();
    let err = ensure_video_decodable(&bogus).expect_err("garbage must be rejected");
    assert!(!err.is_empty(), "error must explain the failure");
}
