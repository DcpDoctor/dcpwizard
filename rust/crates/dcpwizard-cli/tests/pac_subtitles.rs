use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAME_RATE: u32 = 24;
const FRAMES: u32 = 96;

// the DCST the packager writes is frame based, so a cue lands within one frame
// of the millisecond PAC named
const ONE_FRAME_MS: u64 = 1000 / FRAME_RATE as u64 + 1;

const PAC_HEADER_BYTES: usize = 23;
// PAC counts a record's text from the length field, eight bytes before the text
const PAC_TEXT_LENGTH_OFFSET: u8 = 20;
const PAC_ALIGN_CENTRE: u8 = 0x02;
const PAC_VALIGN_BOTTOM: u8 = 10;
const PAC_SPOT_BYTE: u8 = 0x60;
const PAC_RECORD_MARKER: u8 = 0xFE;
const PAC_TEXT_START_MARKER: u8 = 0x03;
// enough zeroes after a record for the scan to leave it and reach the next one
const PAC_RECORD_PADDING: usize = 20;
const PAC_TRAILING_PADDING: usize = 40;

struct PacCue {
    // SSFF as PAC's own decimal pair, at its 25 fps frame rate
    start_seconds_frames: u16,
    end_seconds_frames: u16,
    text: &'static str,
    expected_start_ms: u64,
    expected_end_ms: u64,
}

const PAC_CUES: [PacCue; 2] = [
    PacCue {
        start_seconds_frames: 10,
        end_seconds_frames: 105,
        text: "HELLO",
        expected_start_ms: 400,
        expected_end_ms: 1200,
    },
    PacCue {
        start_seconds_frames: 200,
        end_seconds_frames: 310,
        text: "WORLD",
        expected_start_ms: 2000,
        expected_end_ms: 3400,
    },
];

fn decimal_pair(value: u16) -> [u8; 2] {
    [(value & 0xff) as u8, (value >> 8) as u8]
}

// one PAC paragraph: spot byte, the two HHMM/SSFF timecodes, the text length,
// the vertical alignment, then the record marker and the Latin text
fn pac_record(cue: &PacCue) -> Vec<u8> {
    let mut record = vec![PAC_SPOT_BYTE];
    for part in [0, cue.start_seconds_frames, 0, cue.end_seconds_frames] {
        record.extend_from_slice(&decimal_pair(part));
    }
    record.push(cue.text.len() as u8 + PAC_TEXT_LENGTH_OFFSET);
    record.push(0);
    record.push(PAC_VALIGN_BOTTOM);
    record.extend_from_slice(&[0, 0, 0]);
    record.push(PAC_RECORD_MARKER);
    record.push(PAC_ALIGN_CENTRE);
    record.push(PAC_TEXT_START_MARKER);
    record.extend_from_slice(cue.text.as_bytes());
    record.extend(std::iter::repeat_n(0u8, PAC_RECORD_PADDING));
    record
}

fn write_pac(path: &Path) {
    let mut bytes = vec![0u8; PAC_HEADER_BYTES];
    bytes[0] = 1;
    for cue in &PAC_CUES {
        bytes.extend(pac_record(cue));
    }
    bytes.extend(std::iter::repeat_n(0u8, PAC_TRAILING_PADDING));
    std::fs::write(path, bytes).unwrap();
}

fn write_source(directory: &Path) -> PathBuf {
    let path = directory.join("source.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=size={WIDTH}x{HEIGHT}:rate={FRAME_RATE}"),
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
        "ffmpeg could not write the source: {}",
        String::from_utf8_lossy(&made.stderr)
    );
    path
}

#[test]
fn a_pac_subtitle_file_reaches_the_packaged_timed_text_track() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let source = write_source(directory.path());
    let subtitle = directory.path().join("cues.pac");
    write_pac(&subtitle);
    let out = directory.path().join("dcp");

    Command::cargo_bin("dcpwizard")
        .unwrap()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args([
            "create",
            "--title",
            "Pac Subs",
            "--video",
            source.to_str().unwrap(),
            "--subtitle",
            subtitle.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--twok",
        ])
        .assert()
        .success();

    // read back through the CPL and the timed-text MXF, the only copy the
    // package keeps: the staged DCST XML is removed with the other scratch
    let cues = dcpwizard_core::subtitle_extract::extract_cues(&out)
        .expect("the packaged subtitle track has to read back");
    assert_eq!(cues.len(), PAC_CUES.len(), "{cues:?}");
    for (read, wrote) in cues.iter().zip(PAC_CUES.iter()) {
        assert_eq!(read.text, wrote.text, "{cues:?}");
        assert!(
            read.start_ms.abs_diff(wrote.expected_start_ms) <= ONE_FRAME_MS,
            "{} starts at {} ms against the {} ms the PAC named",
            wrote.text,
            read.start_ms,
            wrote.expected_start_ms
        );
        assert!(
            read.end_ms.abs_diff(wrote.expected_end_ms) <= ONE_FRAME_MS,
            "{} ends at {} ms against the {} ms the PAC named",
            wrote.text,
            read.end_ms,
            wrote.expected_end_ms
        );
    }
}
