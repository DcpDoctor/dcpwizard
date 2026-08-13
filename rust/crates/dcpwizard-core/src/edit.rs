//! Edit a DCP's CPL metadata without re-wrapping essence (dom#1127).
//!
//! Rewrites the CPL's title / content kind / issuer / annotation and gives it a
//! NEW composition id (a changed CPL is a different composition), then updates
//! the CPL's PKL and ASSETMAP entries (new id, new hash/size). Essence MXFs are
//! never touched: their asset ids and bytes stay identical. Encrypted DCPs are
//! refused because the KDM binds the CPL id. Every document rewritten here loses
//! its signature, which the rewrite invalidates.
//!
//! Note (dom#1127 scope): the digest also mentions reel reorder / length edits.
//! This command covers the metadata fields only; reel surgery is out of scope.

use crate::ingest_package::{file_name, is_cpl_path, tag};
use std::path::{Path, PathBuf};

/// What to change on the DCP's CPL. `None` fields are left as-is.
#[derive(Debug, Clone, Default)]
pub struct EditConfig {
    pub input: PathBuf,
    /// Write the edited DCP here (copied first). None edits in place.
    pub output: Option<PathBuf>,
    pub title: Option<String>,
    pub annotation: Option<String>,
    pub content_kind: Option<String>,
    pub issuer: Option<String>,
}

/// Apply the edits. Returns 0 on success.
pub fn edit_dcp(config: &EditConfig) -> i32 {
    if !config.input.is_dir() {
        tracing::error!("input is not a directory: {}", config.input.display());
        return -1;
    }
    if config.title.is_none()
        && config.annotation.is_none()
        && config.content_kind.is_none()
        && config.issuer.is_none()
    {
        tracing::error!(
            "nothing to edit; pass at least one of --title/--annotation/--content-kind/--issuer"
        );
        return -1;
    }

    // Resolve the working directory: --output copies the whole DCP first so the
    // source is left untouched; in place otherwise.
    let work = match config.output.as_ref() {
        Some(out) => {
            if let Err(e) = copy_dir(&config.input, out) {
                tracing::error!("cannot copy DCP to {}: {e}", out.display());
                return -1;
            }
            out.clone()
        }
        None => config.input.clone(),
    };

    let cpls: Vec<PathBuf> = match std::fs::read_dir(&work) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_cpl_path(p))
            .collect(),
        Err(e) => {
            tracing::error!("cannot read {}: {e}", work.display());
            return -1;
        }
    };
    if cpls.is_empty() {
        tracing::error!("no CPL found in {}", work.display());
        return -1;
    }
    if cpls.len() > 1 {
        tracing::error!("multiple CPLs found; edit operates on a single-composition DCP");
        return -1;
    }
    let cpl_path = &cpls[0];

    let cpl = match std::fs::read_to_string(cpl_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("cannot read CPL: {e}");
            return -1;
        }
    };
    if cpl.contains("<KeyId>") {
        tracing::error!(
            "DCP is encrypted; the KDM binds the CPL id, so a metadata edit (new CPL id) would \
             invalidate every KDM. Edit refuses encrypted DCPs"
        );
        return -1;
    }

    let Some(old_id) = tag(&cpl, "Id").map(|v| v.replace("urn:uuid:", "")) else {
        tracing::error!("CPL has no Id");
        return -1;
    };
    let new_id = uuid::Uuid::new_v4().to_string();

    // ── rewrite the CPL text ──
    // the composition id appears once in the CPL (nothing references it), so a
    // plain replace is safe.
    let mut xml = cpl.replace(&old_id, &new_id);
    if let Some(title) = config.title.as_deref() {
        replace_element(&mut xml, "<ContentTitleText>", "</ContentTitleText>", title);
        // keep the ST 429-16 metadata title consistent when present
        replace_element(
            &mut xml,
            "<meta:FullContentTitleText>",
            "</meta:FullContentTitleText>",
            title,
        );
    }
    if let Some(kind) = config.content_kind.as_deref() {
        let kind = normalize_kind(kind);
        replace_element(&mut xml, "<ContentKind>", "</ContentKind>", &kind);
    }
    if let Some(issuer) = config.issuer.as_deref() {
        replace_element(&mut xml, "<Issuer>", "</Issuer>", issuer);
    }
    if let Some(annotation) = config.annotation.as_deref() {
        set_cpl_annotation(&mut xml, annotation);
    }

    // ── write the CPL under its new id, drop the old file ──
    let new_cpl_name = format!("CPL_{new_id}.xml");
    let new_cpl_path = work.join(&new_cpl_name);
    // strip before the write, so the hash below covers what actually lands
    let mut unsigned: Vec<String> = Vec::new();
    if crate::package_signature::strip_signature(&mut xml) {
        unsigned.push(new_cpl_name.clone());
    }
    if let Err(e) = write_atomic(&new_cpl_path, xml.as_bytes()) {
        tracing::error!("cannot write new CPL: {e}");
        return -1;
    }
    let old_cpl_name = file_name(cpl_path);
    if old_cpl_name != new_cpl_name
        && let Err(e) = std::fs::remove_file(cpl_path)
    {
        tracing::error!("cannot remove old CPL {}: {e}", cpl_path.display());
        return -1;
    }

    let new_hash = crate::hash::hash_file(&new_cpl_path).unwrap_or_default();
    let new_size = std::fs::metadata(&new_cpl_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // ── update the PKL(s): the CPL asset's id, hash, size ──
    let mut patched_pkl = false;
    for pkl in pkls(&work) {
        let Ok(content) = std::fs::read_to_string(&pkl) else {
            continue;
        };
        if !content.contains(&old_id) {
            continue;
        }
        let mut updated = patch_pkl_cpl_entry(&content, &old_id, &new_id, &new_hash, new_size);
        if crate::package_signature::strip_signature(&mut updated) {
            unsigned.push(file_name(&pkl));
        }
        if let Err(e) = write_atomic(&pkl, updated.as_bytes()) {
            tracing::error!("cannot update PKL {}: {e}", pkl.display());
            return -1;
        }
        patched_pkl = true;
    }
    if !patched_pkl {
        tracing::error!("no PKL references the CPL id; package is inconsistent");
        return -1;
    }

    // ── update the ASSETMAP: the CPL id and its <Path> ──
    let am_path = ["ASSETMAP.xml", "ASSETMAP"]
        .iter()
        .map(|n| work.join(n))
        .find(|p| p.exists());
    let Some(am_path) = am_path else {
        tracing::error!("no ASSETMAP found");
        return -1;
    };
    let Ok(am) = std::fs::read_to_string(&am_path) else {
        tracing::error!("cannot read ASSETMAP");
        return -1;
    };
    let mut am = am
        .replace(&old_id, &new_id)
        .replace(&old_cpl_name, &new_cpl_name);
    if crate::package_signature::strip_signature(&mut am) {
        unsigned.push(file_name(&am_path));
    }
    if let Err(e) = write_atomic(&am_path, am.as_bytes()) {
        tracing::error!("cannot update ASSETMAP: {e}");
        return -1;
    }

    if !unsigned.is_empty() {
        tracing::warn!(
            "dropped the signature from {}: the edit changes the bytes it covered. \
             re-sign the package if it has to stay signed",
            unsigned.join(", ")
        );
    }

    tracing::info!(
        "edited CPL metadata in {} (new CPL id {new_id})",
        work.display()
    );
    0
}

/// Map an abbreviation (FTR, ...) to its CPL kind, else pass the value through.
fn normalize_kind(kind: &str) -> String {
    crate::ContentType::from_abbrev(kind)
        .map(|c| c.as_cpl_kind().to_string())
        .unwrap_or_else(|| kind.to_string())
}

/// Replace the inner text of the first `open`..`close` element (no attributes).
fn replace_element(xml: &mut String, open: &str, close: &str, value: &str) {
    let Some(start) = xml.find(open) else {
        return;
    };
    let inner_start = start + open.len();
    let Some(rel) = xml[inner_start..].find(close) else {
        return;
    };
    let inner_end = inner_start + rel;
    let escaped = postkit::packaging::escape_xml(value);
    xml.replace_range(inner_start..inner_end, &escaped);
}

/// Set the CPL-level AnnotationText: replace it if present before `<ReelList>`,
/// else insert one right after the composition `<Id>` line (ST 429-7 order).
fn set_cpl_annotation(xml: &mut String, value: &str) {
    let reel_pos = xml.find("<ReelList>").unwrap_or(xml.len());
    if let Some(ann_pos) = xml[..reel_pos].find("<AnnotationText>") {
        let inner = ann_pos + "<AnnotationText>".len();
        if let Some(rel) = xml[inner..].find("</AnnotationText>") {
            let escaped = postkit::packaging::escape_xml(value);
            xml.replace_range(inner..inner + rel, &escaped);
            return;
        }
    }
    // insert after the first <Id>...</Id> line (the composition id)
    if let Some(id_end) = xml.find("</Id>") {
        let after = id_end + "</Id>\n".len().min(xml.len() - id_end);
        // find the real end of the line
        let insert_at = xml[id_end..]
            .find('\n')
            .map(|n| id_end + n + 1)
            .unwrap_or(after);
        let line = format!(
            "  <AnnotationText>{}</AnnotationText>\n",
            postkit::packaging::escape_xml(value)
        );
        xml.insert_str(insert_at, &line);
    }
}

/// Rewrite the CPL asset's block in a PKL: new id, hash, size.
fn patch_pkl_cpl_entry(
    content: &str,
    old_id: &str,
    new_id: &str,
    new_hash: &str,
    new_size: u64,
) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(pos) = rest.find("<Asset>") {
        let block_start = pos;
        let block_end = rest[pos..]
            .find("</Asset>")
            .map(|e| pos + e + "</Asset>".len())
            .unwrap_or(rest.len());
        out.push_str(&rest[..block_start]);
        let block = &rest[block_start..block_end];
        if block.contains(old_id) {
            let mut b = block.replace(old_id, new_id);
            replace_element(&mut b, "<Hash>", "</Hash>", new_hash);
            replace_element(&mut b, "<Size>", "</Size>", &new_size.to_string());
            out.push_str(&b);
        } else {
            out.push_str(block);
        }
        rest = &rest[block_end..];
    }
    out.push_str(rest);
    out
}

fn pkls(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = file_name(p);
            n.starts_with("PKL") && n.ends_with(".xml")
        })
        .collect()
}

/// Write `bytes` to `path` atomically: a temp file in the same dir, then rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp_{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Copy every file in `src` (flat DCP directory) into `dst`.
fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let path = entry.path();
        if path.is_file() {
            std::fs::copy(&path, dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD_ID: &str = "11111111-1111-1111-1111-111111111111";
    const OLD_TITLE: &str = "OLD-TITLE_FTR_S_EN-XX_51_2K";
    const ESSENCE: &[u8] = b"picture essence";

    /// A flat package with one CPL, one PKL and an ASSETMAP, all cross
    /// referencing the CPL id, plus an essence file the edit must not touch.
    fn write_package(dir: &Path, key_id: Option<&str>) {
        let key = key_id
            .map(|k| format!("<KeyId>urn:uuid:{k}</KeyId>"))
            .unwrap_or_default();
        std::fs::write(
            dir.join(format!("CPL_{OLD_ID}.xml")),
            format!(
                r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:{OLD_ID}</Id>
  <ContentTitleText>{OLD_TITLE}</ContentTitleText>
  <ContentKind>feature</ContentKind>
  <Issuer>DCP Wizard</Issuer>
  <ReelList><Reel><AssetList><MainPicture>
    <Id>urn:uuid:22222222-2222-2222-2222-222222222222</Id>{key}
  </MainPicture></AssetList></Reel></ReelList>
</CompositionPlaylist>
"#
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("PKL_33333333-3333-3333-3333-333333333333.xml"),
            format!(
                r#"<?xml version="1.0"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">
  <AssetList>
    <Asset>
      <Id>urn:uuid:{OLD_ID}</Id>
      <Hash>oldhash=</Hash>
      <Size>1</Size>
      <Type>text/xml</Type>
    </Asset>
  </AssetList>
</PackingList>
"#
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("ASSETMAP.xml"),
            format!(
                r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <AssetList><Asset>
    <Id>urn:uuid:{OLD_ID}</Id>
    <ChunkList><Chunk><Path>CPL_{OLD_ID}.xml</Path></Chunk></ChunkList>
  </Asset></AssetList>
</AssetMap>
"#
            ),
        )
        .unwrap();
        std::fs::write(dir.join("picture.mxf"), ESSENCE).unwrap();
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    fn only_cpl(dir: &Path) -> PathBuf {
        let cpls: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| file_name(p).starts_with("CPL"))
            .collect();
        assert_eq!(cpls.len(), 1, "exactly one CPL should remain");
        cpls.into_iter().next().unwrap()
    }

    #[test]
    fn a_retitle_mints_a_new_composition_id_and_leaves_essence_alone() {
        let dir = tempfile::tempdir().unwrap();
        write_package(dir.path(), None);

        let code = edit_dcp(&EditConfig {
            input: dir.path().to_path_buf(),
            title: Some("NEW-TITLE_FTR_S_EN-XX_51_2K".into()),
            ..Default::default()
        });
        assert_eq!(code, 0);

        let cpl_path = only_cpl(dir.path());
        let cpl = std::fs::read_to_string(&cpl_path).unwrap();
        assert!(cpl.contains("<ContentTitleText>NEW-TITLE_FTR_S_EN-XX_51_2K<"));
        assert!(!cpl.contains(OLD_ID), "the composition id must change");
        // the picture asset keeps its own id, only the composition is new
        assert!(cpl.contains("22222222-2222-2222-2222-222222222222"));

        let new_id = file_name(&cpl_path)
            .trim_start_matches("CPL_")
            .trim_end_matches(".xml")
            .to_string();
        let pkl = read(dir.path(), "PKL_33333333-3333-3333-3333-333333333333.xml");
        assert!(pkl.contains(&new_id), "PKL must point at the new CPL id");
        assert!(!pkl.contains("oldhash="), "PKL must carry the new hash");

        let assetmap = read(dir.path(), "ASSETMAP.xml");
        assert!(assetmap.contains(&new_id));
        assert!(assetmap.contains(&format!("CPL_{new_id}.xml")));
        assert!(!assetmap.contains(OLD_ID));

        assert_eq!(
            std::fs::read(dir.path().join("picture.mxf")).unwrap(),
            ESSENCE,
            "essence must be untouched"
        );
    }

    #[test]
    fn an_encrypted_package_is_refused_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        write_package(dir.path(), Some("44444444-4444-4444-4444-444444444444"));

        let code = edit_dcp(&EditConfig {
            input: dir.path().to_path_buf(),
            title: Some("NEW-TITLE".into()),
            ..Default::default()
        });
        assert_eq!(code, -1, "a KDM is bound to the CPL id");

        let cpl = read(dir.path(), &format!("CPL_{OLD_ID}.xml"));
        assert!(cpl.contains(OLD_TITLE), "nothing may change");
    }

    #[test]
    fn an_edit_with_nothing_to_change_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_package(dir.path(), None);
        let code = edit_dcp(&EditConfig {
            input: dir.path().to_path_buf(),
            ..Default::default()
        });
        assert_eq!(code, -1);
    }

    #[test]
    fn an_output_copy_leaves_the_source_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("ov");
        std::fs::create_dir(&source).unwrap();
        write_package(&source, None);
        let copy = dir.path().join("retitled");

        let code = edit_dcp(&EditConfig {
            input: source.clone(),
            output: Some(copy.clone()),
            title: Some("NEW-TITLE".into()),
            ..Default::default()
        });
        assert_eq!(code, 0);

        assert!(read(&source, &format!("CPL_{OLD_ID}.xml")).contains(OLD_TITLE));
        assert!(
            std::fs::read_to_string(only_cpl(&copy))
                .unwrap()
                .contains("NEW-TITLE")
        );
    }
}
