//! Managed folder of head idents, tail idents, rating cards and anti-piracy
//! clips, the media a build joins on either side of the feature.
//!
//! Importing copies the file into the folder and records what a build needs to
//! know about it, so a build never depends on where the operator's copy was.
//! Nothing here conforms or encodes: that is [`crate::library_reel`], at build
//! time, against the raster and rate of the job the item is joined onto.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Media files live here, under the library root.
const MEDIA_DIR: &str = "media";
/// The metadata index, under the library root.
const INDEX_FILE: &str = "index.json";

/// What an item is for. A head ident and a rating card both play before the
/// feature, so the kind is what the operator filed it under, not where a build
/// puts it: that is the caller's `--head-item` / `--tail-item` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LibraryItemKind {
    HeadIdent,
    TailIdent,
    RatingCard,
    AntiPiracy,
}

impl LibraryItemKind {
    pub const ALL: [LibraryItemKind; 4] = [
        LibraryItemKind::HeadIdent,
        LibraryItemKind::TailIdent,
        LibraryItemKind::RatingCard,
        LibraryItemKind::AntiPiracy,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            LibraryItemKind::HeadIdent => "head-ident",
            LibraryItemKind::TailIdent => "tail-ident",
            LibraryItemKind::RatingCard => "rating-card",
            LibraryItemKind::AntiPiracy => "anti-piracy",
        }
    }

    pub fn parse(spec: &str) -> Result<Self, String> {
        LibraryItemKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == spec)
            .ok_or_else(|| {
                let names: Vec<&str> = LibraryItemKind::ALL.iter().map(|k| k.as_str()).collect();
                format!(
                    "unknown library item kind '{spec}': one of {}",
                    names.join(", ")
                )
            })
    }
}

impl std::fmt::Display for LibraryItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One item in the library: the copied media plus what a build reads off it
/// without decoding the file again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryItem {
    /// How the CLI and the GUI address the item. Unique across the library.
    pub name: String,
    pub kind: LibraryItemKind,
    /// File name inside the library's media folder.
    pub file: String,
    /// How long the item runs. Probed for a video, given at import for a still,
    /// which has no length of its own. A build turns this into a frame count at
    /// its own edit rate, so seconds rather than frames is what survives an
    /// item being joined onto jobs of different rates.
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

/// A library item joined onto a build, resolved to media on disk. A build reads
/// nothing back out of the library once it holds one of these, so a job carries
/// everything it needs even if the library changes under it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttachedItem {
    pub item: LibraryItem,
    pub media: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LibraryIndex {
    items: Vec<LibraryItem>,
}

/// A name has to survive as a file name on both platforms and be typed on a
/// command line, so it is limited to what is safe on both.
fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a library item needs a name".into());
    }
    let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-');
    if !name.chars().all(ok) {
        return Err(format!(
            "library item name '{name}' may only hold letters, digits, spaces, '.', '_' and '-'"
        ));
    }
    if name.starts_with('.') {
        return Err(format!("library item name '{name}' cannot start with '.'"));
    }
    Ok(())
}

/// The managed folder. Everything the library holds is under `root`: the media
/// files and the index describing them.
pub struct Library {
    root: PathBuf,
}

impl Library {
    /// The library in the app's data dir (`~/.local/share/dcpwizard/library`).
    pub fn open() -> Self {
        Library::open_at(crate::store::data_dir().join("library"))
    }

    pub fn open_at(root: PathBuf) -> Self {
        Library { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn media_dir(&self) -> PathBuf {
        self.root.join(MEDIA_DIR)
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_FILE)
    }

    /// Where the item's media sits on disk.
    pub fn media_path(&self, item: &LibraryItem) -> PathBuf {
        self.media_dir().join(&item.file)
    }

    /// Every item, in import order. A library that was never written is empty
    /// rather than an error.
    pub fn items(&self) -> Result<Vec<LibraryItem>, String> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let index: LibraryIndex = serde_json::from_str(&text)
            .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
        Ok(index.items)
    }

    /// The item filed under `name`, or a loud error naming what is there.
    pub fn get(&self, name: &str) -> Result<LibraryItem, String> {
        let items = self.items()?;
        items
            .iter()
            .find(|item| item.name == name)
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
                if names.is_empty() {
                    format!("no library item named '{name}': the library is empty")
                } else {
                    format!(
                        "no library item named '{name}': the library holds {}",
                        names.join(", ")
                    )
                }
            })
    }

    /// The item filed under `name`, with its media resolved, ready to join onto
    /// a build.
    pub fn attach(&self, name: &str) -> Result<AttachedItem, String> {
        let item = self.get(name)?;
        let media = self.media_path(&item);
        if !media.is_file() {
            return Err(format!(
                "library item '{name}' has lost its media: {} is not there",
                media.display()
            ));
        }
        Ok(AttachedItem { item, media })
    }

    fn write_items(&self, items: &[LibraryItem]) -> Result<(), String> {
        let index = LibraryIndex {
            items: items.to_vec(),
        };
        let json = serde_json::to_string_pretty(&index)
            .map_err(|e| format!("cannot serialise the library index: {e}"))?;
        postkit::fs::write_atomic(&self.index_path(), json.as_bytes())
    }

    /// Copy `source` into the library under `name`.
    ///
    /// `hold_seconds` is how long a still image is held; a video carries its own
    /// length and is refused a hold, since two answers to one question is one
    /// too many.
    pub fn import(
        &self,
        source: &Path,
        name: &str,
        kind: LibraryItemKind,
        hold_seconds: Option<f64>,
    ) -> Result<LibraryItem, String> {
        check_name(name)?;
        if !source.is_file() {
            return Err(format!("{} is not a file", source.display()));
        }
        let mut items = self.items()?;
        if items.iter().any(|item| item.name == name) {
            return Err(format!(
                "the library already holds an item named '{name}': remove it first"
            ));
        }
        let extension = source
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .ok_or_else(|| {
                format!(
                    "{} has no file extension, so nothing here can tell what it is",
                    source.display()
                )
            })?;

        let still = postkit::still::is_still_image(source);
        if !still && hold_seconds.is_some() {
            return Err(format!(
                "{} is a video and carries its own length: a hold applies to a still image",
                source.display()
            ));
        }
        let info = crate::probe::probe_video(source)
            .ok_or_else(|| format!("cannot probe {}", source.display()))?;
        let seconds = if still {
            let hold = hold_seconds.ok_or_else(|| {
                format!(
                    "{} is a still image and has no length: pass a hold duration",
                    source.display()
                )
            })?;
            if !(hold.is_finite() && hold > 0.0) {
                return Err("a still has to be held for a positive number of seconds".into());
            }
            hold
        } else {
            if info.total_frames == 0 || info.fps_num == 0 {
                return Err(format!(
                    "cannot tell how long {} runs: ffprobe read no frames",
                    source.display()
                ));
            }
            info.total_frames as f64 * info.fps_den.max(1) as f64 / info.fps_num as f64
        };

        let file = format!("{name}.{extension}");
        let media_dir = self.media_dir();
        std::fs::create_dir_all(&media_dir)
            .map_err(|e| format!("cannot create {}: {e}", media_dir.display()))?;
        let destination = media_dir.join(&file);
        std::fs::copy(source, &destination).map_err(|e| {
            format!(
                "cannot copy {} into {}: {e}",
                source.display(),
                destination.display()
            )
        })?;

        let item = LibraryItem {
            name: name.to_string(),
            kind,
            file,
            seconds,
            width: info.width,
            height: info.height,
            has_audio: !still && info.has_audio,
        };
        items.push(item.clone());
        if let Err(e) = self.write_items(&items) {
            let _ = std::fs::remove_file(&destination);
            return Err(e);
        }
        Ok(item)
    }

    /// Drop the item and its copied media.
    pub fn remove(&self, name: &str) -> Result<(), String> {
        let mut items = self.items()?;
        let item = self.get(name)?;
        items.retain(|i| i.name != name);
        self.write_items(&items)?;
        let media = self.media_path(&item);
        if media.exists()
            && let Err(e) = std::fs::remove_file(&media)
        {
            return Err(format!("cannot remove {}: {e}", media.display()));
        }
        Ok(())
    }
}

/// How many frames the item runs for at `fps`, at least one: an item shorter
/// than a frame still has to be a frame of picture.
pub fn item_frames(item: &LibraryItem, fps: u32) -> u64 {
    ((item.seconds * fps.max(1) as f64).round() as u64).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open_at(dir.path().join("library"));
        (dir, library)
    }

    #[test]
    fn kinds_round_trip_through_their_spellings() {
        for kind in LibraryItemKind::ALL {
            assert_eq!(LibraryItemKind::parse(kind.as_str()).unwrap(), kind);
        }
        let error = LibraryItemKind::parse("ident").unwrap_err();
        assert!(error.contains("head-ident"), "{error}");
    }

    #[test]
    fn names_are_limited_to_what_both_platforms_can_hold() {
        assert!(check_name("Studio Ident 2026").is_ok());
        assert!(check_name("bbfc_12a-v2.mov").is_ok());
        assert!(check_name("").is_err());
        assert!(check_name("../escape").is_err());
        assert!(check_name("head/ident").is_err());
        assert!(check_name(".hidden").is_err());
    }

    #[test]
    fn an_empty_library_lists_nothing_and_names_what_is_missing() {
        let (_dir, library) = library();
        assert!(library.items().unwrap().is_empty());
        let error = library.get("ident").unwrap_err();
        assert!(error.contains("the library is empty"), "{error}");
    }

    #[test]
    fn a_still_needs_a_hold_and_a_video_refuses_one() {
        let (dir, library) = library();
        let still = dir.path().join("card.png");
        std::fs::write(&still, "not really a png").unwrap();
        let error = library
            .import(&still, "card", LibraryItemKind::RatingCard, None)
            .unwrap_err();
        assert!(error.contains("no length"), "{error}");

        let video = dir.path().join("ident.mov");
        std::fs::write(&video, "not really a mov").unwrap();
        let error = library
            .import(&video, "ident", LibraryItemKind::HeadIdent, Some(5.0))
            .unwrap_err();
        assert!(error.contains("carries its own length"), "{error}");
    }

    #[test]
    fn import_refuses_a_missing_file_and_a_bad_name() {
        let (dir, library) = library();
        let missing = dir.path().join("nowhere.mov");
        assert!(
            library
                .import(&missing, "ident", LibraryItemKind::HeadIdent, None)
                .is_err()
        );
        let present = dir.path().join("card.png");
        std::fs::write(&present, "x").unwrap();
        let error = library
            .import(
                &present,
                "../escape",
                LibraryItemKind::RatingCard,
                Some(2.0),
            )
            .unwrap_err();
        assert!(error.contains("may only hold"), "{error}");
    }

    #[test]
    fn items_survive_a_write_and_a_read_and_removal_takes_the_media_with_it() {
        let (_dir, library) = library();
        let item = LibraryItem {
            name: "Studio Ident".into(),
            kind: LibraryItemKind::HeadIdent,
            file: "Studio Ident.mov".into(),
            seconds: 8.0,
            width: 1920,
            height: 1080,
            has_audio: true,
        };
        library.write_items(std::slice::from_ref(&item)).unwrap();
        std::fs::create_dir_all(library.media_dir()).unwrap();
        std::fs::write(library.media_path(&item), "media").unwrap();

        assert_eq!(library.items().unwrap(), vec![item.clone()]);
        assert_eq!(library.get("Studio Ident").unwrap(), item);

        library.remove("Studio Ident").unwrap();
        assert!(library.items().unwrap().is_empty());
        assert!(!library.media_path(&item).exists());
        assert!(library.remove("Studio Ident").is_err());
    }

    #[test]
    fn a_name_is_claimed_once() {
        let (dir, library) = library();
        let item = LibraryItem {
            name: "card".into(),
            kind: LibraryItemKind::RatingCard,
            file: "card.png".into(),
            seconds: 5.0,
            width: 1998,
            height: 1080,
            has_audio: false,
        };
        library.write_items(&[item]).unwrap();
        let source = dir.path().join("card.png");
        std::fs::write(&source, "x").unwrap();
        let error = library
            .import(&source, "card", LibraryItemKind::RatingCard, Some(5.0))
            .unwrap_err();
        assert!(error.contains("already holds"), "{error}");
    }

    #[test]
    fn a_length_in_seconds_becomes_a_frame_count_at_the_jobs_rate() {
        let item = LibraryItem {
            name: "ident".into(),
            kind: LibraryItemKind::HeadIdent,
            file: "ident.mov".into(),
            seconds: 5.0,
            width: 1920,
            height: 1080,
            has_audio: true,
        };
        assert_eq!(item_frames(&item, 24), 120);
        assert_eq!(item_frames(&item, 25), 125);
        assert_eq!(item_frames(&item, 48), 240);
        // shorter than a frame still has to be a frame of picture
        let blink = LibraryItem {
            seconds: 0.001,
            ..item
        };
        assert_eq!(item_frames(&blink, 24), 1);
    }
}
