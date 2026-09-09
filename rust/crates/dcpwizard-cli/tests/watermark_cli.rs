use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAME_RATE: u32 = 24;
const FRAMES: u32 = 3;
const CODESTREAM_BUFFER_BYTES: usize = 4 * 1024 * 1024;

const MARK_TEXT: &str = "SCREENER DIST-001";
// text height as a percent of the frame, well above the default so the band it
// draws in is unmistakable
const MARK_FONT_SIZE_PERCENT: f32 = 8.0;
const PERCENT_DIVISOR: f32 = 100.0;

// how far a sample has to rise over the flat source to count as drawn on, a
// quarter of the 12-bit range, so a codec artefact cannot pass for the mark
const MARK_MINIMUM_RISE: i32 = 1024;
// about the samples a line of text at MARK_FONT_SIZE_PERCENT covers on a 2K frame
const MARK_MINIMUM_SAMPLES: usize = 500;

fn dcpwizard(config_home: &Path) -> Command {
    let mut command = Command::cargo_bin("dcpwizard").unwrap();
    command.env("XDG_CONFIG_HOME", config_home);
    command
}

// a flat field, so anything drawn on it stands out of an otherwise constant frame
fn grey_source(directory: &Path) -> PathBuf {
    let path = directory.join("grey.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=gray:s={WIDTH}x{HEIGHT}:rate={FRAME_RATE}"),
            "-frames:v",
            &FRAMES.to_string(),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .output()
        .expect("ffmpeg has to run");
    assert!(
        made.status.success(),
        "ffmpeg could not write the grey source: {}",
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

// the first row the mark can reach: its line box, plus a line height of glyph
// overhang and drop shadow, above the margin it is anchored at
fn first_marked_row() -> usize {
    let line_height = MARK_FONT_SIZE_PERCENT / PERCENT_DIVISOR
        * postkit::subtitle_raster::DEFAULT_LINE_HEIGHT_RATIO;
    let from_bottom = postkit::subtitle_raster::DEFAULT_MARGIN_RATIO + line_height * 2.0;
    ((1.0 - from_bottom) * HEIGHT as f32).floor() as usize
}

fn decode_frame0(picture_mxf: &Path) -> Vec<Vec<i32>> {
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&picture_mxf.to_string_lossy())
        .expect("the picture MXF has to open");
    let mut buffer = vec![0u8; CODESTREAM_BUFFER_BYTES];
    let read = reader
        .read_frame(0, &mut buffer, None, None)
        .expect("the first codestream has to read");
    let decoded =
        postkit::grok_decoder::decode(buffer[..read].to_vec(), 0).expect("frame 0 has to decode");
    assert_eq!((decoded.width, decoded.height), (WIDTH, HEIGHT));
    decoded.components
}

fn picture_is_encrypted(picture_mxf: &Path) -> bool {
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&picture_mxf.to_string_lossy())
        .expect("the picture MXF has to open");
    reader.writer_info().expect("writer info").encrypted_essence
}

// samples raised over the flat source, split by whether they land in the band
// the mark is anchored in
fn raised_samples(frame: &[Vec<i32>]) -> (usize, usize) {
    let width = WIDTH as usize;
    let band = first_marked_row();
    let mut in_band = 0usize;
    let mut above_band = 0usize;
    for component in frame {
        // the source is one flat colour, so the median is its own code value
        let mut sorted = component.clone();
        sorted.sort_unstable();
        let background = sorted[sorted.len() / 2];
        for (at, &code) in component.iter().enumerate() {
            if code - background < MARK_MINIMUM_RISE {
                continue;
            }
            if at / width >= band {
                in_band += 1;
            } else {
                above_band += 1;
            }
        }
    }
    (in_band, above_band)
}

fn create_grey_dcp(source: &Path, out: &Path, config_home: &Path, extra: &[&str]) {
    let mut command = dcpwizard(config_home);
    command.args([
        "create",
        "--title",
        "Marked",
        "--video",
        source.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--twok",
    ]);
    command.args(extra).assert().success();
}

#[test]
fn create_watermark_draws_the_mark_into_the_packaged_picture() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let source = grey_source(directory.path());

    let plain = directory.path().join("plain");
    create_grey_dcp(&source, &plain, config_home.path(), &[]);
    let marked = directory.path().join("marked");
    create_grey_dcp(
        &source,
        &marked,
        config_home.path(),
        &[
            "--watermark",
            MARK_TEXT,
            "--watermark-font-size",
            &MARK_FONT_SIZE_PERCENT.to_string(),
        ],
    );

    let (unmarked_in_band, unmarked_above) =
        raised_samples(&decode_frame0(&only_file_starting_with(&plain, "picture_")));
    assert_eq!(
        (unmarked_in_band, unmarked_above),
        (0, 0),
        "a build without --watermark must leave the flat source flat"
    );

    let (in_band, above_band) = raised_samples(&decode_frame0(&only_file_starting_with(
        &marked, "picture_",
    )));
    assert_eq!(
        above_band,
        0,
        "--watermark drew above row {}, outside the band it anchors in",
        first_marked_row()
    );
    assert!(
        in_band >= MARK_MINIMUM_SAMPLES,
        "--watermark raised {in_band} samples, fewer than the {MARK_MINIMUM_SAMPLES} a line of \
         text covers"
    );
}

#[test]
fn the_watermark_command_marks_an_encrypted_source_under_its_keys() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let source = grey_source(directory.path());

    let encrypted = directory.path().join("encrypted");
    let keys = directory.path().join("KEYS.json");
    create_grey_dcp(
        &source,
        &encrypted,
        config_home.path(),
        &["--encrypt", "--key-out", keys.to_str().unwrap()],
    );
    assert!(
        picture_is_encrypted(&only_file_starting_with(&encrypted, "picture_")),
        "the source package has to be encrypted for the key path to mean anything"
    );

    let marked = directory.path().join("marked");
    dcpwizard(config_home.path())
        .args([
            "watermark",
            "--input",
            encrypted.to_str().unwrap(),
            "--output",
            marked.to_str().unwrap(),
            "--payload",
            MARK_TEXT,
            "--font-size",
            &MARK_FONT_SIZE_PERCENT.to_string(),
            "--keys",
            keys.to_str().unwrap(),
        ])
        .assert()
        .success();

    let marked_picture = only_file_starting_with(&marked, "picture_");
    assert!(
        !picture_is_encrypted(&marked_picture),
        "the marked package is cleartext"
    );
    let (in_band, above_band) = raised_samples(&decode_frame0(&marked_picture));
    assert_eq!(
        above_band,
        0,
        "the mark drew above row {}, outside the band it anchors in",
        first_marked_row()
    );
    assert!(
        in_band >= MARK_MINIMUM_SAMPLES,
        "the mark raised {in_band} samples of the decrypted picture, fewer than the \
         {MARK_MINIMUM_SAMPLES} a line of text covers"
    );

    let verified = dcpwizard_core::verify::verify_dcp(&marked);
    assert!(verified.valid, "dcpdoctor errors: {:?}", verified.errors);

    // the same source with no key material has nothing to decrypt with
    dcpwizard(config_home.path())
        .args([
            "watermark",
            "--input",
            encrypted.to_str().unwrap(),
            "--output",
            directory.path().join("nokeys").to_str().unwrap(),
            "--payload",
            MARK_TEXT,
        ])
        .assert()
        .failure();
}
