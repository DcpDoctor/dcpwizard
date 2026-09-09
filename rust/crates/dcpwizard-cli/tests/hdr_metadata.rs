use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 256;
const HEIGHT: u32 = 144;
const FRAME_RATE: u32 = 24;
const FRAMES: u32 = 4;

const MAX_CLL: u16 = 1200;
const MAX_FALL: u16 = 350;

const PQ_TAG: &str = "smpte2084";
const HLG_TAG: &str = "arib-std-b67";
const BT709_TAG: &str = "bt709";
const BT2020_PRIMARIES_TAG: &str = "bt2020";

struct ProbedRatio {
    key: &'static str,
    value: &'static str,
}

// ST 2086 over the units ffprobe counts them in, and the P3-D65 display
// hdr10-inject writes
const MASTERING_DISPLAY: [ProbedRatio; 10] = [
    ProbedRatio {
        key: "red_x",
        value: "34000/50000",
    },
    ProbedRatio {
        key: "red_y",
        value: "16000/50000",
    },
    ProbedRatio {
        key: "green_x",
        value: "13250/50000",
    },
    ProbedRatio {
        key: "green_y",
        value: "34500/50000",
    },
    ProbedRatio {
        key: "blue_x",
        value: "7500/50000",
    },
    ProbedRatio {
        key: "blue_y",
        value: "3000/50000",
    },
    ProbedRatio {
        key: "white_point_x",
        value: "15635/50000",
    },
    ProbedRatio {
        key: "white_point_y",
        value: "16450/50000",
    },
    ProbedRatio {
        key: "max_luminance",
        value: "10000000/10000",
    },
    ProbedRatio {
        key: "min_luminance",
        value: "1/10000",
    },
];

fn dcpwizard(config_home: &Path) -> Command {
    let mut command = Command::cargo_bin("dcpwizard").unwrap();
    command.env("XDG_CONFIG_HOME", config_home);
    command
}

fn run_ffmpeg(arguments: &[&str], what: &str) {
    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error"])
        .args(arguments)
        .output()
        .expect("ffmpeg has to run");
    assert!(
        made.status.success(),
        "ffmpeg could not write {what}: {}",
        String::from_utf8_lossy(&made.stderr)
    );
}

fn tagged_source(directory: &Path, name: &str, transfer: &str, primaries: &str) -> PathBuf {
    let path = directory.join(name);
    run_ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=size={WIDTH}x{HEIGHT}:rate={FRAME_RATE}"),
            "-frames:v",
            &FRAMES.to_string(),
            "-vf",
            &format!(
                "setparams=color_primaries={primaries}:color_trc={transfer}:colorspace=bt2020nc"
            ),
            "-pix_fmt",
            "yuv420p10le",
            "-c:v",
            "libx265",
            "-x265-params",
            "log-level=none",
            path.to_str().unwrap(),
        ],
        name,
    );
    path
}

fn ffprobe_entries(video: &Path, entries: &str, extra: &[&str]) -> String {
    let probed = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(extra)
        .args(["-show_entries", entries])
        .args(["-of", "json"])
        .arg(video)
        .output()
        .expect("ffprobe has to run");
    assert!(
        probed.status.success(),
        "ffprobe could not read {}: {}",
        video.display(),
        String::from_utf8_lossy(&probed.stderr)
    );
    String::from_utf8_lossy(&probed.stdout).into_owned()
}

fn colour_tags(video: &Path) -> (String, String) {
    let probed = ffprobe_entries(video, "stream=color_transfer,color_primaries", &[]);
    let field = |key: &str| {
        probed
            .split_once(&format!("\"{key}\": \""))
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(value, _)| value.to_string())
            .unwrap_or_else(|| panic!("{key} missing from {probed}"))
    };
    (field("color_transfer"), field("color_primaries"))
}

// the decoded picture itself, so a tag change with no tone map would fail
fn decoded_frames(video: &Path) -> Vec<u8> {
    let decoded = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(video)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("ffmpeg has to run");
    assert!(
        decoded.status.success(),
        "ffmpeg could not decode {}: {}",
        video.display(),
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert!(
        !decoded.stdout.is_empty(),
        "{} decoded to nothing",
        video.display()
    );
    decoded.stdout
}

#[test]
fn hdr10_inject_writes_a_mastering_display_and_light_levels_ffprobe_reads_back() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let source = tagged_source(directory.path(), "source.mp4", PQ_TAG, BT2020_PRIMARIES_TAG);
    let injected = directory.path().join("hdr10.mp4");

    dcpwizard(config_home.path())
        .args([
            "hdr10-inject",
            "-i",
            source.to_str().unwrap(),
            "-o",
            injected.to_str().unwrap(),
            "--max-cll",
            &MAX_CLL.to_string(),
            "--max-fall",
            &MAX_FALL.to_string(),
        ])
        .assert()
        .success();

    let probed = ffprobe_entries(
        &injected,
        "frame=side_data_list",
        &["-show_frames", "-read_intervals", "%+#1"],
    );
    assert!(
        probed.contains("Mastering display metadata"),
        "no ST 2086 side data: {probed}"
    );
    assert!(
        probed.contains("Content light level metadata"),
        "no CTA 861.3 side data: {probed}"
    );
    for item in &MASTERING_DISPLAY {
        assert!(
            probed.contains(&format!("\"{}\": \"{}\"", item.key, item.value)),
            "{} must read back as {}: {probed}",
            item.key,
            item.value
        );
    }
    assert!(
        probed.contains(&format!("\"max_content\": {MAX_CLL}")),
        "MaxCLL must read back as {MAX_CLL}: {probed}"
    );
    assert!(
        probed.contains(&format!("\"max_average\": {MAX_FALL}")),
        "MaxFALL must read back as {MAX_FALL}: {probed}"
    );
}

#[test]
fn hdr_convert_tone_maps_between_hdr10_hlg_and_sdr() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let hdr10 = tagged_source(directory.path(), "hdr10.mp4", PQ_TAG, BT2020_PRIMARIES_TAG);
    let hlg = tagged_source(directory.path(), "hlg.mp4", HLG_TAG, BT2020_PRIMARIES_TAG);

    for (source, target, expected_transfer) in [
        (&hdr10, "hlg", HLG_TAG),
        (&hdr10, "sdr", BT709_TAG),
        (&hlg, "hdr10", PQ_TAG),
    ] {
        let converted = directory.path().join(format!(
            "{}_to_{target}.mp4",
            source.file_stem().unwrap().to_string_lossy()
        ));
        dcpwizard(config_home.path())
            .args([
                "hdr-convert",
                "-i",
                source.to_str().unwrap(),
                "-o",
                converted.to_str().unwrap(),
                "--target",
                target,
            ])
            .assert()
            .success();

        let (transfer, _) = colour_tags(&converted);
        assert_eq!(
            transfer,
            expected_transfer,
            "converting {} to {target} must tag {expected_transfer}",
            source.display()
        );
        assert_ne!(
            decoded_frames(source),
            decoded_frames(&converted),
            "converting {} to {target} left the picture alone",
            source.display()
        );
    }
}
