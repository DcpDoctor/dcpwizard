//! A qualifying build writes its picture MXF while the encode runs, and the DCP
//! that comes out is the one the encode-then-wrap path used to produce: the
//! codestreams the encoder wrote are the essence, and dcpdoctor passes it.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::overlapped_picture::{
    PackageShape, PictureSource, PictureWrapTarget, encode_and_wrap_picture, overlap_refusal,
};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: u64 = 4;
const FPS: u32 = 24;

fn have_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn make_clip(path: &Path) {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=s={WIDTH}x{HEIGHT}:r={FPS}"),
            "-frames:v",
            &FRAMES.to_string(),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("ffmpeg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn plain_video() -> PictureSource {
    PictureSource {
        input_type: postkit::encode::InputType::Video,
        still_hold: false,
        trims_picture: false,
    }
}

fn single_reel() -> PackageShape {
    PackageShape {
        stereoscopic: false,
        pads: false,
        splits_reels: false,
        multiple_versions: false,
        encrypts: false,
    }
}

fn cpl_of(dir: &Path) -> String {
    let path = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_"))
        })
        .expect("a CPL");
    std::fs::read_to_string(path).unwrap()
}

fn between(haystack: &str, start: &str, end: &str) -> String {
    let from = haystack.find(start).expect(start) + start.len();
    let rest = &haystack[from..];
    rest[..rest.find(end).expect(end)].to_string()
}

#[test]
fn a_video_build_wraps_its_picture_during_the_encode() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let video = root.path().join("clip.mp4");
    make_clip(&video);
    let encode_dir = root.path().join("enc");
    let dcp_dir = root.path().join("dcp");

    assert_eq!(overlap_refusal(&plain_video(), &single_reel()), None);

    let (encode, wrapped) = encode_and_wrap_picture(
        &video,
        &encode_dir,
        &postkit::pipeline::EncodeRunOptions {
            fps: postkit::encode::FrameRate::whole(FPS),
            ..Default::default()
        },
        PictureWrapTarget {
            dcp_dir: dcp_dir.clone(),
            fps: FPS,
            hdr_dci: false,
        },
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(AtomicBool::new(false)),
        |_| {},
        |_| {},
    )
    .expect("overlapped encode and wrap");

    assert_eq!(encode.frames_encoded, FRAMES);
    assert_eq!(wrapped.duration, FRAMES);
    let picture_mxf = dcp_dir.join(wrapped.mxf_name());
    assert!(
        picture_mxf.is_file(),
        "{} is missing",
        picture_mxf.display()
    );

    let config = DcpConfig {
        title: "Overlapped".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: dcp_dir.clone(),
        j2k_dir: Some(encode.j2k_dir.clone()),
        picture_mxf: Some(wrapped),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "packaging must succeed");

    // packaging did not wrap a second picture over the same frames
    let pictures: Vec<String> = std::fs::read_dir(&dcp_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("picture_"))
        .collect();
    assert_eq!(
        pictures,
        vec![picture_mxf.file_name().unwrap().to_string_lossy()]
    );

    // the essence holds the codestreams the encoder wrote, in index order: the
    // wrap sees them in completion order, so a reordering slip shows up here
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&picture_mxf.to_string_lossy())
        .expect("the picture MXF opens");
    assert_eq!(
        reader.picture_descriptor().unwrap().container_duration,
        FRAMES as u32
    );
    for index in 0..FRAMES {
        let codestream = encode.j2k_dir.join(format!("frame_{index:08}.j2c"));
        let mut buf = vec![0u8; 16 << 20];
        let read = reader
            .read_frame(index as u32, &mut buf, None, None)
            .unwrap();
        buf.truncate(read);
        assert_eq!(
            buf,
            std::fs::read(&codestream).unwrap(),
            "frame {index} of the MXF is not {}",
            codestream.display()
        );
    }

    let cpl = cpl_of(&dcp_dir);
    assert!(cpl.contains("<ContentTitleText>Overlapped</ContentTitleText>"));
    assert_eq!(
        between(&cpl, "<IntrinsicDuration>", "</IntrinsicDuration>"),
        FRAMES.to_string()
    );

    let result = dcpwizard_core::verify::verify_dcp(&dcp_dir);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}

/// The package half of the predicate is enforced where it matters: a config that
/// wraps its picture in some other shape refuses an already-wrapped one rather
/// than wrapping a second MXF and leaving the first orphaned.
#[test]
fn a_package_that_reshapes_the_picture_refuses_an_overlapped_wrap() {
    let root = tempfile::tempdir().unwrap();
    let j2k_dir = root.path().join("j2k");
    std::fs::create_dir_all(&j2k_dir).unwrap();
    dcpwizard_core::pad::generate_black_frame(WIDTH, HEIGHT, FPS, &j2k_dir.join("frame_00000.j2c"))
        .expect("encode a frame");

    let config = DcpConfig {
        title: "Padded".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: root.path().join("dcp"),
        j2k_dir: Some(j2k_dir),
        pad_head: Some("24f".into()),
        picture_mxf: Some(dcpwizard_core::overlapped_picture::PreWrappedPicture {
            asset_uuid: [7u8; 16],
            duration: 1,
        }),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), -1);
}
