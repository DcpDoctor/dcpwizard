//! Metadata-only DCP repackaging: rebuild ASSETMAP and PKL to cover every asset
//! file physically present in a directory, without touching the essence.
//!
//! Motivating case: a Sony server exports a VF folder that carries the VF package
//! plus hardlinked OV MXFs which the VF CPL references but the exported
//! ASSETMAP/PKL never list, so the package can't be re-ingested elsewhere. This
//! scans the directory, reads each MXF's embedded asset UUID, and regenerates the
//! ASSETMAP and PKL (and VOLINDEX) covering all present assets, reusing hashes
//! from the old PKL where available so nothing gets re-hashed needlessly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One asset file present in the package.
struct Asset {
    id: String,
    filename: String,
    asset_type: String,
    path: PathBuf,
}

/// Regenerate ASSETMAP + PKL (+ VOLINDEX) for the package in `dir`, covering
/// every asset file present. Replaces the existing ASSETMAP/PKL/VOLINDEX.
/// `signer` signs the new PKL. The CPLs already in `dir` are hashed, never
/// rewritten, so a signature one of them carries stays valid.
pub fn ingest_package(dir: &Path, signer: Option<&crate::package_signature::PackageSigner>) -> i32 {
    if !dir.is_dir() {
        tracing::error!("not a directory: {}", dir.display());
        return -1;
    }
    // prove the signer works before anything is written, so a bad one cannot
    // leave a half-signed package behind
    if let Some(signer) = signer
        && let Err(e) = signer.check_usable()
    {
        tracing::error!("unusable signer: {e}");
        return -1;
    }

    // Collect the CPLs to detect the standard and cross-check referenced ids.
    let cpl_files = find_cpls(dir);
    if cpl_files.is_empty() {
        tracing::error!("no CPL found in {}; nothing to repackage", dir.display());
        return -1;
    }
    let standard = detect_standard(&cpl_files);
    let referenced = collect_referenced_ids(&cpl_files);

    // Reuse hashes/sizes/types from any existing PKL so present assets that are
    // already declared don't get re-hashed.
    let old_pkl = parse_existing_pkls(dir);
    // Old ASSETMAP maps loose (non-MXF, non-CPL) assets like Interop subtitles.
    let old_assetmap = parse_existing_assetmap(dir);

    let mut assets: Vec<Asset> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // CPLs first (matches the create/create-vf ordering), then essence.
    for path in &cpl_files {
        let Some(id) = read_cpl_id(path) else {
            tracing::error!("CPL {} has no <Id>", path.display());
            return -1;
        };
        push_asset(&mut assets, &mut seen_ids, id, path, "text/xml");
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("cannot read {}: {e}", dir.display());
            return -1;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = file_name(&path);
        if is_packaging_file(&name) || is_cpl_path(&path) {
            continue;
        }
        if has_ext(&path, "mxf") {
            match read_mxf_asset_uuid(&path) {
                Ok(id) => push_asset(&mut assets, &mut seen_ids, id, &path, "application/mxf"),
                Err(e) => {
                    tracing::error!("cannot read asset UUID from {}: {e}", path.display());
                    return -1;
                }
            }
        } else if let Some(id) = old_assetmap.get(&name) {
            // loose asset (e.g. Interop subtitle or font) carried by the old map
            let ty = old_pkl
                .get(id)
                .map(|a| a.asset_type.clone())
                .unwrap_or_else(|| guess_type(&path));
            push_asset(&mut assets, &mut seen_ids, id.clone(), &path, &ty);
        }
    }

    // Warn about any CPL-referenced id with no present asset file.
    for id in &referenced {
        if !seen_ids.contains(id) {
            tracing::warn!("CPL references {id} but no matching asset file is present");
        }
    }

    // Build PKL entries, reusing hash/size from the old PKL when the id matches.
    let mut pkl_entries = Vec::with_capacity(assets.len());
    for a in &assets {
        let (hash, size) = match old_pkl.get(&a.id) {
            Some(o) => (o.hash.clone(), o.size),
            None => {
                let hash = match crate::hash::hash_file(&a.path) {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::error!("{e}");
                        return -1;
                    }
                };
                let size = std::fs::metadata(&a.path).map(|m| m.len()).unwrap_or(0);
                (hash, size)
            }
        };
        pkl_entries.push(crate::pkl::PklEntry {
            id: a.id.clone(),
            asset_type: a.asset_type.clone(),
            file: a.path.clone(),
            hash,
            size,
        });
    }

    // Replace the old packaging files (note it in the output).
    let removed = remove_existing_packaging(dir);
    if !removed.is_empty() {
        tracing::info!("replacing existing packaging files: {}", removed.join(", "));
    }

    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_path = dir.join(format!("PKL_{pkl_uuid}.xml"));
    if crate::pkl::generate_pkl(&pkl_entries, &pkl_uuid, standard, None, &pkl_path) != 0 {
        tracing::error!("failed to write PKL");
        return -1;
    }
    if !crate::package_signature::sign_if_configured(signer, &pkl_path, "PKL") {
        return -1;
    }

    let mut am_entries = vec![crate::assetmap::AssetMapEntry {
        id: pkl_uuid,
        path: file_name(&pkl_path),
        packing_list: true,
    }];
    for a in &assets {
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: a.id.clone(),
            path: a.filename.clone(),
            packing_list: false,
        });
    }
    if crate::assetmap::generate_assetmap(&am_entries, dir, standard, None) != 0 {
        tracing::error!("failed to write ASSETMAP");
        return -1;
    }

    tracing::info!(
        "repackaged {} ({} asset(s), {} CPL(s))",
        dir.display(),
        assets.len(),
        cpl_files.len()
    );
    0
}

fn push_asset(
    assets: &mut Vec<Asset>,
    seen: &mut std::collections::HashSet<String>,
    id: String,
    path: &Path,
    asset_type: &str,
) {
    if !seen.insert(id.clone()) {
        return;
    }
    assets.push(Asset {
        id,
        filename: file_name(path),
        asset_type: asset_type.to_string(),
        path: path.to_path_buf(),
    });
}

/// Read a DCP MXF's embedded asset UUID via asdcplib, dispatching on essence type.
fn read_mxf_asset_uuid(path: &Path) -> Result<String, String> {
    use asdcplib::EssenceType;
    let name = path
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 path {}", path.display()))?;
    let et = asdcplib::essence_type(name).map_err(|e| e.to_string())?;
    let info = match et {
        EssenceType::Jpeg2000 => {
            let mut r = asdcplib::jp2k::MxfReader::new();
            r.open_read(name).map_err(|e| e.to_string())?;
            r.writer_info().map_err(|e| e.to_string())?
        }
        EssenceType::Jpeg2000Stereo => {
            let mut r = asdcplib::jp2k::StereoMxfReader::new();
            r.open_read(name).map_err(|e| e.to_string())?;
            r.writer_info().map_err(|e| e.to_string())?
        }
        EssenceType::Pcm24b48k | EssenceType::Pcm24b96k => {
            let mut r = asdcplib::pcm::MxfReader::new();
            r.open_read(name).map_err(|e| e.to_string())?;
            r.writer_info().map_err(|e| e.to_string())?
        }
        EssenceType::TimedText => {
            let mut r = asdcplib::timed_text::MxfReader::new();
            r.open_read(name).map_err(|e| e.to_string())?;
            r.writer_info().map_err(|e| e.to_string())?
        }
        EssenceType::DcDataDolbyAtmos | EssenceType::DcDataUnknown => {
            let mut r = asdcplib::atmos::MxfReader::new();
            r.open_read(name).map_err(|e| e.to_string())?;
            r.writer_info().map_err(|e| e.to_string())?
        }
        other => return Err(format!("unsupported MXF essence type {other:?}")),
    };
    Ok(uuid::Uuid::from_bytes(info.asset_uuid).to_string())
}

/// Asset metadata carried over from an existing PKL.
struct OldAsset {
    hash: String,
    size: u64,
    asset_type: String,
}

fn find_cpls(dir: &Path) -> Vec<PathBuf> {
    let mut cpls = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if is_cpl_path(&path) {
                cpls.push(path);
            }
        }
    }
    cpls.sort();
    cpls
}

pub(crate) fn is_cpl_path(path: &Path) -> bool {
    if !path.is_file() || !has_ext(path, "xml") {
        return false;
    }
    std::fs::read_to_string(path)
        .map(|c| c.contains("CompositionPlaylist"))
        .unwrap_or(false)
}

pub(crate) fn detect_standard(cpls: &[PathBuf]) -> crate::Standard {
    for path in cpls {
        if let Ok(c) = std::fs::read_to_string(path)
            && c.contains("digicine.com")
        {
            return crate::Standard::Interop;
        }
    }
    crate::Standard::Smpte
}

/// All `urn:uuid:` asset ids referenced anywhere in the CPLs (for cross-check).
fn collect_referenced_ids(cpls: &[PathBuf]) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    for path in cpls {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        for id in uuids_in(&content) {
            ids.insert(id);
        }
    }
    ids
}

fn read_cpl_id(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    // the CPL's own <Id> is the first urn:uuid in the document
    uuids_in(&content).into_iter().next()
}

/// Every bare UUID following a `urn:uuid:` prefix in `text`, in order.
pub(crate) fn uuids_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (pos, _) in text.match_indices("urn:uuid:") {
        let rest = &text[pos + "urn:uuid:".len()..];
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
            .collect();
        if id.len() == 36 {
            out.push(id);
        }
    }
    out
}

fn parse_existing_pkls(dir: &Path) -> HashMap<String, OldAsset> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = file_name(&path);
        if !(name.starts_with("PKL") && name.ends_with(".xml")) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        // walk <Asset> blocks: Id, Hash, Size, Type
        for block in content.split("<Asset>").skip(1) {
            let block = block.split("</Asset>").next().unwrap_or("");
            let id = tag(block, "Id").map(|v| v.replace("urn:uuid:", ""));
            let hash = tag(block, "Hash");
            let (Some(id), Some(hash)) = (id, hash) else {
                continue;
            };
            let size = tag(block, "Size").and_then(|s| s.parse().ok()).unwrap_or(0);
            let asset_type = tag(block, "Type").unwrap_or_else(|| "application/mxf".into());
            map.insert(
                id,
                OldAsset {
                    hash,
                    size,
                    asset_type,
                },
            );
        }
    }
    map
}

/// Map filename -> asset id from an existing ASSETMAP (for loose assets).
fn parse_existing_assetmap(dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let path = ["ASSETMAP.xml", "ASSETMAP"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.exists());
    let Some(path) = path else {
        return map;
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return map;
    };
    for block in content.split("<Asset>").skip(1) {
        let block = block.split("</Asset>").next().unwrap_or("");
        let id = tag(block, "Id").map(|v| v.replace("urn:uuid:", ""));
        let file = tag(block, "Path");
        if let (Some(id), Some(file)) = (id, file) {
            map.insert(file, id);
        }
    }
    map
}

/// Delete the existing ASSETMAP/PKL/VOLINDEX files, returning their names.
fn remove_existing_packaging(dir: &Path) -> Vec<String> {
    let mut removed = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = file_name(&path);
            let is_pkl = name.starts_with("PKL") && name.ends_with(".xml");
            if (is_packaging_file(&name) || is_pkl) && std::fs::remove_file(&path).is_ok() {
                removed.push(name);
            }
        }
    }
    removed.sort();
    removed
}

fn is_packaging_file(name: &str) -> bool {
    matches!(
        name,
        "ASSETMAP" | "ASSETMAP.xml" | "VOLINDEX" | "VOLINDEX.xml"
    )
}

/// First `<tag>...</tag>` value inside `text` (single-line or spanning).
pub(crate) fn tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = text.find(&open)?;
    let after = &text[start + open.len()..];
    let gt = after.find('>')?;
    let content = &after[gt + 1..];
    let end = content.find(&format!("</{tag}>"))?;
    let value = content[..end].trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn guess_type(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("xml") => "text/xml".into(),
        Some(e) if e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf") => {
            "application/x-font-opentype".into()
        }
        Some(e) if e.eq_ignore_ascii_case("png") => "image/png".into(),
        _ => "application/octet-stream".into(),
    }
}

pub(crate) fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}
