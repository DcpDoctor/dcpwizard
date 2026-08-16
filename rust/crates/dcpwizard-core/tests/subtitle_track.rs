//! End-to-end subtitle track: SRT -> ST 428-7 DCST XML (schema-checked) ->
//! timed-text MXF -> CPL registration.

use dcpwizard_core::cpl::{CplConfig, CplReel, generate_cpl};
use dcpwizard_core::mxf_wrap::{MxfType, MxfWrapConfig, wrap_mxf_result};
use dcpwizard_core::subtitle::convert_srt_to_dcp_xml;
use std::path::Path;

const SRT: &str = "1\n00:00:01,000 --> 00:00:04,000\nHello world\n\n2\n00:00:05,500 --> 00:00:08,000\nSecond line\nwith two rows\n";

fn xmllint_available() -> bool {
    std::process::Command::new("xmllint")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn srt_wraps_into_a_registered_timed_text_track() {
    let dir = tempfile::tempdir().unwrap();
    let srt = dir.path().join("in.srt");
    std::fs::write(&srt, SRT).unwrap();

    // 1. SRT -> ST 428-7 DCST XML at 24 fps.
    let dcst = dir.path().join("sub.xml");
    convert_srt_to_dcp_xml(&srt, &dcst, "de", 24, 8.0).expect("srt->dcst");
    let xml = std::fs::read_to_string(&dcst).unwrap();
    // frame-based timecodes, not the illegal dot-millisecond form
    assert!(
        xml.contains("TimeOut=\"00:00:08:00\""),
        "frame timecode: {xml}"
    );

    // 2. Validate against the vendored ST 428-7 schema.
    if xmllint_available() {
        let xsd = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas/DCDMSubtitle-2010.xsd");
        let ok = std::process::Command::new("xmllint")
            .args(["--noout", "--schema"])
            .arg(&xsd)
            .arg(&dcst)
            .output()
            .expect("run xmllint")
            .status
            .success();
        assert!(ok, "DCST XML must validate against ST 428-7 XSD");
    }

    // 3. Wrap the DCST XML into a timed-text MXF (real asdcplib).
    let mxf = dir.path().join("sub.mxf");
    let track = wrap_mxf_result(&MxfWrapConfig {
        input_path: dcst.clone(),
        output_mxf: mxf.clone(),
        mxf_type: MxfType::TimedText,
        frame_rate: 24,
        ..Default::default()
    })
    .expect("timed-text wrap");
    assert!(mxf.exists(), "MXF written");
    // 8.000 s out at 24 fps = 192 frames
    assert_eq!(track.duration, 192, "duration from the subtitle timing");

    // 4. Register the track in a CPL and confirm MainSubtitle is present.
    let cpl_path = dir.path().join("CPL.xml");
    let reel = CplReel {
        reel_id: "11111111-1111-1111-1111-111111111111".into(),
        picture_id: "22222222-2222-2222-2222-222222222222".into(),
        picture_width: 1998,
        picture_height: 1080,
        picture_edit_rate_num: 24,
        picture_edit_rate_den: 1,
        picture_duration: 192,
        subtitle_id: Some("33333333-3333-3333-3333-333333333333".into()),
        subtitle_edit_rate_num: 24,
        subtitle_edit_rate_den: 1,
        subtitle_duration: track.duration,
        subtitle_language: Some("de".into()),
        ..Default::default()
    };
    let config = CplConfig {
        title: "Sub Test".into(),
        content_kind: "feature".into(),
        reels: vec![reel],
        ..Default::default()
    };
    assert_eq!(
        generate_cpl(&config, "44444444-4444-4444-4444-444444444444", &cpl_path),
        0
    );
    let cpl = std::fs::read_to_string(&cpl_path).unwrap();
    assert!(
        cpl.contains("<MainSubtitle>"),
        "CPL registers the subtitle track"
    );
    assert!(
        cpl.contains("<Id>urn:uuid:33333333-3333-3333-3333-333333333333</Id>"),
        "MainSubtitle references the wrapped asset id"
    );
    assert!(cpl.contains("<Language>de</Language>"), "subtitle language");
}

/// Reel splitting with a subtitle whose cues all sit in the first reel: the
/// second reel still has to carry a MainSubtitle, or dcpdoctor reports
/// `subtitle_missing_from_reel` on the composition.
#[test]
fn every_reel_of_a_split_subtitled_dcp_carries_a_subtitle() {
    use dcpwizard_core::dcp::{DcpConfig, create_dcp};

    const FPS: u32 = 24;
    // 1 minute = 1440 frames per reel; 1470 forces two reels (30-frame tail)
    const FRAMES: usize = 1470;

    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    std::fs::create_dir_all(&j2k).unwrap();
    let seed = j2k.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(2048, 1080, FPS, &seed).expect("black frame");
    for index in 0..FRAMES {
        std::fs::copy(&seed, j2k.join(format!("frame_{index:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();

    let wav = dir.path().join("audio.wav");
    write_silent_wav(&wav, FRAMES as u64 * 48_000 / FPS as u64);

    let srt = dir.path().join("in.srt");
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:02,000\nHello\n").unwrap();

    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "Split Subs".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(j2k),
        audio_path: Some(wav),
        subtitle_path: Some(srt),
        subtitle_opts: dcpwizard_core::subtitle::SubtitleOptions {
            // a font in the repo, so the package does not depend on the machine's
            font_path: Some(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/LiberationSans-Regular.ttf"),
            ),
            ..Default::default()
        },
        reel_length_minutes: 1,
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0);

    let cpl = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_"))
        })
        .expect("a CPL");
    let xml = std::fs::read_to_string(&cpl).unwrap();
    assert_eq!(
        xml.matches("<MainSubtitle>").count(),
        2,
        "both reels carry a subtitle: {xml}"
    );

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}

/// A stereo 24-bit 48 kHz WAV of `samples` silent frames.
fn write_silent_wav(path: &Path, samples: u64) {
    let channels = 2u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let sample_rate = 48_000u32;
    let data_len = samples * block_align as u64;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data_len as u32).to_le_bytes());
    w.resize(w.len() + data_len as usize, 0);
    std::fs::write(path, &w).unwrap();
}
