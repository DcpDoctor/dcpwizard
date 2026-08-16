//! Multi-composition package end-to-end: several CPLs with DIFFERENT content
//! (own picture/audio/subtitle each) over one shared PKL/ASSETMAP. Validates the
//! package with dcpdoctor and checks the compositions do NOT share essence
//! (contrast the versions path, which does).

use dcpwizard_core::dcp::DcpConfig;
use dcpwizard_core::multi_cpl::{CompositionSpec, create_multi_composition, load_compositions};
use std::path::{Path, PathBuf};

const W: u32 = 2048;
const H: u32 = 1080;
const FPS: u32 = 24;

fn make_content_frames(dir: &Path, count: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(W, H, FPS, &seed).expect("encode content frame");
    for i in 0..count {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

fn make_wav(path: &Path, frames: usize) {
    let sample_rate = 48_000u32;
    let channels = 2u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let n_samples = frames as u64 * (sample_rate as u64 / FPS as u64);
    let data_len = n_samples * block_align as u64;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
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

fn make_srt(path: &Path, text: &str) {
    std::fs::write(path, format!("1\n00:00:00,500 --> 00:00:02,000\n{text}\n")).unwrap();
}

fn base_config(out: &Path) -> DcpConfig {
    DcpConfig {
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Feature,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        ..Default::default()
    }
}

fn cpls(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_") && n.ends_with(".xml"))
        })
        .collect()
}

fn picture_id(cpl: &Path) -> String {
    let xml = std::fs::read_to_string(cpl).unwrap();
    let seg = xml.split("<MainPicture>").nth(1).unwrap();
    let seg = seg.split("</MainPicture>").next().unwrap();
    let s = seg.find("urn:uuid:").unwrap() + "urn:uuid:".len();
    seg[s..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
        .collect()
}

#[test]
fn distinct_compositions_share_pkl_not_essence_and_validate() {
    if no_system_font() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let j2k_a = root.path().join("a_j2k");
    let j2k_b = root.path().join("b_j2k");
    make_content_frames(&j2k_a, 48);
    make_content_frames(&j2k_b, 72);
    let wav_a = root.path().join("a.wav");
    make_wav(&wav_a, 48);
    let srt_b = root.path().join("b.srt");
    make_srt(&srt_b, "Hallo");

    let out = root.path().join("pkg");
    let comps = vec![
        CompositionSpec {
            title: "Trailer".into(),
            j2k_dir: j2k_a,
            audio: Some(wav_a),
            subtitle: None,
            subtitle_language: None,
            kind: Some("TLR".into()),
        },
        CompositionSpec {
            title: "Feature".into(),
            j2k_dir: j2k_b,
            audio: None,
            subtitle: Some(srt_b),
            subtitle_language: Some("de".into()),
            kind: None,
        },
    ];
    assert_eq!(create_multi_composition(&base_config(&out), &comps), 0);

    let found = cpls(&out);
    assert_eq!(found.len(), 2, "one CPL per composition");
    // different content: pictures must NOT be shared
    let p0 = picture_id(&found[0]);
    let p1 = picture_id(&found[1]);
    assert_ne!(p0, p1, "compositions have distinct picture essence");

    // exactly one PKL
    let pkl_count = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("PKL") && n.ends_with(".xml")
        })
        .count();
    assert_eq!(pkl_count, 1, "one shared PKL");

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}

#[test]
fn manifest_loader_rejects_missing_j2k_dir() {
    let root = tempfile::tempdir().unwrap();
    let manifest = root.path().join("m.json");
    std::fs::write(&manifest, r#"[{"title":"X","j2k_dir":"/no/such/dir"}]"#).unwrap();
    assert!(load_compositions(&manifest).is_err());
}

/// A packaged text track has to embed a font, and the paths these tests exercise
/// take no `--subtitle-font`, so they need a face on the machine running them.
fn no_system_font() -> bool {
    if postkit::subtitle_raster::find_system_sans_font().is_none() {
        eprintln!("skipping: this machine carries no system sans font");
        return true;
    }
    false
}
