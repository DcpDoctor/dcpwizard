// `export` over a DCP built from one flat colour: the screener has to come back the colour of
// the master, carry the raster and frame count of the DCP, and hold PCM under ProRes.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;

const SOURCE_COLOUR: &str = "0x5a8f3c";
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FRAME_RATE: u32 = 24;
const DURATION_SECONDS: f64 = 0.5;
const FRAMES: u32 = 12;

// how far the export may sit from the DCP picture it was made from, the only drift the export
// itself owns: a matrix or transfer mistake moves a channel by 4 percent or more
const PICTURE_TOLERANCE: f64 = 0.03;

// the master to X'Y'Z' to J2K trip costs 3.9 percent per channel on this fixture before the
// export runs at all, so the whole chain gets the wider bound
const MASTER_TOLERANCE: f64 = 0.06;

const SIXTEEN_BIT_PER_EIGHT_BIT: f64 = 257.0;

struct Fixture {
    directory: TempDir,
    config_home: TempDir,
    master: PathBuf,
    picture_mxf: PathBuf,
    sound_mxf: PathBuf,
}

fn ffmpeg() -> std::process::Command {
    let mut command = std::process::Command::new("ffmpeg");
    command.args(["-hide_banner", "-v", "error", "-y"]);
    command
}

fn run(command: &mut std::process::Command, what: &str) {
    let output = command.output().unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(
        output.status.success(),
        "{what}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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

fn dcp_fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(build_dcp)
}

fn build_dcp() -> Fixture {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();

    let master = directory.path().join("master.mkv");
    run(
        ffmpeg()
            .args(["-f", "lavfi", "-i"])
            .arg(format!(
                "color=c={SOURCE_COLOUR}:size={WIDTH}x{HEIGHT}:rate={FRAME_RATE}:duration={DURATION_SECONDS}"
            ))
            .args(["-c:v", "ffv1", "-pix_fmt", "gbrp"])
            .arg(&master),
        "ffmpeg must be installed to write the flat colour master",
    );

    let wav = directory.path().join("tone.wav");
    run(
        ffmpeg()
            .args(["-f", "lavfi", "-i"])
            .arg(format!(
                "sine=frequency=440:duration={DURATION_SECONDS}:sample_rate=48000"
            ))
            .args(["-ac", "2", "-c:a", "pcm_s24le"])
            .arg(&wav),
        "the tone has to be written",
    );

    let package = directory.path().join("dcp");
    Command::cargo_bin("dcpwizard")
        .unwrap()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args([
            "create",
            "--title",
            "Export Colour",
            "--video",
            master.to_str().unwrap(),
            "--audio",
            wav.to_str().unwrap(),
            "-o",
            package.to_str().unwrap(),
        ])
        .assert()
        .success();

    let picture_mxf = only_file_starting_with(&package, "picture_");
    let sound_mxf = only_file_starting_with(&package, "sound_");
    Fixture {
        directory,
        config_home,
        master,
        picture_mxf,
        sound_mxf,
    }
}

fn export(fixture: &Fixture, format: &str, output: &Path) {
    Command::cargo_bin("dcpwizard")
        .unwrap()
        .env("XDG_CONFIG_HOME", fixture.config_home.path())
        .args([
            "export",
            "--input",
            fixture.picture_mxf.to_str().unwrap(),
            "--audio",
            fixture.sound_mxf.to_str().unwrap(),
            "--format",
            format,
            "-o",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
}

fn probe(file: &Path, stream: &str, entries: &str) -> Vec<String> {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", stream])
        .arg("-show_entries")
        .arg(format!("stream={entries}"))
        .args(["-of", "default=nk=1:nw=1"])
        .arg(file)
        .output()
        .expect("ffprobe must be installed");
    assert!(
        output.status.success(),
        "ffprobe failed on {}: {}",
        file.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    // ffprobe 9 on macos prints the stream section twice, so keep one value per entry
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .take(entries.split(',').count())
        .collect()
}

// the mean of the first frame in RGB. A player reads an HD file as Rec.709, so the readback
// forces that matrix instead of trusting swscale's unspecified-means-601 default, and the
// accurate flags keep swscale's fast 8-bit path from moving red by 3 codes on its own.
fn mean_rgb(file: &Path) -> [f64; 3] {
    let output = ffmpeg()
        .arg("-i")
        .arg(file)
        .args(["-frames:v", "1"])
        .args(["-sws_flags", "+accurate_rnd+full_chroma_int"])
        .args(["-vf", "scale=in_color_matrix=bt709,format=rgb48le"])
        .args(["-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg must be installed");
    assert!(
        output.status.success(),
        "could not decode {}: {}",
        file.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let samples: Vec<u16> = output
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    assert_eq!(
        samples.len(),
        (WIDTH * HEIGHT * 3) as usize,
        "a full {WIDTH}x{HEIGHT} frame has to come back from {}",
        file.display()
    );
    let pixels = (samples.len() / 3) as f64;
    [0, 1, 2].map(|channel| {
        samples
            .iter()
            .skip(channel)
            .step_by(3)
            .map(|&s| s as f64)
            .sum::<f64>()
            / pixels
            / SIXTEEN_BIT_PER_EIGHT_BIT
    })
}

fn assert_colour_within(measured: [f64; 3], reference: [f64; 3], tolerance: f64, what: &str) {
    for channel in 0..3 {
        let drift = (measured[channel] - reference[channel]).abs() / reference[channel];
        assert!(
            drift <= tolerance,
            "{what}: channel {channel} reads {:.2} against {:.2}, {:.1} percent off, over the {:.1} percent allowed. \
             Measured RGB {measured:.2?} against {reference:.2?}",
            measured[channel],
            reference[channel],
            drift * 100.0,
            tolerance * 100.0
        );
    }
}

fn assert_rec709_tags(file: &Path) {
    assert_eq!(
        probe(
            file,
            "v:0",
            "color_range,color_space,color_transfer,color_primaries"
        ),
        ["tv", "bt709", "bt709", "bt709"],
        "the export has to name the colour a player must assume, or it reads the picture as BT.601"
    );
}

#[test]
fn a_prores_export_holds_the_dcp_colour_and_pcm_audio() {
    let fixture = dcp_fixture();
    let output = fixture.directory.path().join("screener.mov");
    export(fixture, "prores", &output);

    assert_eq!(
        probe(&output, "v:0", "codec_name,profile"),
        ["prores", "HQ"]
    );
    assert_eq!(
        probe(&output, "v:0", "width,height,nb_frames"),
        [WIDTH.to_string(), HEIGHT.to_string(), FRAMES.to_string()]
    );
    // 422 HQ is a 4:2:2 codec, so a 4:4:4 frame under the apch tag is not a ProRes a grade reads
    assert_eq!(probe(&output, "v:0", "pix_fmt"), ["yuv422p10le"]);
    assert_eq!(probe(&output, "a:0", "codec_name"), ["pcm_s24le"]);
    assert_rec709_tags(&output);

    let exported = mean_rgb(&output);
    assert_colour_within(
        exported,
        mean_rgb(&fixture.picture_mxf),
        PICTURE_TOLERANCE,
        "the ProRes export against the DCP picture",
    );
    assert_colour_within(
        exported,
        mean_rgb(&fixture.master),
        MASTER_TOLERANCE,
        "the ProRes export against the master",
    );
}

#[test]
fn an_h264_export_holds_the_dcp_colour_and_aac_audio() {
    let fixture = dcp_fixture();
    let output = fixture.directory.path().join("screener.mp4");
    export(fixture, "h264", &output);

    assert_eq!(
        probe(&output, "v:0", "codec_name,profile"),
        ["h264", "High"]
    );
    assert_eq!(
        probe(&output, "v:0", "width,height,nb_frames"),
        [WIDTH.to_string(), HEIGHT.to_string(), FRAMES.to_string()]
    );
    // 8-bit 4:2:0 plays everywhere a screener is opened, High 4:4:4 10-bit does not
    assert_eq!(probe(&output, "v:0", "pix_fmt"), ["yuv420p"]);
    assert_eq!(probe(&output, "a:0", "codec_name"), ["aac"]);
    assert_rec709_tags(&output);

    let exported = mean_rgb(&output);
    assert_colour_within(
        exported,
        mean_rgb(&fixture.picture_mxf),
        PICTURE_TOLERANCE,
        "the H.264 export against the DCP picture",
    );
    assert_colour_within(
        exported,
        mean_rgb(&fixture.master),
        MASTER_TOLERANCE,
        "the H.264 export against the master",
    );
}

#[test]
fn an_h265_export_holds_the_dcp_colour_and_aac_audio() {
    let fixture = dcp_fixture();
    let output = fixture.directory.path().join("screener_h265.mp4");
    export(fixture, "h265", &output);

    assert_eq!(probe(&output, "v:0", "codec_name"), ["hevc"]);
    assert_eq!(
        probe(&output, "v:0", "width,height,nb_frames"),
        [WIDTH.to_string(), HEIGHT.to_string(), FRAMES.to_string()]
    );
    assert_eq!(probe(&output, "v:0", "pix_fmt"), ["yuv420p"]);
    assert_eq!(probe(&output, "a:0", "codec_name"), ["aac"]);
    assert_rec709_tags(&output);

    let exported = mean_rgb(&output);
    assert_colour_within(
        exported,
        mean_rgb(&fixture.picture_mxf),
        PICTURE_TOLERANCE,
        "the H.265 export against the DCP picture",
    );
    assert_colour_within(
        exported,
        mean_rgb(&fixture.master),
        MASTER_TOLERANCE,
        "the H.265 export against the master",
    );
}

#[test]
fn a_dnxhr_export_holds_the_dcp_colour_and_pcm_audio() {
    let fixture = dcp_fixture();
    let output = fixture.directory.path().join("screener.mxf");
    export(fixture, "dnxhr", &output);

    assert_eq!(probe(&output, "v:0", "codec_name"), ["dnxhd"]);
    assert_eq!(
        probe(&output, "v:0", "width,height"),
        [WIDTH.to_string(), HEIGHT.to_string()]
    );
    // DNxHR HQ is 4:2:2 8-bit, which is the layout a master for approval carries
    assert_eq!(probe(&output, "v:0", "pix_fmt"), ["yuv422p"]);
    assert_eq!(probe(&output, "a:0", "codec_name"), ["pcm_s24le"]);
    assert_rec709_tags(&output);

    let exported = mean_rgb(&output);
    assert_colour_within(
        exported,
        mean_rgb(&fixture.picture_mxf),
        PICTURE_TOLERANCE,
        "the DNxHR export against the DCP picture",
    );
    assert_colour_within(
        exported,
        mean_rgb(&fixture.master),
        MASTER_TOLERANCE,
        "the DNxHR export against the master",
    );
}

#[test]
fn an_image_sequence_export_writes_a_readable_png_per_frame() {
    let fixture = dcp_fixture();
    let output = fixture.directory.path().join("stills");
    export(fixture, "image-sequence", &output);

    let mut frames: Vec<PathBuf> = std::fs::read_dir(&output)
        .expect("the sequence directory has to be readable")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    frames.sort();
    assert_eq!(
        frames.len(),
        FRAMES as usize,
        "one still per packaged frame in {}",
        output.display()
    );

    let first = &frames[0];
    assert_eq!(probe(first, "v:0", "codec_name"), ["png"]);
    assert_eq!(probe(first, "v:0", "pix_fmt"), ["rgb48be"]);
    assert_eq!(
        probe(first, "v:0", "width,height"),
        [WIDTH.to_string(), HEIGHT.to_string()]
    );
    assert_colour_within(
        mean_rgb(first),
        mean_rgb(&fixture.picture_mxf),
        PICTURE_TOLERANCE,
        "the first still against the DCP picture",
    );
}

#[test]
fn exporting_something_that_is_not_a_track_file_says_so() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let not_a_dcp = directory.path().join("holiday_photos");
    std::fs::create_dir(&not_a_dcp).unwrap();

    Command::cargo_bin("dcpwizard")
        .unwrap()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args([
            "export",
            "--input",
            not_a_dcp.to_str().unwrap(),
            "-o",
            directory.path().join("out.mp4").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("holiday_photos"));
}
