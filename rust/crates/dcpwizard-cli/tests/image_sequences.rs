use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAME_RATE: u32 = 24;
const FRAMES: u32 = 3;
const DCI_PRECISION_BITS: u8 = 12;
const DCI_COMPONENTS: usize = 3;
const CODESTREAM_BUFFER_BYTES: usize = 4 * 1024 * 1024;

struct StillFormat {
    extension: &'static str,
    // ffmpeg's image encoders each take one of the source's pixel layouts
    pixel_format: &'static str,
}

const STILL_FORMATS: [StillFormat; 6] = [
    StillFormat {
        extension: "png",
        pixel_format: "rgb48be",
    },
    StillFormat {
        extension: "dpx",
        pixel_format: "rgb48le",
    },
    StillFormat {
        extension: "exr",
        pixel_format: "gbrpf32le",
    },
    StillFormat {
        extension: "bmp",
        pixel_format: "bgr24",
    },
    StillFormat {
        extension: "tiff",
        pixel_format: "rgb48le",
    },
    StillFormat {
        extension: "jpg",
        pixel_format: "yuvj420p",
    },
];

fn write_sequence(directory: &Path, format: &StillFormat) -> PathBuf {
    let sequence = directory.join(format.extension);
    std::fs::create_dir_all(&sequence).unwrap();
    let seconds = f64::from(FRAMES) / f64::from(FRAME_RATE);
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "testsrc=size={WIDTH}x{HEIGHT}:rate={FRAME_RATE}:duration={seconds}"
        ))
        .args(["-pix_fmt", format.pixel_format])
        .arg(sequence.join(format!("frame_%03d.{}", format.extension)))
        .output()
        .expect("ffmpeg has to run");
    assert!(
        made.status.success(),
        "ffmpeg could not write the {} sequence: {}",
        format.extension,
        String::from_utf8_lossy(&made.stderr)
    );
    assert_eq!(
        std::fs::read_dir(&sequence).unwrap().count(),
        FRAMES as usize,
        "one still per frame"
    );
    sequence
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

fn assert_picture_decodes(picture_mxf: &Path, source: &str) {
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&picture_mxf.to_string_lossy())
        .expect("the picture MXF has to open");
    assert_eq!(
        reader
            .picture_descriptor()
            .expect("picture descriptor")
            .container_duration,
        FRAMES,
        "a {source} sequence packages one frame per still"
    );
    let mut buffer = vec![0u8; CODESTREAM_BUFFER_BYTES];
    for index in 0..FRAMES {
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
fn create_packages_every_image_sequence_format_the_readme_names() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();

    for format in &STILL_FORMATS {
        let sequence = write_sequence(directory.path(), format);
        let out = directory.path().join(format!("dcp_{}", format.extension));
        Command::cargo_bin("dcpwizard")
            .unwrap()
            .env("XDG_CONFIG_HOME", config_home.path())
            .args([
                "create",
                "--title",
                "Sequence",
                "--video",
                sequence.to_str().unwrap(),
                "-o",
                out.to_str().unwrap(),
                "--twok",
            ])
            .assert()
            .success();

        assert_picture_decodes(&only_file_starting_with(&out, "picture_"), format.extension);
        assert!(
            dcpwizard_core::verify::verify_dcp(&out).valid,
            "a {} sequence must package a valid DCP",
            format.extension
        );
        // the concat list and the codestreams are scratch, not part of the package
        for scratch in ["frames.ffconcat", "j2k"] {
            assert!(
                !out.join(scratch).exists(),
                "{scratch} was left inside the {} package",
                format.extension
            );
        }
    }
}
