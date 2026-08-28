//! `create --twok --container 2k-scope --fill-crop` on a letterboxed HD source:
//! the bars are cut, the picture is scaled to the scope container and centred on
//! the 2K raster, and the CPL declares that raster with the container masked out
//! of it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::source_picture::{EncodeGeometry, SourcePictureOptions, resolve_picture};

const SOURCE_WIDTH: u32 = 1920;
const SOURCE_HEIGHT: u32 = 1080;
/// Height of the picture inside the letterbox, 1920 at the scope aspect.
const CONTENT_HEIGHT: u32 = 804;
const FPS: u32 = 24;
const FRAMES: u64 = 6;

const TWO_K_RASTER: (u32, u32) = (2048, 1080);
const TWO_K_SCOPE: (u32, u32) = (2048, 858);

/// A letterboxed HD clip: colour bars at the scope aspect with real black above
/// and below them.
fn make_letterboxed_clip(path: &Path) {
    let duration = format!("{}", FRAMES as f32 / FPS as f32);
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=duration={duration}:size={SOURCE_WIDTH}x{CONTENT_HEIGHT}:rate={FPS}"),
            "-vf",
            &format!(
                "pad={SOURCE_WIDTH}:{SOURCE_HEIGHT}:0:{}",
                (SOURCE_HEIGHT - CONTENT_HEIGHT) / 2
            ),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("run ffmpeg");
    assert!(
        ok.status.success() && path.exists(),
        "ffmpeg wrote no letterboxed clip\n  stdout: {}\n  stderr: {}",
        String::from_utf8_lossy(&ok.stdout).trim(),
        String::from_utf8_lossy(&ok.stderr).trim(),
    );
}

/// A stereo 48 kHz WAV covering the content, so the CPL carries the
/// CompositionMetadataAsset holding the stored and active areas.
fn make_wav(path: &Path) -> PathBuf {
    let sample_rate = 48_000u32;
    let channels = 2u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let data_len = FRAMES * (sample_rate as u64 / FPS as u64) * block_align as u64;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_len as u32).to_le_bytes());
    wav.resize(wav.len() + data_len as usize, 0);
    std::fs::write(path, &wav).unwrap();
    path.to_path_buf()
}

fn read_cpl(dir: &Path) -> String {
    let path = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("CPL_"))
        })
        .expect("CPL written");
    std::fs::read_to_string(path).unwrap()
}

/// The raster the picture MXF really carries, read back through asdcplib.
fn essence_raster(dir: &Path) -> (u32, u32) {
    let mxf = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("picture") && name.ends_with(".mxf"))
        })
        .expect("picture MXF");
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader.open_read(&mxf.to_string_lossy()).expect("open mxf");
    let descriptor = reader.picture_descriptor().expect("picture descriptor");
    (descriptor.stored_width, descriptor.stored_height)
}

/// The edges of one CPL geometry block.
fn area(cpl: &str, block_name: &str) -> (u32, u32) {
    let block = cpl
        .split_once(&format!("<meta:{block_name}>"))
        .and_then(|(_, rest)| rest.split_once(&format!("</meta:{block_name}>")))
        .unwrap_or_else(|| panic!("{block_name} present in {cpl}"))
        .0;
    let edge = |tag: &str| -> u32 {
        block
            .split_once(&format!("<meta:{tag}>"))
            .and_then(|(_, rest)| rest.split_once(&format!("</meta:{tag}>")))
            .and_then(|(value, _)| value.trim().parse().ok())
            .unwrap_or(0)
    };
    (edge("Width"), edge("Height"))
}

#[test]
fn a_letterboxed_source_fills_the_scope_container_on_the_two_k_raster() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("letterboxed.mp4");
    make_letterboxed_clip(&clip);

    let resolved = resolve_picture(
        &SourcePictureOptions {
            fill_crop: true,
            ..SourcePictureOptions::default()
        },
        &clip,
        SOURCE_WIDTH,
        SOURCE_HEIGHT,
        &EncodeGeometry {
            forced_raster: Some(TWO_K_RASTER),
            container: Some(TWO_K_SCOPE),
        },
        false,
    )
    .expect("the fill crop resolves");
    assert_eq!(
        (resolved.encode_width, resolved.encode_height),
        TWO_K_RASTER
    );

    let work = dir.path().join("encode");
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let encoded = postkit::pipeline::run_encode_with_options(
        &clip,
        &work,
        &postkit::pipeline::EncodeRunOptions {
            fps: postkit::encode::FrameRate::whole(FPS),
            picture: resolved.processing.clone(),
            ..postkit::pipeline::EncodeRunOptions::default()
        },
        &cancel,
        &pause,
        |_progress| {},
        |_message| {},
    )
    .expect("the fitted encode runs");
    assert!(encoded.frames_encoded >= 1, "no frames were encoded");

    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Fitting".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        container_width: TWO_K_SCOPE.0,
        container_height: TWO_K_SCOPE.1,
        output_dir: out.clone(),
        j2k_dir: Some(encoded.j2k_dir),
        audio_path: Some(make_wav(&dir.path().join("audio.wav"))),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "the fitted DCP must package");

    assert_eq!(
        essence_raster(&out),
        TWO_K_RASTER,
        "the encoder wrote the forced raster, not the source size"
    );
    let cpl = read_cpl(&out);
    assert_eq!(area(&cpl, "MainPictureStoredArea"), TWO_K_RASTER);
    assert_eq!(
        area(&cpl, "MainPictureActiveArea"),
        TWO_K_SCOPE,
        "the container is masked out of the raster: {cpl}"
    );
}
