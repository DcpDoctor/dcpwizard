use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 3;
const FRAME_RATE: u32 = 24;
const DCI_PRECISION_BITS: u8 = 12;
const DCI_COMPONENTS: usize = 3;
const CODESTREAM_BUFFER_BYTES: usize = 4 * 1024 * 1024;

struct SourceContainer {
    file_name: &'static str,
    encoder_args: &'static [&'static str],
    // what ffprobe calls the codec, which is what create routes on
    probed_codec: &'static str,
}

const SOURCE_CONTAINERS: [SourceContainer; 5] = [
    SourceContainer {
        file_name: "prores.mov",
        encoder_args: &[
            "-c:v",
            "prores_ks",
            "-profile:v",
            "2",
            "-pix_fmt",
            "yuv422p10le",
        ],
        probed_codec: "prores",
    },
    SourceContainer {
        file_name: "h265.mp4",
        encoder_args: &[
            "-c:v",
            "libx265",
            "-x265-params",
            "log-level=none",
            "-pix_fmt",
            "yuv420p",
        ],
        probed_codec: "hevc",
    },
    SourceContainer {
        file_name: "dnxhr.mov",
        encoder_args: &[
            "-c:v",
            "dnxhd",
            "-profile:v",
            "dnxhr_sq",
            "-pix_fmt",
            "yuv422p",
        ],
        probed_codec: "dnxhd",
    },
    SourceContainer {
        file_name: "source.mxf",
        encoder_args: &["-c:v", "mpeg2video"],
        probed_codec: "mpeg2video",
    },
    SourceContainer {
        file_name: "source.avi",
        encoder_args: &["-c:v", "mpeg4"],
        probed_codec: "mpeg4",
    },
];

fn write_source(directory: &Path, container: &SourceContainer) -> PathBuf {
    let path = directory.join(container.file_name);
    let seconds = FRAMES as f64 / f64::from(FRAME_RATE);
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "testsrc=size={WIDTH}x{HEIGHT}:rate={FRAME_RATE}:duration={seconds}"
        ))
        .args(container.encoder_args)
        .arg(&path)
        .output()
        .expect("ffmpeg has to run");
    assert!(
        made.status.success(),
        "ffmpeg could not write {}: {}",
        path.display(),
        String::from_utf8_lossy(&made.stderr)
    );
    path
}

fn only_file_starting_with(directory: &Path, prefix: &str) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .expect("the package directory has to be readable")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    found.sort();
    assert_eq!(found.len(), 1, "one {prefix}* in {}", directory.display());
    found.remove(0)
}

// every frame is decoded, so a header the packager wrote cannot pass for picture
fn assert_picture_decodes(picture_mxf: &Path, source: &str) {
    assert_eq!(
        asdcplib::essence_type(&picture_mxf.to_string_lossy()).expect("essence type"),
        asdcplib::EssenceType::Jpeg2000,
        "{source} must package as an AS-DCP JPEG 2000 track file"
    );
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&picture_mxf.to_string_lossy())
        .expect("the picture MXF has to open");
    let descriptor = reader.picture_descriptor().expect("picture descriptor");
    assert_eq!(
        descriptor.container_duration, FRAMES as u32,
        "{source} must package {FRAMES} frames"
    );

    let mut buffer = vec![0u8; CODESTREAM_BUFFER_BYTES];
    for index in 0..FRAMES as u32 {
        let read = reader
            .read_frame(index, &mut buffer, None, None)
            .unwrap_or_else(|e| panic!("{source} frame {index} has to read: {e}"));
        let frame = postkit::grok_decoder::decode(buffer[..read].to_vec(), 0)
            .unwrap_or_else(|e| panic!("{source} frame {index} has to decode: {e}"));
        assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT), "{source}");
        assert_eq!(frame.precision, DCI_PRECISION_BITS, "{source}");
        assert_eq!(frame.components.len(), DCI_COMPONENTS, "{source}");
        assert!(
            frame.components[0].iter().any(|&code| code > 0),
            "{source} frame {index} decoded to an all-black picture"
        );
    }
}

#[test]
fn create_packages_every_source_container_the_readme_names() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();

    for container in &SOURCE_CONTAINERS {
        let source = write_source(directory.path(), container);
        assert_eq!(
            dcpwizard_core::probe::source_video_codec(&source).expect("ffprobe reads the codec"),
            container.probed_codec,
            "{} has to reach create as {}",
            container.file_name,
            container.probed_codec
        );

        let out = directory
            .path()
            .join(format!("dcp_{}", container.probed_codec));
        Command::cargo_bin("dcpwizard")
            .unwrap()
            .env("XDG_CONFIG_HOME", config_home.path())
            .args([
                "create",
                "--title",
                "Container",
                "--video",
                source.to_str().unwrap(),
                "-o",
                out.to_str().unwrap(),
                "--twok",
            ])
            .assert()
            .success();

        assert_picture_decodes(
            &only_file_starting_with(&out, "picture_"),
            container.file_name,
        );
        assert!(
            dcpwizard_core::verify::verify_dcp(&out).valid,
            "{} must package a valid DCP",
            container.file_name
        );
    }
}
