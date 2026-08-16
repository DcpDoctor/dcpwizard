//! Multi-version (multi-CPL) package assembly: build small real packages via the
//! versions path and check the shared essence, per-version subtitle/audio, single
//! PKL/ASSETMAP, per-CPL key files, and dcpdoctor validation.

use dcpwizard_core::dcp::DcpConfig;
use dcpwizard_core::versions::{VersionSpec, create_versioned_dcp};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const W: u32 = 2048;
const H: u32 = 1080;

fn make_content_frames(dir: &Path, count: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(W, H, FPS, &seed).expect("encode content frame");
    for i in 0..count {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

/// A stereo 48 kHz 16-bit WAV, `frames` frames long (2000 samples per frame).
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
    let srt = format!("1\n00:00:00,500 --> 00:00:02,000\n{text}\n");
    std::fs::write(path, srt).unwrap();
}

fn version(title: &str, subtitle: Option<PathBuf>, audio: Option<PathBuf>) -> VersionSpec {
    VersionSpec {
        title: title.into(),
        subtitle,
        subtitle_language: None,
        ccap: None,
        audio,
        kind: None,
    }
}

fn base_config(out: &Path, j2k: PathBuf, audio: Option<PathBuf>) -> DcpConfig {
    DcpConfig {
        title: "Master".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(j2k),
        audio_path: audio,
        ..Default::default()
    }
}

fn list_cpls(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_") && n.ends_with(".xml"))
        })
        .collect()
}

fn tag(text: &str, t: &str) -> Option<String> {
    let open = format!("<{t}");
    let start = text.find(&open)?;
    let after = &text[start + open.len()..];
    let gt = after.find('>')?;
    let content = &after[gt + 1..];
    let end = content.find(&format!("</{t}>"))?;
    let v = content[..end].trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// The asset ids and durations of one reel, in reel order.
struct ReelIds {
    picture: String,
    picture_duration: u64,
    sound: Option<String>,
    subtitle: Option<String>,
    subtitle_duration: Option<u64>,
}

fn cpl_reels(cpl: &Path) -> Vec<ReelIds> {
    let xml = std::fs::read_to_string(cpl).unwrap();
    let mut out = Vec::new();
    for seg in xml.split("<Reel>").skip(1) {
        let seg = seg.split("</Reel>").next().unwrap_or("");
        let block_in = |tagname: &str| -> Option<String> {
            let open = format!("<{tagname}>");
            let start = seg.find(&open)?;
            let block = &seg[start..];
            let block = block.split(&format!("</{tagname}>")).next().unwrap_or("");
            Some(block.to_string())
        };
        let id_in =
            |tagname: &str| tag(&block_in(tagname)?, "Id").map(|v| v.replace("urn:uuid:", ""));
        let duration_in =
            |tagname: &str| tag(&block_in(tagname)?, "IntrinsicDuration")?.parse().ok();
        let picture = id_in("MainPicture").expect("reel has a picture");
        out.push(ReelIds {
            picture,
            picture_duration: duration_in("MainPicture").expect("reel has a picture duration"),
            sound: id_in("MainSound"),
            subtitle: id_in("MainSubtitle"),
            subtitle_duration: duration_in("MainSubtitle"),
        });
    }
    out
}

/// asset ids listed in the single PKL.
fn pkl_ids(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("PKL") && name.ends_with(".xml")) {
            continue;
        }
        let content = std::fs::read_to_string(entry.path()).unwrap();
        for block in content.split("<Asset>").skip(1) {
            let block = block.split("</Asset>").next().unwrap_or("");
            if let Some(id) = tag(block, "Id") {
                out.push(id.replace("urn:uuid:", ""));
            }
        }
    }
    out
}

fn assetmap_ids(dir: &Path) -> HashSet<String> {
    let xml = std::fs::read_to_string(dir.join("ASSETMAP.xml")).unwrap();
    let mut ids = HashSet::new();
    for block in xml.split("<Asset>").skip(1) {
        let block = block.split("</Asset>").next().unwrap_or("");
        if let Some(id) = tag(block, "Id") {
            ids.insert(id.replace("urn:uuid:", ""));
        }
    }
    ids
}

fn pkl_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("PKL") && n.ends_with(".xml")
        })
        .count()
}

#[test]
fn two_versions_share_essence_and_validate() {
    let root = tempfile::tempdir().unwrap();
    let j2k = root.path().join("j2k");
    make_content_frames(&j2k, 48);
    let wav = root.path().join("base.wav");
    make_wav(&wav, 48);
    let sub_en = root.path().join("en.srt");
    let sub_fr = root.path().join("fr.srt");
    make_srt(&sub_en, "Hello");
    make_srt(&sub_fr, "Bonjour");

    let out = root.path().join("pkg");
    let config = base_config(&out, j2k, Some(wav));
    let versions = vec![
        version("English", Some(sub_en), None),
        version("French", Some(sub_fr), None),
    ];
    assert_eq!(create_versioned_dcp(&config, &versions), 0);

    let cpls = list_cpls(&out);
    assert_eq!(cpls.len(), 2, "one CPL per version");

    let a = cpl_reels(&cpls[0]);
    let b = cpl_reels(&cpls[1]);
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    // picture + sound shared across the two CPLs
    assert_eq!(a[0].picture, b[0].picture, "shared picture UUID");
    assert_eq!(a[0].sound, b[0].sound, "shared base sound UUID");
    // subtitle tracks distinct per version
    let sa = a[0].subtitle.as_ref().expect("version A subtitle");
    let sb = b[0].subtitle.as_ref().expect("version B subtitle");
    assert_ne!(sa, sb, "per-version subtitle MXFs are distinct");

    // PKL lists each asset exactly once, including both CPLs
    let ids = pkl_ids(&out);
    let set: HashSet<_> = ids.iter().cloned().collect();
    assert_eq!(set.len(), ids.len(), "no PKL asset appears twice: {ids:?}");
    let cpl_ids: Vec<String> = cpls
        .iter()
        .map(|c| {
            tag(&std::fs::read_to_string(c).unwrap(), "Id")
                .unwrap()
                .replace("urn:uuid:", "")
        })
        .collect();
    for cid in &cpl_ids {
        assert!(set.contains(cid), "PKL lists CPL {cid}");
    }

    // ASSETMAP covers every PKL asset
    let am = assetmap_ids(&out);
    for id in &ids {
        assert!(am.contains(id), "ASSETMAP covers {id}");
    }

    assert_eq!(pkl_count(&out), 1, "one merged PKL");

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}

#[test]
fn multi_reel_versions_share_picture_per_reel() {
    let root = tempfile::tempdir().unwrap();
    let j2k = root.path().join("j2k");
    // 1 minute = 1440 frames per reel; 1470 forces two reels (30-frame tail)
    make_content_frames(&j2k, 1470);
    let wav = root.path().join("base.wav");
    make_wav(&wav, 1470);
    let sub_en = root.path().join("en.srt");
    let sub_fr = root.path().join("fr.srt");
    make_srt(&sub_en, "Hello");
    make_srt(&sub_fr, "Bonjour");

    let out = root.path().join("pkg");
    let mut config = base_config(&out, j2k, Some(wav));
    config.reel_length_minutes = 1;
    let versions = vec![
        version("English", Some(sub_en), None),
        version("French", Some(sub_fr), None),
    ];
    assert_eq!(create_versioned_dcp(&config, &versions), 0);

    let cpls = list_cpls(&out);
    assert_eq!(cpls.len(), 2);
    let a = cpl_reels(&cpls[0]);
    let b = cpl_reels(&cpls[1]);
    assert_eq!(a.len(), 2, "two reels");
    assert_eq!(b.len(), 2, "two reels");
    for r in 0..2 {
        assert_eq!(a[r].picture, b[r].picture, "reel {r} picture shared");
        assert_eq!(a[r].sound, b[r].sound, "reel {r} sound shared");
    }

    // both cues sit in reel 1, so reel 2 carries an empty document; every reel of
    // a subtitled composition still has to carry a subtitle spanning it
    for reels in [&a, &b] {
        for (index, reel) in reels.iter().enumerate() {
            let subtitle = reel
                .subtitle
                .as_ref()
                .unwrap_or_else(|| panic!("reel {} has a MainSubtitle", index + 1));
            assert_eq!(
                reel.subtitle_duration,
                Some(reel.picture_duration),
                "reel {} subtitle spans the reel",
                index + 1
            );
            let asset = dcpdoctor_core::subtitle::read_wrapped_timed_text(
                &out.join(format!("subtitle_{subtitle}.mxf")),
                &dcpdoctor_core::kdm::ContentKeys::none(),
                dcpdoctor_core::subtitle::FontData::Omit,
            )
            .unwrap_or_else(|| panic!("read reel {} subtitle essence", index + 1));
            assert_eq!(
                u64::from(asset.container_duration),
                reel.picture_duration,
                "reel {} essence spans the reel",
                index + 1
            );
            if index == 0 {
                assert!(
                    asset.xml.contains("<dcst:Subtitle "),
                    "reel 1 carries the cue: {}",
                    asset.xml
                );
            } else {
                assert!(
                    asset.xml.contains("<dcst:SubtitleList/>"),
                    "reel 2 document is empty: {}",
                    asset.xml
                );
            }
        }
    }

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}

#[test]
fn encrypted_two_versions_write_per_cpl_keys() {
    let root = tempfile::tempdir().unwrap();
    let j2k = root.path().join("j2k");
    make_content_frames(&j2k, 48);
    let wav = root.path().join("base.wav");
    make_wav(&wav, 48);
    let fr_wav = root.path().join("fr.wav");
    make_wav(&fr_wav, 48);

    let out = root.path().join("pkg");
    let keys = root.path().join("keys.json");
    let mut config = base_config(&out, j2k, Some(wav));
    config.encrypt = true;
    config.key_out = Some(keys);
    // the versions differ by sound, not subtitle: an encrypted package cannot
    // carry timed text, which the wrap layer has no way to encrypt
    let versions = vec![
        version("English", None, None),
        version("French", None, Some(fr_wav)),
    ];
    assert_eq!(create_versioned_dcp(&config, &versions), 0);

    let k1 = root.path().join("keys_1_English.json");
    let k2 = root.path().join("keys_2_French.json");
    assert!(k1.exists(), "per-CPL key file 1");
    assert!(k2.exists(), "per-CPL key file 2");

    let b1 = dcpwizard_core::encrypt::KeyBundle::read(&k1).unwrap();
    let b2 = dcpwizard_core::encrypt::KeyBundle::read(&k2).unwrap();

    let pic_key = |b: &dcpwizard_core::encrypt::KeyBundle| -> String {
        b.keys
            .iter()
            .find(|k| k.key_type == dcpwizard_core::encrypt::KeyType::Mdik)
            .expect("picture key")
            .key_id
            .clone()
    };
    assert_eq!(pic_key(&b1), pic_key(&b2), "shared picture key_id");

    // each bundle's cpl_id names a real CPL in the package
    let cpl_ids: HashSet<String> = list_cpls(&out)
        .iter()
        .map(|c| {
            tag(&std::fs::read_to_string(c).unwrap(), "Id")
                .unwrap()
                .replace("urn:uuid:", "")
        })
        .collect();
    assert!(cpl_ids.contains(&b1.cpl_id), "b1 cpl_id valid");
    assert!(cpl_ids.contains(&b2.cpl_id), "b2 cpl_id valid");
    assert_ne!(b1.cpl_id, b2.cpl_id, "distinct CPLs");
}

#[test]
fn per_version_audio_overrides_base_sound() {
    let root = tempfile::tempdir().unwrap();
    let j2k = root.path().join("j2k");
    make_content_frames(&j2k, 48);
    let base_wav = root.path().join("base.wav");
    make_wav(&base_wav, 48);
    let own_wav = root.path().join("own.wav");
    make_wav(&own_wav, 48);

    let out = root.path().join("pkg");
    let config = base_config(&out, j2k, Some(base_wav));
    let versions = vec![
        version("WithOwnAudio", None, Some(own_wav)),
        version("BaseAudio", None, None),
    ];
    assert_eq!(create_versioned_dcp(&config, &versions), 0);

    let cpls = list_cpls(&out);
    // map title -> reels
    let mut own = None;
    let mut base = None;
    for c in &cpls {
        let xml = std::fs::read_to_string(c).unwrap();
        let reels = cpl_reels(c);
        if xml.contains("<ContentTitleText>WithOwnAudio</ContentTitleText>") {
            own = Some(reels);
        } else if xml.contains("<ContentTitleText>BaseAudio</ContentTitleText>") {
            base = Some(reels);
        }
    }
    let own = own.unwrap();
    let base = base.unwrap();
    // same shared picture, different sound
    assert_eq!(own[0].picture, base[0].picture, "picture shared");
    assert_ne!(
        own[0].sound, base[0].sound,
        "own-audio version has a distinct sound MXF"
    );
    // both sound assets exist on disk
    for reels in [&own, &base] {
        let sid = reels[0].sound.as_ref().unwrap();
        assert!(
            out.join(format!("sound_{sid}.mxf")).exists(),
            "sound {sid} wrapped"
        );
    }

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}
