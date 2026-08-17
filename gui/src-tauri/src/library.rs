//! The ident / rating-card library, as the panel sees it.
//!
//! The store itself is `dcpwizard_core::library`; this is the thin Tauri seam
//! over it, plus the one shape the panel draws a row from.

use dcpwizard_core::library::{Library, LibraryItem, LibraryItemKind};
use serde::Serialize;

/// One library row the panel draws.
#[derive(Serialize)]
pub struct LibraryRow {
    pub name: String,
    pub kind: String,
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

impl From<LibraryItem> for LibraryRow {
    fn from(item: LibraryItem) -> Self {
        LibraryRow {
            name: item.name,
            kind: item.kind.as_str().to_string(),
            seconds: item.seconds,
            width: item.width,
            height: item.height,
            has_audio: item.has_audio,
        }
    }
}

#[tauri::command]
pub async fn library_list() -> Result<Vec<LibraryRow>, String> {
    Ok(Library::open()
        .items()?
        .into_iter()
        .map(LibraryRow::from)
        .collect())
}

/// Copy media into the library. `duration_seconds` is the hold a still image
/// needs and a video refuses.
#[tauri::command]
pub async fn library_add(
    file: String,
    name: String,
    kind: String,
    duration_seconds: Option<f64>,
) -> Result<LibraryRow, String> {
    let kind = LibraryItemKind::parse(&kind)?;
    let item = Library::open().import(
        std::path::Path::new(&file),
        name.trim(),
        kind,
        duration_seconds,
    )?;
    Ok(LibraryRow::from(item))
}

#[tauri::command]
pub async fn library_remove(name: String) -> Result<(), String> {
    Library::open().remove(&name)
}

/// Whether a file the panel is about to import needs a hold duration asked for.
#[tauri::command]
pub async fn library_needs_duration(file: String) -> bool {
    dcpwizard_core::still::is_still(std::path::Path::new(&file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_carries_what_the_panel_draws() {
        let row = LibraryRow::from(LibraryItem {
            name: "Studio Ident".into(),
            kind: LibraryItemKind::HeadIdent,
            file: "Studio Ident.mov".into(),
            seconds: 8.0,
            width: 1920,
            height: 1080,
            has_audio: true,
        });
        assert_eq!(row.name, "Studio Ident");
        assert_eq!(row.kind, "head-ident");
        assert!(row.has_audio);
    }

    #[test]
    fn an_unknown_kind_is_refused_before_anything_is_copied() {
        assert!(LibraryItemKind::parse("ident").is_err());
        assert!(LibraryItemKind::parse("anti-piracy").is_ok());
    }
}
