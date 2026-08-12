//! DCP combiner: merge several complete DCPs into one delivery volume with a
//! single merged ASSETMAP/VOLINDEX (and, by default, a single merged PKL).
//!
//! CPLs and essence are copied byte-identical: their signatures and hashes stay
//! valid because nothing inside them is rewritten. Everything is cross-referenced
//! by uuid, so relocating or renaming a file on disk only changes the ASSETMAP
//! <Path>, never a CPL/PKL. Interop subtitle assets (loose XML + fonts + PNGs)
//! move into a per-CPL subdirectory to avoid filename collisions while keeping
//! the subtitle XML's relative font/image references intact (dom#2420).
//!
//! Feature requests covered: merged ASSETMAP/PKL (dom#2019), alphabetical CPL
//! ordering (dom#2026), settable AnnotationText (dom#2027), interop subtitle
//! relocation (dom#2420).

use crate::ingest_package::{detect_standard, file_name, has_ext, is_cpl_path, tag, uuids_in};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// How to combine a set of input DCPs.
#[derive(Debug, Clone)]
pub struct CombineConfig {
    pub inputs: Vec<PathBuf>,
    pub output_dir: PathBuf,
    /// Keep each input's PKL as its own file instead of writing one merged PKL.
    pub separate_pkls: bool,
    /// Order CPL entries alphabetically by content title (default: input order).
    pub sort: bool,
    /// AnnotationText for the merged PKL/ASSETMAP. When None a value is derived
    /// from the input CPL titles.
    pub annotation: Option<String>,
    /// Signs the merged PKL. The CPLs are copied byte for byte, so any signature
    /// they already carry stays valid and is left alone.
    pub signer: Option<crate::package_signature::PackageSigner>,
}

/// One asset listed in an input's ASSETMAP (PKL excluded).
#[derive(Clone)]
struct InputAsset {
    id: String,
    filename: String,
    abs_path: PathBuf,
    hash: String,
    size: u64,
    asset_type: String,
    kind: AssetKind,
    /// CPL content title (only set for `AssetKind::Cpl`).
    title: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    Cpl,
    Mxf,
    /// Loose asset (interop subtitle XML, font, PNG): must keep its filename so
    /// the subtitle XML's relative references survive, so it goes in a subdir.
    Loose,
}

struct InputDcp {
    standard: crate::Standard,
    assets: Vec<InputAsset>,
    /// The input's PKL files (path kept for --separate-pkls).
    pkls: Vec<PathBuf>,
    /// Subdirectory name for this input's loose assets in the combined volume.
    subdir: String,
}

/// A chosen (deduped) asset already placed in the output volume.
struct Chosen {
    hash: String,
    size: u64,
    asset_type: String,
    out_path: String,
    kind: AssetKind,
    title: String,
}

/// Merge the input DCPs into `config.output_dir`. Returns 0 on success.
pub fn combine(config: &CombineConfig) -> i32 {
    if config.inputs.is_empty() {
        tracing::error!("no input DCPs given");
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
    for dir in &config.inputs {
        if !dir.is_dir() {
            tracing::error!("input is not a directory: {}", dir.display());
            return -1;
        }
    }

    // Parse each input.
    let mut inputs = Vec::with_capacity(config.inputs.len());
    for (i, dir) in config.inputs.iter().enumerate() {
        match parse_input(dir, i) {
            Ok(input) => inputs.push(input),
            Err(e) => {
                tracing::error!("{}: {e}", dir.display());
                return -1;
            }
        }
    }

    // All inputs must share a standard: one ASSETMAP/PKL carries one namespace.
    let standard = inputs[0].standard;
    if inputs.iter().any(|i| i.standard != standard) {
        tracing::error!("cannot combine SMPTE and Interop DCPs into one volume");
        return -1;
    }

    if let Err(e) = std::fs::create_dir_all(&config.output_dir) {
        tracing::error!("cannot create output {}: {e}", config.output_dir.display());
        return -1;
    }

    let mut chosen: HashMap<String, Chosen> = HashMap::new();
    // Filenames already used at the volume root (for collision renaming).
    let mut used_root: HashSet<String> = HashSet::new();
    // Order in which unique asset ids were first placed.
    let mut order: Vec<String> = Vec::new();
    // PKL entries for the merged ASSETMAP (id, filename).
    let mut pkl_entries: Vec<(String, String)> = Vec::new();

    for input in &inputs {
        for a in &input.assets {
            if let Some(existing) = chosen.get(&a.id) {
                // Same id in two inputs: dedupe on identical hash, else it is a
                // real conflict (two different files claiming one uuid).
                if existing.hash != a.hash {
                    tracing::error!(
                        "asset {} appears in more than one input with different content \
                         ({} vs {}); cannot combine",
                        a.id,
                        existing.hash,
                        a.hash
                    );
                    return -1;
                }
                continue;
            }

            let out_path = match a.kind {
                AssetKind::Loose => {
                    let sub = config.output_dir.join(&input.subdir);
                    if let Err(e) = std::fs::create_dir_all(&sub) {
                        tracing::error!("cannot create {}: {e}", sub.display());
                        return -1;
                    }
                    format!("{}/{}", input.subdir, a.filename)
                }
                _ => unique_root_name(&a.filename, &a.id, &mut used_root),
            };

            let dst = config.output_dir.join(&out_path);
            if let Err(e) = std::fs::copy(&a.abs_path, &dst) {
                tracing::error!(
                    "cannot copy {} -> {}: {e}",
                    a.abs_path.display(),
                    dst.display()
                );
                return -1;
            }

            chosen.insert(
                a.id.clone(),
                Chosen {
                    hash: a.hash.clone(),
                    size: a.size,
                    asset_type: a.asset_type.clone(),
                    out_path,
                    kind: a.kind,
                    title: a.title.clone(),
                },
            );
            order.push(a.id.clone());
        }

        if config.separate_pkls {
            for pkl in &input.pkls {
                let name = unique_root_name(
                    &file_name(pkl),
                    &uuid::Uuid::new_v4().to_string(),
                    &mut used_root,
                );
                let dst = config.output_dir.join(&name);
                if let Err(e) = std::fs::copy(pkl, &dst) {
                    tracing::error!(
                        "cannot copy PKL {} -> {}: {e}",
                        pkl.display(),
                        dst.display()
                    );
                    return -1;
                }
                let id = read_pkl_id(pkl).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                pkl_entries.push((id, name));
            }
        }
    }

    // Resolve the AnnotationText: explicit, else derived from the CPL titles.
    let cpl_ids_ordered = ordered_cpl_ids(&order, &chosen, config.sort);
    let annotation = config.annotation.clone().unwrap_or_else(|| {
        let titles: Vec<&str> = cpl_ids_ordered
            .iter()
            .map(|id| chosen[id].title.as_str())
            .filter(|t| !t.is_empty())
            .collect();
        if titles.is_empty() {
            "Combined DCP".to_string()
        } else {
            format!("Combined: {}", titles.join(", "))
        }
    });

    // Merged PKL (default) covering every deduped asset.
    if !config.separate_pkls {
        let pkl_uuid = uuid::Uuid::new_v4().to_string();
        let entries: Vec<crate::pkl::PklEntry> = order
            .iter()
            .map(|id| {
                let c = &chosen[id];
                crate::pkl::PklEntry {
                    id: id.clone(),
                    asset_type: c.asset_type.clone(),
                    file: PathBuf::new(),
                    hash: c.hash.clone(),
                    size: c.size,
                }
            })
            .collect();
        let pkl_path = config.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
        if crate::pkl::generate_pkl(&entries, &pkl_uuid, standard, Some(&annotation), &pkl_path)
            != 0
        {
            tracing::error!("failed to write merged PKL");
            return -1;
        }
        if !crate::package_signature::sign_if_configured(
            config.signer.as_ref(),
            &pkl_path,
            "merged PKL",
        ) {
            return -1;
        }
        pkl_entries.push((pkl_uuid, file_name(&pkl_path)));
    }

    // Merged ASSETMAP: PKL(s) first, then CPLs (ordered), then other assets.
    let mut am_entries: Vec<crate::assetmap::AssetMapEntry> = pkl_entries
        .iter()
        .map(|(id, path)| crate::assetmap::AssetMapEntry {
            id: id.clone(),
            path: path.clone(),
            packing_list: true,
        })
        .collect();
    for id in &cpl_ids_ordered {
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: id.clone(),
            path: chosen[id].out_path.clone(),
            packing_list: false,
        });
    }
    for id in &order {
        if chosen[id].kind == AssetKind::Cpl {
            continue;
        }
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: id.clone(),
            path: chosen[id].out_path.clone(),
            packing_list: false,
        });
    }

    if crate::assetmap::generate_assetmap(
        &am_entries,
        &config.output_dir,
        standard,
        Some(&annotation),
    ) != 0
    {
        tracing::error!("failed to write merged ASSETMAP");
        return -1;
    }

    let cpl_count = cpl_ids_ordered.len();
    tracing::info!(
        "combined {} input(s) into {} ({} CPL(s), {} asset(s))",
        inputs.len(),
        config.output_dir.display(),
        cpl_count,
        order.len()
    );
    0
}

/// CPL ids in the listing order: input order, or alphabetical by title if sort.
fn ordered_cpl_ids(order: &[String], chosen: &HashMap<String, Chosen>, sort: bool) -> Vec<String> {
    let mut ids: Vec<String> = order
        .iter()
        .filter(|id| chosen[*id].kind == AssetKind::Cpl)
        .cloned()
        .collect();
    if sort {
        ids.sort_by(|a, b| {
            chosen[a]
                .title
                .to_lowercase()
                .cmp(&chosen[b].title.to_lowercase())
        });
    }
    ids
}

/// Pick a root-level filename, appending a short id suffix on collision.
fn unique_root_name(filename: &str, id: &str, used: &mut HashSet<String>) -> String {
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

/// Parse one input DCP: its standard, its ASSETMAP assets (with hashes/types
/// from the PKL), and its PKL files.
fn parse_input(dir: &Path, index: usize) -> Result<InputDcp, String> {
    let am_path = ["ASSETMAP.xml", "ASSETMAP"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.exists())
        .ok_or("no ASSETMAP")?;
    let am = std::fs::read_to_string(&am_path).map_err(|e| e.to_string())?;
    let am_assets = parse_assetmap(&am);

    let pkl_map = parse_pkls(dir);
    let pkl_files = find_pkls(dir);

    let cpls = find_cpls(dir);
    if cpls.is_empty() {
        return Err("no CPL found".into());
    }
    let standard = detect_standard(&cpls);

    // Title the subdir after the input's (first) CPL, else the input dir name.
    let subdir = interop_subdir(&cpls, dir, index);

    let mut assets = Vec::new();
    for (id, rel_path, is_pkl) in am_assets {
        if is_pkl {
            continue;
        }
        let abs_path = resolve_asset_path(dir, &rel_path);
        if !abs_path.is_file() {
            return Err(format!("ASSETMAP references missing file {rel_path}"));
        }
        let (hash, size, asset_type) = match pkl_map.get(&id) {
            Some(a) => (a.0.clone(), a.1, a.2.clone()),
            None => {
                // Not in any PKL (rare): hash it ourselves so the merged PKL is complete.
                let h = crate::hash::hash_file(&abs_path)?;
                let s = std::fs::metadata(&abs_path).map(|m| m.len()).unwrap_or(0);
                (h, s, guess_type(&abs_path))
            }
        };
        let kind = classify(&abs_path);
        let title = if kind == AssetKind::Cpl {
            cpl_title(&abs_path)
        } else {
            String::new()
        };
        assets.push(InputAsset {
            id,
            filename: file_name(&abs_path),
            abs_path,
            hash,
            size,
            asset_type,
            kind,
            title,
        });
    }

    Ok(InputDcp {
        standard,
        assets,
        pkls: pkl_files,
        subdir,
    })
}

fn classify(path: &Path) -> AssetKind {
    if is_cpl_path(path) {
        AssetKind::Cpl
    } else if has_ext(path, "mxf") {
        AssetKind::Mxf
    } else {
        AssetKind::Loose
    }
}

/// Derive a filesystem-safe subdirectory name for an input's loose assets.
fn interop_subdir(cpls: &[PathBuf], dir: &Path, index: usize) -> String {
    let base = cpls
        .first()
        .map(|c| cpl_title(c))
        .filter(|t| !t.is_empty())
        .or_else(|| dir.file_name().and_then(|n| n.to_str()).map(String::from))
        .unwrap_or_default();
    let mut s: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    s = s.trim_matches('_').to_string();
    if s.is_empty() {
        format!("dcp_{index}")
    } else {
        s
    }
}

/// Parse ASSETMAP <Asset> blocks into (id, path, is_packing_list), in order.
fn parse_assetmap(xml: &str) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for block in xml.split("<Asset>").skip(1) {
        let block = block.split("</Asset>").next().unwrap_or("");
        let Some(id) = tag(block, "Id").map(|v| v.replace("urn:uuid:", "")) else {
            continue;
        };
        let Some(path) = tag(block, "Path") else {
            continue;
        };
        let is_pkl = block.contains("<PackingList>true</PackingList>");
        out.push((id, path, is_pkl));
    }
    out
}

/// id -> (hash, size, type) across every PKL in the directory.
fn parse_pkls(dir: &Path) -> HashMap<String, (String, u64, String)> {
    let mut map = HashMap::new();
    for path in find_pkls(dir) {
        let Ok(content) = std::fs::read_to_string(&path) else {
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
            let asset_type = tag(block, "Type").unwrap_or_else(|| "application/mxf".into());
            map.insert(id, (hash, size, asset_type));
        }
    }
    map
}

fn find_pkls(dir: &Path) -> Vec<PathBuf> {
    let mut pkls = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = file_name(&entry.path());
            if name.starts_with("PKL") && name.ends_with(".xml") {
                pkls.push(entry.path());
            }
        }
    }
    pkls.sort();
    pkls
}

fn find_cpls(dir: &Path) -> Vec<PathBuf> {
    let mut cpls = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if is_cpl_path(&entry.path()) {
                cpls.push(entry.path());
            }
        }
    }
    cpls.sort();
    cpls
}

fn read_pkl_id(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    uuids_in(&content).into_iter().next()
}

fn cpl_title(path: &Path) -> String {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| tag(&c, "ContentTitleText"))
        .unwrap_or_default()
}

/// Resolve an ASSETMAP <Path> (may contain a subdirectory) against the DCP root.
fn resolve_asset_path(dir: &Path, rel: &str) -> PathBuf {
    dir.join(rel)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_root_name_renames_on_collision() {
        let mut used = HashSet::new();
        assert_eq!(unique_root_name("a.mxf", "0123456789", &mut used), "a.mxf");
        assert_eq!(
            unique_root_name("a.mxf", "0123456789", &mut used),
            "a_01234567.mxf"
        );
    }

    #[test]
    fn interop_subdir_sanitizes_title() {
        // no cpl -> falls back to dir name
        let dir = tempfile::tempdir().unwrap();
        let s = interop_subdir(&[], dir.path(), 0);
        assert!(!s.is_empty());
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }
}
