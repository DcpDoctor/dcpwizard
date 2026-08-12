//! One id per track file: the uuid in the MXF file name, the asset Id in the
//! CPL, the Id in the PKL, the Id in the ASSETMAP and the AssetUUID the MXF
//! itself carries must all be the same value. The MXF side is read back with an
//! external asdcp-info binary, so dcpwizard cannot agree with itself and pass.
//! Set DCPWIZARD_ASDCP_INFO to an asdcp-info binary to run these.

use dcpwizard_core::dcp::DcpConfig;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 24;

/// The CPL asset element that describes the composition itself, not a track file.
const COMPOSITION_METADATA_ASSET: &str = "CompositionMetadataAsset";

fn asdcp_info() -> Option<String> {
    match std::env::var("DCPWIZARD_ASDCP_INFO") {
        Ok(tool) => Some(tool),
        Err(_) => {
            eprintln!("skipping: set DCPWIZARD_ASDCP_INFO to an asdcp-info binary");
            None
        }
    }
}

/// The AssetUUID an MXF actually carries, read by an independent binary.
fn embedded_asset_uuid(tool: &str, path: &Path) -> String {
    let out = std::process::Command::new(tool)
        .arg("-i")
        .arg(path)
        .output()
        .expect("run asdcp-info");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("AssetUUID:"))
        .map(|v| v.trim().to_lowercase())
        .unwrap_or_else(|| panic!("asdcp-info reported no AssetUUID for {}", path.display()))
}

fn make_frames(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(WIDTH, HEIGHT, FPS, &seed).expect("encode frame");
    for i in 0..FRAMES {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

fn make_wav(path: &Path) {
    let sample_rate = 48_000u32;
    let channels = 2u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let samples = FRAMES as u64 * (sample_rate as u64 / FPS as u64);
    let data_len = samples * block_align as u64;
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

fn make_srt(path: &Path) {
    std::fs::write(path, "1\n00:00:00,200 --> 00:00:00,800\nHello\n").unwrap();
}

fn base_config(out: &Path, j2k: PathBuf) -> DcpConfig {
    DcpConfig {
        title: "IdAgreement".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(j2k),
        ..Default::default()
    }
}

fn only_file_matching(dir: &Path, prefix: &str) -> PathBuf {
    let mut hits: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".xml"))
        })
        .collect();
    hits.sort();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one {prefix}*.xml in {dir:?}"
    );
    hits.pop().unwrap()
}

fn strip_urn(id: &str) -> String {
    id.trim().trim_start_matches("urn:uuid:").to_lowercase()
}

/// The ASSETMAP's id-to-file-name pairs, restricted to the track files.
fn assetmap_track_files(dcp_dir: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(dcp_dir.join("ASSETMAP.xml")).unwrap();
    let doc = roxmltree::Document::parse(&text).expect("parse ASSETMAP");
    let mut entries = BTreeMap::new();
    for asset in doc.descendants().filter(|n| n.has_tag_name("Asset")) {
        let id = asset
            .children()
            .find(|c| c.has_tag_name("Id"))
            .and_then(|c| c.text())
            .map(strip_urn)
            .expect("ASSETMAP Asset without an Id");
        let path = asset
            .descendants()
            .find(|c| c.has_tag_name("Path"))
            .and_then(|c| c.text())
            .expect("ASSETMAP Asset without a Path")
            .to_string();
        if path.ends_with(".mxf") {
            entries.insert(id, path);
        }
    }
    entries
}

/// Every asset id the CPL's reels reference for a track file.
fn cpl_track_asset_ids(cpl: &Path) -> BTreeSet<String> {
    let text = std::fs::read_to_string(cpl).unwrap();
    let doc = roxmltree::Document::parse(&text).expect("parse CPL");
    let mut ids = BTreeSet::new();
    for list in doc.descendants().filter(|n| {
        n.has_tag_name("AssetList") && n.parent().is_some_and(|p| p.has_tag_name("Reel"))
    }) {
        for asset in list.children().filter(|c| c.is_element()) {
            if asset.tag_name().name() == COMPOSITION_METADATA_ASSET {
                continue;
            }
            let id = asset
                .children()
                .find(|c| c.has_tag_name("Id"))
                .and_then(|c| c.text())
                .map(strip_urn)
                .unwrap_or_else(|| panic!("CPL {} asset without an Id", asset.tag_name().name()));
            ids.insert(id);
        }
    }
    ids
}

fn pkl_asset_ids(pkl: &Path) -> BTreeSet<String> {
    let text = std::fs::read_to_string(pkl).unwrap();
    let doc = roxmltree::Document::parse(&text).expect("parse PKL");
    doc.descendants()
        .filter(|n| n.has_tag_name("Asset"))
        .filter_map(|a| a.children().find(|c| c.has_tag_name("Id")))
        .filter_map(|c| c.text())
        .map(strip_urn)
        .collect()
}

/// Assert the five ids agree for every track file in a built DCP, and that the
/// CPL references no track id the package does not actually contain.
fn assert_ids_agree(tool: &str, dcp_dir: &Path, expected_tracks: usize) {
    let assetmap = assetmap_track_files(dcp_dir);
    assert_eq!(
        assetmap.len(),
        expected_tracks,
        "ASSETMAP should list {expected_tracks} track files, got {assetmap:?}"
    );

    let cpl_ids = cpl_track_asset_ids(&only_file_matching(dcp_dir, "CPL_"));
    let pkl_ids = pkl_asset_ids(&only_file_matching(dcp_dir, "PKL_"));

    for (assetmap_id, file_name) in &assetmap {
        let path = dcp_dir.join(file_name);
        assert!(path.exists(), "ASSETMAP names a missing file {file_name}");

        let name_id = file_name
            .rsplit_once('_')
            .and_then(|(_, tail)| tail.strip_suffix(".mxf"))
            .expect("track file name is <kind>_<uuid>.mxf")
            .to_lowercase();
        let mxf_id = embedded_asset_uuid(tool, &path);

        assert_eq!(
            name_id, *assetmap_id,
            "{file_name}: ASSETMAP id differs from the file name"
        );
        assert_eq!(
            mxf_id, *assetmap_id,
            "{file_name}: the MXF's own AssetUUID differs from the id the package claims"
        );
        assert!(
            cpl_ids.contains(assetmap_id),
            "{file_name}: no CPL asset uses id {assetmap_id}"
        );
        assert!(
            pkl_ids.contains(assetmap_id),
            "{file_name}: no PKL asset uses id {assetmap_id}"
        );
    }

    let assetmap_ids: BTreeSet<String> = assetmap.keys().cloned().collect();
    let dangling: Vec<&String> = cpl_ids.difference(&assetmap_ids).collect();
    assert!(
        dangling.is_empty(),
        "CPL references track ids that no packaged file carries: {dangling:?}"
    );
}

#[test]
fn every_track_file_id_agrees_on_the_main_create_path() {
    let Some(tool) = asdcp_info() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let j2k = root.join("frames");
    make_frames(&j2k);
    let wav = root.join("audio.wav");
    make_wav(&wav);
    let srt = root.join("subs.srt");
    make_srt(&srt);
    let ccap = root.join("ccap.srt");
    make_srt(&ccap);

    let out = root.join("dcp");
    let mut config = base_config(&out, j2k);
    config.audio_path = Some(wav);
    config.subtitle_path = Some(srt);
    config.ccap_path = Some(ccap);
    assert_eq!(dcpwizard_core::dcp::create_dcp(&config), 0, "create DCP");

    // picture, sound, subtitle, closed caption
    assert_ids_agree(&tool, &out, 4);
}

#[test]
fn every_track_file_id_agrees_on_the_stereoscopic_path() {
    let Some(tool) = asdcp_info() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let left = root.join("left");
    make_frames(&left);
    let right = root.join("right");
    make_frames(&right);
    let wav = root.join("audio.wav");
    make_wav(&wav);

    let out = root.join("dcp3d");
    let mut config = base_config(&out, left);
    config.stereo_3d = true;
    config.right_eye_dir = Some(right);
    config.audio_path = Some(wav);
    assert_eq!(dcpwizard_core::dcp::create_dcp(&config), 0, "create 3D DCP");

    assert_ids_agree(&tool, &out, 2);
}

#[test]
fn every_track_file_id_agrees_on_the_encrypted_path() {
    let Some(tool) = asdcp_info() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let j2k = root.join("frames");
    make_frames(&j2k);
    let wav = root.join("audio.wav");
    make_wav(&wav);

    let out = root.join("enc");
    let keys = root.join("KEYS.json");
    let mut config = base_config(&out, j2k);
    config.audio_path = Some(wav);
    config.encrypt = true;
    config.key_out = Some(keys.clone());
    assert_eq!(
        dcpwizard_core::dcp::create_dcp(&config),
        0,
        "create encrypted DCP"
    );

    assert_ids_agree(&tool, &out, 2);

    // each content key is bound to the asset id it encrypts, so unifying the ids
    // must leave the key file pointing at the ids the package actually ships
    let bundle = dcpwizard_core::encrypt::KeyBundle::read(&keys).unwrap();
    let key_asset_ids: BTreeSet<String> = bundle
        .keys
        .iter()
        .map(|k| strip_urn(&k.asset_uuid))
        .collect();
    let assetmap_ids: BTreeSet<String> = assetmap_track_files(&out).into_keys().collect();
    assert_eq!(
        key_asset_ids, assetmap_ids,
        "each content key must be bound to a packaged track id"
    );
}
