//! Edit a DCP's CPL metadata without re-wrapping essence (dom#1127).
//!
//! The rewrite is postkit's `package_edit::edit_package`, which does the same job
//! for an IMP. This is the CLI's shape of it: an `i32` exit code and the log
//! lines the GUI's retitle reads.
//!
//! Note (dom#1127 scope): the digest also mentions reel reorder / length edits.
//! This command covers the metadata fields only; reel surgery is out of scope.

use postkit::package_edit::{PackageEdit, edit_package};
use std::path::PathBuf;

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
    let edited = match edit_package(&PackageEdit {
        input: config.input.clone(),
        output: config.output.clone(),
        title: config.title.clone(),
        annotation: config.annotation.clone(),
        content_kind: config.content_kind.clone(),
        issuer: config.issuer.clone(),
    }) {
        Ok(edited) => edited,
        Err(e) => {
            tracing::error!("{e}");
            return -1;
        }
    };

    if !edited.unsigned_documents.is_empty() {
        tracing::warn!(
            "dropped the signature from {}: the edit changes the bytes it covered. \
             re-sign the package if it has to stay signed",
            edited.unsigned_documents.join(", ")
        );
    }
    tracing::info!(
        "edited CPL metadata in {} (new CPL id {})",
        edited.package_dir.display(),
        edited.composition_id
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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

    fn only_cpl(dir: &Path) -> PathBuf {
        let cpls: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("CPL"))
            })
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

        let new_id = cpl_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .trim_start_matches("CPL_")
            .to_string();
        let pkl = std::fs::read_to_string(
            dir.path()
                .join("PKL_33333333-3333-3333-3333-333333333333.xml"),
        )
        .unwrap();
        assert!(pkl.contains(&new_id), "PKL must point at the new CPL id");
        assert!(!pkl.contains("oldhash="), "PKL must carry the new hash");

        let assetmap = std::fs::read_to_string(dir.path().join("ASSETMAP.xml")).unwrap();
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

        let cpl = std::fs::read_to_string(dir.path().join(format!("CPL_{OLD_ID}.xml"))).unwrap();
        assert!(cpl.contains(OLD_TITLE), "nothing may change");
    }
}
