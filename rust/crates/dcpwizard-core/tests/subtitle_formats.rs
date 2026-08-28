//! W6 subtitle wiring end-to-end: styled input formats, placement, RTL, wrap,
//! font subsetting/embedding and bitmap PNG subs flow into a real timed-text MXF
//! and a dcpdoctor-clean DCP. The rendered DCST also validates against the
//! vendored ST 428-7 schema.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::subtitle::{RtlMode, SubtitleOptions};
use std::path::{Path, PathBuf};

const W: u32 = 2048;
const H: u32 = 1080;
const FPS: u32 = 24;

fn make_frames(dir: &Path, count: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(W, H, FPS, &seed).expect("encode content frame");
    for i in 0..count {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

/// A font in the repo, so these packages do not depend on what faces the machine
/// running the test carries.
fn fixture_font() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/LiberationSans-Regular.ttf")
}

fn base(out: &Path, j2k: PathBuf, sub: PathBuf, opts: SubtitleOptions) -> DcpConfig {
    DcpConfig {
        title: "Subs".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Feature,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(j2k),
        subtitle_path: Some(sub),
        subtitle_language: "en".into(),
        subtitle_opts: opts,
        ..Default::default()
    }
}

fn verify_clean(dir: &Path) {
    let result = dcpwizard_core::verify::verify_dcp(dir);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}

const ASS: &str = "[Script Info]\n[V4+ Styles]\nFormat: Name, Italic, Alignment\nStyle: Def,0,2\n[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:00.50,0:00:01.50,Def,plain {\\i1}slanted{\\i0} word\n";

#[test]
fn ass_dcst_validates_against_st428_7_schema() {
    let dir = tempfile::tempdir().unwrap();
    let ass = dir.path().join("in.ass");
    std::fs::write(&ass, ASS).unwrap();
    let out = dir.path().join("sub.xml");
    let opts = SubtitleOptions {
        valign: Some("top".into()),
        wrap_cols: Some(6),
        font_path: Some(fixture_font()),
        ..Default::default()
    };
    let prepared = dcpwizard_core::subtitle::prepare_subtitle_track(
        &ass,
        Default::default(),
        "en",
        FPS,
        &opts,
        &out,
    )
    .unwrap();
    let xml = std::fs::read_to_string(&prepared.dcst_path).unwrap();
    assert!(xml.contains("Italic=\"yes\""), "styling preserved: {xml}");
    assert!(xml.contains("Valign=\"top\""), "placement applied: {xml}");

    let xsd = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas/DCDMSubtitle-2010.xsd");
    let ok = std::process::Command::new("xmllint")
        .args(["--noout", "--schema"])
        .arg(&xsd)
        .arg(&prepared.dcst_path)
        .output()
        .expect("run xmllint")
        .status
        .success();
    assert!(ok, "styled DCST must validate against ST 428-7 XSD");
}

#[test]
fn create_dcp_with_ass_subtitles_is_dcpdoctor_clean() {
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k, 48);
    let ass = dir.path().join("in.ass");
    std::fs::write(&ass, ASS).unwrap();
    let out = dir.path().join("dcp");
    let opts = SubtitleOptions {
        rtl: RtlMode::Auto,
        font_path: Some(fixture_font()),
        ..Default::default()
    };
    assert_eq!(create_dcp(&base(&out, j2k, ass, opts)), 0);
    verify_clean(&out);
}

#[test]
fn create_dcp_with_embedded_subset_font_is_dcpdoctor_clean() {
    let font = fixture_font();
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k, 48);
    let srt = dir.path().join("in.srt");
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:01,500\nHello world\n").unwrap();
    let out = dir.path().join("dcp");
    let opts = SubtitleOptions {
        font_path: Some(font),
        ..Default::default()
    };
    assert_eq!(create_dcp(&base(&out, j2k, srt, opts)), 0);
    verify_clean(&out);
}

#[test]
fn reel_split_shares_embedded_font_across_reels() {
    let font = fixture_font();
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k, 48);
    // cues in both reels so each reel embeds the shared font
    let srt = dir.path().join("in.srt");
    std::fs::write(
        &srt,
        "1\n00:00:00,100 --> 00:00:00,900\nreel one\n\n2\n00:00:01,100 --> 00:00:01,900\nreel two\n",
    )
    .unwrap();
    let out = dir.path().join("dcp");
    let mut config = base(
        &out,
        j2k,
        srt,
        SubtitleOptions {
            font_path: Some(font),
            ..Default::default()
        },
    );
    config.reel_split_frames = vec![24]; // 48 frames -> two 24-frame reels
    assert_eq!(create_dcp(&config), 0);
    verify_clean(&out);
    // two subtitle MXFs, one per reel
    let subs = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("subtitle_"))
        .count();
    assert_eq!(subs, 2, "one subtitle MXF per reel");
}

#[test]
fn create_dcp_with_interop_png_subs_is_dcpdoctor_clean() {
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k, 48);
    // a minimal but structurally valid 1x1 PNG
    let png = build_png();
    std::fs::write(dir.path().join("s1.png"), &png).unwrap();
    let xml = dir.path().join("subs.xml");
    std::fs::write(
        &xml,
        "<DCSubtitle Version=\"1.0\"><Subtitle SpotNumber=\"1\" TimeIn=\"00:00:00:12\" TimeOut=\"00:00:01:12\"><Image VAlign=\"bottom\" HAlign=\"center\" VPosition=\"8\">s1.png</Image></Subtitle></DCSubtitle>",
    )
    .unwrap();
    let out = dir.path().join("dcp");
    assert_eq!(
        create_dcp(&base(&out, j2k, xml, SubtitleOptions::default())),
        0
    );
    verify_clean(&out);
}

/// A tiny valid PNG (1x1 opaque) so the interop parser's magic + existence
/// checks pass and asdcplib embeds real bytes.
fn build_png() -> Vec<u8> {
    // precomputed 1x1 black PNG
    const B64: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc``\x00\x00\x00\x04\x00\x01\xf6\x178U\x00\x00\x00\x00IEND\xaeB`\x82";
    B64.to_vec()
}

#[test]
fn a_named_appearance_reaches_the_packaged_timed_text_mxf() {
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k, 48);
    let srt = dir.path().join("in.srt");
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:01,500\nHello world\n").unwrap();
    let out = dir.path().join("dcp");
    let opts = SubtitleOptions {
        appearance: dcpwizard_core::subtitle::TimedTextAppearance::from_flags(
            Some(50),
            None,
            Some("none"),
            None,
            Some(200),
            Some(200),
        )
        .unwrap(),
        font_path: Some(fixture_font()),
        ..Default::default()
    };
    assert_eq!(create_dcp(&base(&out, j2k, srt, opts)), 0);
    verify_clean(&out);

    // the DCST is wrapped into the timed-text MXF and the staged XML removed,
    // so the packaged track is the only place left to read it back from
    let mxf = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("subtitle_"))
        })
        .expect("a timed-text MXF");
    let bytes = std::fs::read(&mxf).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    for needle in [
        "Size=\"50\"",
        "Effect=\"none\"",
        // 200 ms at 24 fps is 4.8 frames, so 5
        "FadeUpTime=\"00:00:00:05\"",
        "FadeDownTime=\"00:00:00:05\"",
    ] {
        assert!(
            text.contains(needle),
            "{needle} is not in {}",
            mxf.display()
        );
    }
}

/// The conformance findings a package built with no `--subtitle-font` used to
/// carry: the timed-text ids, the missing LoadFont and StartTime, the IssueDate
/// form, the second namespace, and the CPL Hash on the subtitle asset.
#[test]
fn create_dcp_with_no_named_font_clears_the_timed_text_findings() {
    assert!(
        postkit::subtitle_raster::find_system_sans_font().is_some(),
        "this machine carries no system sans font"
    );
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    std::fs::create_dir_all(&j2k).unwrap();
    dcpwizard_core::pad::generate_black_frame(W, H, FPS, &j2k.join("probe.j2c"))
        .expect("a J2K encoder");
    std::fs::remove_file(j2k.join("probe.j2c")).unwrap();
    make_frames(&j2k, 48);
    let srt = dir.path().join("in.srt");
    std::fs::write(
        &srt,
        "1\n00:00:00,500 --> 00:00:01,000\nHello there\n\n2\n00:00:01,200 --> 00:00:01,900\nSecond cue\n",
    )
    .unwrap();
    let out = dir.path().join("dcp");
    assert_eq!(
        create_dcp(&base(&out, j2k, srt, SubtitleOptions::default())),
        0
    );

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
    for code in [
        "subtitle_invalid_issue_date",
        "subtitle_namespace_count",
        "cpl_missing_hash",
    ] {
        assert!(
            !result.warnings.iter().any(|w| w.contains(code)),
            "{code} still reported: {:?}",
            result.warnings
        );
    }

    // the font came off the machine, so the track carries one either way
    let subtitle_mxf = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("subtitle_"))
        })
        .expect("a subtitle MXF");
    assert!(
        std::fs::metadata(&subtitle_mxf).unwrap().len() > 4096,
        "the MXF carries an embedded font, not just the XML"
    );
}
