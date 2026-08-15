//! Assemble one new OV composition from several existing DCPs (dom#1675).
//!
//! Builds a single new CPL whose reels are the source DCPs' reels in input order
//! (a program, e.g. short + feature). Essence is copied byte-identical and
//! referenced by its existing asset UUIDs; only a fresh CPL/PKL/ASSETMAP/VOLINDEX
//! is written. Inputs must agree on standard, frame rate and resolution, and none
//! may be encrypted (a new CPL id breaks the KDM's CPL binding).

use crate::ingest_package::{detect_standard, file_name, is_cpl_path, tag};
use crate::multi_cpl::{TimelineEntry, get_timeline};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// How to assemble the output OV.
#[derive(Debug, Clone)]
pub struct AssembleConfig {
    pub inputs: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub title: String,
    pub signer: Option<crate::package_signature::PackageSigner>,
}

/// One parsed input DCP: its reels plus picture dimensions and edit rate.
struct InputDcp {
    dir: PathBuf,
    standard: crate::Standard,
    reels: Vec<TimelineEntry>,
    pic_w: u32,
    pic_h: u32,
    edit_rate: String,
}

/// Assemble the inputs into a single-CPL OV at `config.output_dir`. Returns 0 ok.
pub fn assemble(config: &AssembleConfig) -> i32 {
    if config.inputs.len() < 2 {
        tracing::error!("assemble needs at least two input DCPs");
        return -1;
    }
    // prove the signer works before anything is written, so a bad one cannot
    // leave a half-signed package behind
    if let Some(signer) = config.signer.as_ref()
        && let Err(e) = signer.check_usable()
    {
        tracing::error!("unusable signer: {e}");
        return -1;
    }

    let mut inputs = Vec::with_capacity(config.inputs.len());
    for dir in &config.inputs {
        match parse_input(dir) {
            Ok(i) => inputs.push(i),
            Err(e) => {
                tracing::error!("{}: {e}", dir.display());
                return -1;
            }
        }
    }

    // All inputs must agree: one CPL over one shared standard/rate/resolution.
    let standard = inputs[0].standard;
    let (w0, h0) = (inputs[0].pic_w, inputs[0].pic_h);
    let rate0 = inputs[0].edit_rate.clone();
    for i in &inputs[1..] {
        if i.standard != standard {
            tracing::error!(
                "standard mismatch: {} is {:?} but the first input is {:?}",
                i.dir.display(),
                i.standard,
                standard
            );
            return -1;
        }
        if i.edit_rate != rate0 {
            tracing::error!(
                "frame-rate mismatch: {} is '{}' but the first input is '{}'",
                i.dir.display(),
                i.edit_rate,
                rate0
            );
            return -1;
        }
        if (i.pic_w, i.pic_h) != (w0, h0) {
            tracing::error!(
                "resolution mismatch: {} is {}x{} but the first input is {}x{}",
                i.dir.display(),
                i.pic_w,
                i.pic_h,
                w0,
                h0
            );
            return -1;
        }
    }

    if let Err(e) = std::fs::create_dir_all(&config.output_dir) {
        tracing::error!("cannot create output {}: {e}", config.output_dir.display());
        return -1;
    }

    let (edit_num, edit_den) = parse_edit_rate(&rate0);

    let mut cpl_reels: Vec<crate::cpl::CplReel> = Vec::new();
    let mut new_assets: Vec<(String, String, String, u64)> = Vec::new(); // id, filename, hash, size
    let mut copied: HashSet<String> = HashSet::new();
    let mut used_names: HashSet<String> = HashSet::new();

    for input in &inputs {
        let pkl_map = parse_pkls(&input.dir);
        for reel in &input.reels {
            // copy every track file the reel references, deduped by asset id
            let mut copy_track = |id: &str, src: &str| -> Result<(), i32> {
                if id.is_empty() {
                    return Ok(());
                }
                if !copied.insert(id.to_string()) {
                    return Ok(());
                }
                let src_path = PathBuf::from(src);
                if !src_path.is_file() {
                    tracing::error!("asset {id} file missing: {src}");
                    return Err(-1);
                }
                let name = unique_name(&file_name(&src_path), id, &mut used_names);
                let dst = config.output_dir.join(&name);
                if let Err(e) = std::fs::copy(&src_path, &dst) {
                    tracing::error!(
                        "cannot copy {} -> {}: {e}",
                        src_path.display(),
                        dst.display()
                    );
                    return Err(-1);
                }
                let (hash, size) = match pkl_map.get(id) {
                    Some((h, s)) => (h.clone(), *s),
                    None => (
                        crate::hash::hash_file(&dst).unwrap_or_default(),
                        std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0),
                    ),
                };
                new_assets.push((id.to_string(), name, hash, size));
                Ok(())
            };

            if let Err(c) = copy_track(&reel.picture_asset_id, &reel.picture_file) {
                return c;
            }
            if let Err(c) = copy_track(&reel.sound_asset_id, &reel.sound_file) {
                return c;
            }
            if let Err(c) = copy_track(&reel.subtitle_asset_id, &reel.subtitle_file) {
                return c;
            }

            let has_sub = !reel.subtitle_asset_id.is_empty();
            cpl_reels.push(crate::cpl::CplReel {
                reel_id: uuid::Uuid::new_v4().to_string(),
                picture_id: reel.picture_asset_id.clone(),
                picture_width: input.pic_w,
                picture_height: input.pic_h,
                picture_edit_rate_num: edit_num,
                picture_edit_rate_den: edit_den,
                picture_duration: reel.duration_frames,
                picture_entry_point: reel.entry_point,
                picture_key_id: None,
                sound_id: (!reel.sound_asset_id.is_empty()).then(|| reel.sound_asset_id.clone()),
                sound_edit_rate_num: edit_num,
                sound_edit_rate_den: edit_den,
                sound_duration: reel.duration_frames,
                sound_entry_point: 0,
                sound_key_id: None,
                subtitle_id: has_sub.then(|| reel.subtitle_asset_id.clone()),
                subtitle_edit_rate_num: if has_sub { edit_num } else { 0 },
                subtitle_edit_rate_den: if has_sub { edit_den } else { 0 },
                subtitle_duration: if has_sub { reel.duration_frames } else { 0 },
                subtitle_entry_point: 0,
                subtitle_language: has_sub
                    .then(|| reel.subtitle_language.clone())
                    .filter(|l| !l.is_empty()),
                stereoscopic: false,
                aux_data: None,
                ..Default::default()
            });
        }
    }

    if cpl_reels.is_empty() {
        tracing::error!("no reels found in the input DCPs");
        return -1;
    }
    // the assembled program is a new composition, so it gets its own FFOC/LFOC
    crate::cpl::apply_default_markers(&mut cpl_reels);

    let title = if config.title.is_empty() {
        "Assembled OV".to_string()
    } else {
        config.title.clone()
    };

    // ── CPL ──
    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let cpl_path = config.output_dir.join(format!("CPL_{cpl_uuid}.xml"));
    let cpl_config = crate::cpl::CplConfig {
        title,
        content_kind: "feature".into(),
        rating: String::new(),
        reels: cpl_reels,
        standard,
        main_sound: None,
        sign_language: None,
    };
    if crate::cpl::generate_cpl(&cpl_config, &cpl_uuid, &cpl_path) != 0 {
        tracing::error!("failed to write assembled CPL");
        return -1;
    }
    // before the PKL entry below hashes it
    if !crate::package_signature::sign_if_configured(config.signer.as_ref(), &cpl_path, "CPL") {
        return -1;
    }

    // ── PKL: CPL plus every copied asset ──
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let mut pkl_entries = vec![crate::pkl::PklEntry {
        id: cpl_uuid.clone(),
        asset_type: "text/xml".into(),
        file: cpl_path.clone(),
        hash: crate::hash::hash_file(&cpl_path).unwrap_or_default(),
        size: std::fs::metadata(&cpl_path).map(|m| m.len()).unwrap_or(0),
    }];
    for (id, name, hash, size) in &new_assets {
        pkl_entries.push(crate::pkl::PklEntry {
            id: id.clone(),
            asset_type: "application/mxf".into(),
            file: config.output_dir.join(name),
            hash: hash.clone(),
            size: *size,
        });
    }
    let pkl_path = config.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    if crate::pkl::generate_pkl(&pkl_entries, &pkl_uuid, standard, None, &pkl_path) != 0 {
        tracing::error!("failed to write assembled PKL");
        return -1;
    }
    if !crate::package_signature::sign_if_configured(config.signer.as_ref(), &pkl_path, "PKL") {
        return -1;
    }

    // ── ASSETMAP + VOLINDEX ──
    let mut am_entries = vec![
        crate::assetmap::AssetMapEntry {
            id: pkl_uuid,
            path: file_name(&pkl_path),
            packing_list: true,
        },
        crate::assetmap::AssetMapEntry {
            id: cpl_uuid,
            path: file_name(&cpl_path),
            packing_list: false,
        },
    ];
    for (id, name, _, _) in &new_assets {
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: id.clone(),
            path: name.clone(),
            packing_list: false,
        });
    }
    if crate::assetmap::generate_assetmap(&am_entries, &config.output_dir, standard, None) != 0 {
        tracing::error!("failed to write assembled ASSETMAP");
        return -1;
    }

    tracing::info!(
        "assembled {} input(s) into {} ({} reels, {} assets)",
        inputs.len(),
        config.output_dir.display(),
        cpl_config.reels.len(),
        new_assets.len()
    );
    0
}

/// Parse one input DCP: exactly one CPL, its reels, picture size and edit rate.
/// Fails loud on an encrypted CPL (a new CPL id would break its KDM binding).
fn parse_input(dir: &Path) -> Result<InputDcp, String> {
    if !dir.is_dir() {
        return Err("not a directory".into());
    }
    let cpls: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_cpl_path(p))
        .collect();
    if cpls.is_empty() {
        return Err("no CPL found".into());
    }
    if cpls.len() > 1 {
        return Err("has multiple CPLs; assemble takes single-composition DCPs".into());
    }
    let cpl_path = &cpls[0];
    let cpl = std::fs::read_to_string(cpl_path).map_err(|e| e.to_string())?;
    if cpl.contains("<KeyId>") {
        return Err(
            "input is encrypted; a new CPL id breaks the KDM's CPL binding, so assemble refuses \
             encrypted DCPs. Decrypt first (decrypt command) or assemble the plaintext masters"
                .into(),
        );
    }
    let standard = detect_standard(&cpls);
    let reels = get_timeline(cpl_path);
    if reels.is_empty() {
        return Err("CPL has no reels".into());
    }
    let (pic_w, pic_h) = parse_screen_aspect(&cpl);
    let edit_rate = reels[0].edit_rate.clone();
    Ok(InputDcp {
        dir: dir.to_path_buf(),
        standard,
        reels,
        pic_w,
        pic_h,
        edit_rate,
    })
}

/// id -> (hash, size) across every PKL in the directory.
fn parse_pkls(dir: &Path) -> HashMap<String, (String, u64)> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let name = file_name(&entry.path());
        if !(name.starts_with("PKL") && name.ends_with(".xml")) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for block in content.split("<Asset>").skip(1) {
            let block = block.split("</Asset>").next().unwrap_or("");
            let id = tag(block, "Id").map(|v| v.replace("urn:uuid:", ""));
            let hash = tag(block, "Hash");
            let (Some(id), Some(hash)) = (id, hash) else {
                continue;
            };
            let size = tag(block, "Size").and_then(|s| s.parse().ok()).unwrap_or(0);
            map.insert(id, (hash, size));
        }
    }
    map
}

/// Pick a unique output filename, appending a short id suffix on collision.
fn unique_name(filename: &str, id: &str, used: &mut HashSet<String>) -> String {
    if used.insert(filename.to_string()) {
        return filename.to_string();
    }
    let (stem, ext) = match filename.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (filename, String::new()),
    };
    let short = &id[..id.len().min(8)];
    let mut candidate = format!("{stem}_{short}{ext}");
    let mut n = 1;
    while !used.insert(candidate.clone()) {
        candidate = format!("{stem}_{short}_{n}{ext}");
        n += 1;
    }
    candidate
}

fn parse_edit_rate(s: &str) -> (u32, u32) {
    let mut it = s.split_whitespace();
    let num = it.next().and_then(|v| v.parse().ok()).unwrap_or(24);
    let den = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    (num, den)
}

/// Picture dimensions from a CPL's ScreenAspectRatio. SMPTE carries a "w h" pair;
/// Interop carries a decimal, from which we recover w at 1080p.
fn parse_screen_aspect(cpl: &str) -> (u32, u32) {
    let inner = cpl
        .split_once("<ScreenAspectRatio>")
        .and_then(|(_, r)| r.split_once("</ScreenAspectRatio>"))
        .map(|(v, _)| v.trim());
    match inner {
        Some(v) if v.contains(char::is_whitespace) => {
            let mut it = v.split_whitespace();
            let w = it.next().and_then(|x| x.parse().ok()).unwrap_or(2048);
            let h = it.next().and_then(|x| x.parse().ok()).unwrap_or(1080);
            (w, h)
        }
        Some(v) => match v.parse::<f64>() {
            Ok(r) if r > 0.0 => ((r * 1080.0).round() as u32, 1080),
            _ => (2048, 1080),
        },
        None => (2048, 1080),
    }
}
