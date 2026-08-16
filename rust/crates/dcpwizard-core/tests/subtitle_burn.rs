//! `create --burn-subtitle`: the refusal matrix, and a burnt-in still that
//! packages into a dcpdoctor-clean DCP.
//!
//! Most picture assertions live in postkit (`tests/subtitle_burn_e2e.rs`), which
//! can decode a codestream back to pixels. What matters here is that the flag
//! combinations fail before an encode starts, that a burnt hold produces a real
//! package, and that the appearance flags reach the pixels.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::still::StillHold;
use dcpwizard_core::subtitle::{check_burn_supported, prepare_subtitle_burn};
use postkit::encode::FrameRate;
use postkit::subtitle_raster::BurnStyleOverrides;
use std::path::Path;

const W: u32 = 2048;
const H: u32 = 1080;
const FPS: FrameRate = FrameRate {
    numerator: 24,
    denominator: 1,
};

const SRT: &str = "1\n00:00:00,000 --> 00:00:01,000\nfirst line\n\n\
                   2\n00:00:02,000 --> 00:00:03,000\nsecond line\n\n";

fn have(bin: &str, arg: &str) -> bool {
    std::process::Command::new(bin)
        .arg(arg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn write_srt(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("cues.srt");
    std::fs::write(&path, SRT).unwrap();
    path
}

#[test]
fn a_burn_is_refused_wherever_it_would_be_drawn_in_the_wrong_place() {
    let dir = tempfile::tempdir().unwrap();
    let srt = write_srt(dir.path());

    check_burn_supported(&srt, None, false, false).expect("a plain display-RGB burn is fine");
    check_burn_supported(&srt, Some(&dir.path().join("other.srt")), false, false)
        .expect("a different timed-text file is fine");

    let missing = dir.path().join("nope.srt");
    for (label, result, needle) in [
        (
            "missing file",
            check_burn_supported(&missing, None, false, false),
            "not found",
        ),
        (
            "same file as --subtitle",
            check_burn_supported(&srt, Some(&srt), false, false),
            "pick one",
        ),
        (
            "J2K input",
            check_burn_supported(&srt, None, false, true),
            "already compressed",
        ),
        (
            "frames already X'Y'Z'",
            check_burn_supported(&srt, None, true, false),
            "X'Y'Z' already",
        ),
    ] {
        let err = result.expect_err(label);
        assert!(err.contains(needle), "{label}: got {err}");
    }
}

#[test]
fn smpte_dcst_xml_is_refused_with_a_message_naming_what_to_pass_instead() {
    let dir = tempfile::tempdir().unwrap();
    let xml = dir.path().join("subs.xml");
    std::fs::write(
        &xml,
        r#"<?xml version="1.0"?><SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST"></SubtitleReel>"#,
    )
    .unwrap();
    let err = prepare_subtitle_burn(&xml, None, FPS, &BurnStyleOverrides::default()).unwrap_err();
    assert!(err.contains("SMPTE DCST"), "got: {err}");
    assert!(
        err.contains("SRT"),
        "the message must name a way out: {err}"
    );
}

#[test]
fn a_missing_burn_in_font_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let srt = write_srt(dir.path());
    let font = dir.path().join("nothere.ttf");
    let err =
        prepare_subtitle_burn(&srt, Some(&font), FPS, &BurnStyleOverrides::default()).unwrap_err();
    assert!(err.contains("font not found"), "got: {err}");
}

/// A held still is the input shape with no decoder of its own, so this is the
/// one that proves the burn is not tied to the ffmpeg path.
#[test]
fn a_burnt_still_holds_one_codestream_per_cue_change_and_packages_clean() {
    if !have("ffmpeg", "-version") {
        eprintln!("skipping: ffmpeg not found");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let card = dir.path().join("card.png");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=gray:s={W}x{H}"),
            "-frames:v",
            "1",
        ])
        .arg(&card)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );

    let srt = write_srt(dir.path());
    let Ok(burn) = prepare_subtitle_burn(&srt, None, FPS, &BurnStyleOverrides::default()) else {
        eprintln!("skipping: no font available to burn with");
        return;
    };

    // 3 seconds of hold over cues at 0-1s and 2-3s: the picture changes at
    // frames 24, 48 and 72, so four distinct frames are encoded.
    let held_frames = 3 * FPS.numerator as u64;
    let j2k = dir.path().join("j2k");
    dcpwizard_core::still::build_still_frames(&StillHold {
        image: &card,
        frames: held_frames,
        fps: FPS,
        width: W,
        height: H,
        picture_filter: None,
        route: dcpwizard_core::encode::XyzRoute::CompressorTransform,
        burn: Some(burn),
        out_dir: &j2k,
    })
    .expect("burnt still");

    let landed = (0..held_frames)
        .filter(|i| j2k.join(format!("frame_{i:08}.j2c")).exists())
        .count() as u64;
    assert_eq!(landed, held_frames, "every frame of the hold needs a file");

    // Frames inside one cue's window repeat a single codestream; the frame
    // where the cue leaves is a different picture.
    let bytes = |index: u64| std::fs::read(j2k.join(format!("frame_{index:08}.j2c"))).unwrap();
    assert_eq!(bytes(0), bytes(12), "frames under one cue must be the same");
    assert_ne!(
        bytes(0),
        bytes(24),
        "the frame where the first cue ends must be a different picture"
    );
    assert_ne!(
        bytes(24),
        bytes(48),
        "the frame where the second cue starts must be a different picture"
    );

    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Burnt".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Feature,
        frame_rate_num: FPS.numerator,
        frame_rate_den: FPS.denominator,
        output_dir: out.clone(),
        j2k_dir: Some(j2k),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0);
    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);

    // The burn is in the picture, so nothing may register a subtitle track.
    let cpl = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().contains("CPL"))
        })
        .expect("a CPL");
    let xml = std::fs::read_to_string(&cpl).unwrap();
    assert!(
        !xml.contains("MainSubtitle"),
        "a burnt-in subtitle must not also be a timed-text track"
    );
}

fn find_grk_decompress() -> Option<std::path::PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        let path = std::path::PathBuf::from(home).join("bin/grok/bin/grk_decompress");
        if path.exists() {
            return Some(path);
        }
    }
    std::process::Command::new("which")
        .arg("grk_decompress")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| std::path::PathBuf::from(s.trim()))
}

/// Decode one codestream to a 16-bit-per-channel PPM and return its samples.
fn decode_frame(grk_decompress: &Path, codestream: &Path, out: &Path) -> Vec<u16> {
    let output = std::process::Command::new(grk_decompress)
        .env("LD_LIBRARY_PATH", postkit::grok::grok_lib_path())
        .args(["-i", &codestream.to_string_lossy()])
        .args(["-o", &out.to_string_lossy()])
        .output()
        .expect("grk_decompress");
    assert!(
        output.status.success(),
        "grk_decompress failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(out).expect("decoded ppm");
    // P6 header: magic, width, height, maxval, whitespace-separated, then one
    // whitespace byte before the raster.
    let mut at = 0usize;
    let mut fields = 0;
    while fields < 4 {
        while bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if bytes[at] == b'#' {
            while bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        while !bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        fields += 1;
    }
    at += 1;
    bytes[at..]
        .chunks_exact(2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
        .collect()
}

/// Yellow outlined text on a grey card, read back out of the encoded frame.
///
/// The frames land in X'Y'Z', where yellow keeps its signature: far more Y than
/// the card has and almost no Z, while the black outline sits below the card on
/// every component.
#[test]
fn a_styled_burn_lands_yellow_text_over_a_black_outline() {
    if !have("ffmpeg", "-version") {
        eprintln!("skipping: ffmpeg not found");
        return;
    }
    let Some(grk_decompress) = find_grk_decompress() else {
        eprintln!("skipping: grk_decompress not found");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let card = dir.path().join("card.png");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=gray:s={W}x{H}"),
            "-frames:v",
            "1",
        ])
        .arg(&card)
        .output()
        .expect("ffmpeg");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );

    let srt = write_srt(dir.path());
    let style = BurnStyleOverrides {
        font_size_percent: Some(8.0),
        colour: Some(postkit::subtitle_formats::Rgba {
            r: 255,
            g: 255,
            b: 0,
            a: 255,
        }),
        effect: Some(postkit::subtitle_raster::BurnEffect::Outline),
        ..Default::default()
    };
    let Ok(burn) = prepare_subtitle_burn(&srt, None, FPS, &style) else {
        eprintln!("skipping: no font available to burn with");
        return;
    };

    // the first cue runs 0-1s, so every frame of this hold carries it
    let j2k = dir.path().join("j2k");
    dcpwizard_core::still::build_still_frames(&StillHold {
        image: &card,
        frames: 2,
        fps: FPS,
        width: W,
        height: H,
        picture_filter: None,
        route: dcpwizard_core::encode::XyzRoute::CompressorTransform,
        burn: Some(burn),
        out_dir: &j2k,
    })
    .expect("burnt still");

    let samples = decode_frame(
        &grk_decompress,
        &j2k.join("frame_00000000.j2c"),
        &dir.path().join("frame.ppm"),
    );
    assert_eq!(samples.len(), (W * H * 3) as usize);

    // the top-left corner is card, nowhere near the bottom-anchored cue
    let card_x = samples[0] as u32;
    let card_y = samples[1] as u32;
    let card_z = samples[2] as u32;
    // the 1/2.6 gamma squashes both ratios, so yellow reads as a quarter more
    // luminance than the card with a tenth less blue, not as extremes
    let lit = card_y + card_y / 4;
    let unblued = card_z - card_z / 10;
    let mut yellow = 0u32;
    let mut outline = 0u32;
    for pixel in samples.chunks_exact(3) {
        let (x, y, z) = (pixel[0] as u32, pixel[1] as u32, pixel[2] as u32);
        if y > lit && z < unblued {
            yellow += 1;
        }
        if x * 2 < card_x && y * 2 < card_y && z * 2 < card_z {
            outline += 1;
        }
    }
    assert!(yellow > 100, "no yellow text in the frame: {yellow} pixels");
    assert!(
        outline > 100,
        "no black outline under the text: {outline} pixels"
    );
}
