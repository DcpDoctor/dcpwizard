//! `create --burn-subtitle`: the refusal matrix, and a burnt-in still that
//! packages into a dcpdoctor-clean DCP.
//!
//! Most picture assertions live in postkit (`tests/subtitle_burn_e2e.rs`), which
//! can decode a codestream back to pixels. What matters here is that the flag
//! combinations fail before an encode starts, that a burnt hold produces a real
//! package, and that the appearance flags reach the pixels.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::subtitle::{check_burn_supported, prepare_subtitle_burn};
use postkit::encode::FrameRate;
use postkit::still::StillHold;
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

const DISPLAY_RGB: postkit::encode::SourceColour = postkit::encode::SourceColour::DisplayRgb;

fn write_srt(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("cues.srt");
    std::fs::write(&path, SRT).unwrap();
    path
}

#[test]
fn a_burn_is_refused_wherever_it_would_be_drawn_in_the_wrong_place() {
    let dir = tempfile::tempdir().unwrap();
    let srt = write_srt(dir.path());

    check_burn_supported(&srt, &[], &DISPLAY_RGB, false).expect("a plain display-RGB burn is fine");
    check_burn_supported(
        &srt,
        &[dir.path().join("other.srt").as_path()],
        &DISPLAY_RGB,
        false,
    )
    .expect("a different timed-text file is fine");

    let missing = dir.path().join("nope.srt");
    for (label, result, needle) in [
        (
            "missing file",
            check_burn_supported(&missing, &[], &DISPLAY_RGB, false),
            "not found",
        ),
        (
            "same file as --subtitle",
            check_burn_supported(&srt, &[srt.as_path()], &DISPLAY_RGB, false),
            "pick one",
        ),
        (
            "J2K input",
            check_burn_supported(&srt, &[], &DISPLAY_RGB, true),
            "already compressed",
        ),
        (
            "frames already X'Y'Z'",
            check_burn_supported(&srt, &[], &postkit::encode::SourceColour::AlreadyPq, false),
            "X'Y'Z' already",
        ),
        (
            "an HDR master the encoder gets as PQ samples",
            check_burn_supported(
                &srt,
                &[],
                &postkit::encode::SourceColour::HdrDcdm {
                    source: postkit::colour::HdrSource::Hdr10,
                    source_peak_nits: postkit::colour::HdrSource::DEFAULT_PEAK_NITS,
                },
                false,
            ),
            "PQ-encoded HDR samples",
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
        "ffmpeg wrote no card\n  stdout: {}\n  stderr: {}",
        String::from_utf8_lossy(&made.stdout).trim(),
        String::from_utf8_lossy(&made.stderr).trim(),
    );

    let srt = write_srt(dir.path());
    let burn = prepare_subtitle_burn(&srt, None, FPS, &BurnStyleOverrides::default())
        .expect("a system font to burn with");

    // 3 seconds of hold over cues at 0-1s and 2-3s: the picture changes at
    // frames 24, 48 and 72, so four distinct frames are encoded.
    let held_frames = 3 * FPS.numerator as u64;
    let j2k = dir.path().join("j2k");
    postkit::still::build_still_frames(&StillHold {
        image: &card,
        frames: held_frames,
        fps: FPS,
        width: W,
        height: H,
        filters: &[],
        apply_xyz_transform: true,
        rsiz: postkit::encode::default_rsiz(),
        colour_transform: None,
        burn: Some(burn),
        watermark: None,
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

/// Decode one codestream in memory and return its samples pixel-interleaved
/// at the codestream's own 12 bits.
fn decode_frame(codestream: &Path) -> Vec<u16> {
    let data = std::fs::read(codestream).expect("codestream");
    postkit::grok_decoder::decode(data, 0)
        .unwrap_or_else(|e| panic!("cannot decode {}: {e}", codestream.display()))
        .interleaved_samples()
        .expect("three components")
}

/// Yellow outlined text on a grey card, read back out of the encoded frame.
///
/// The frames land in X'Y'Z', where yellow keeps its signature: far more Y than
/// the card has and almost no Z, while the black outline sits below the card on
/// every component.
#[test]
fn a_styled_burn_lands_yellow_text_over_a_black_outline() {
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
        "ffmpeg wrote no card\n  stdout: {}\n  stderr: {}",
        String::from_utf8_lossy(&made.stdout).trim(),
        String::from_utf8_lossy(&made.stderr).trim(),
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
    let burn = prepare_subtitle_burn(&srt, None, FPS, &style).expect("a system font to burn with");

    // the first cue runs 0-1s, so every frame of this hold carries it
    let j2k = dir.path().join("j2k");
    postkit::still::build_still_frames(&StillHold {
        image: &card,
        frames: 2,
        fps: FPS,
        width: W,
        height: H,
        filters: &[],
        apply_xyz_transform: true,
        rsiz: postkit::encode::default_rsiz(),
        colour_transform: None,
        burn: Some(burn),
        watermark: None,
        out_dir: &j2k,
    })
    .expect("burnt still");

    let samples = decode_frame(&j2k.join("frame_00000000.j2c"));
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
    for pixel in samples.as_chunks::<3>().0 {
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
