//! Read an AAF composition through libaaf.
//!
//! libaaf is GPL-2.0-or-later, which this AGPL-3.0-or-later repository takes
//! under the "or later" clause as GPL-3.0.
//!
//! The C side is narrowed by `shim/aaf_shim.c` to the flat item list conform
//! needs, so nothing here mirrors libaaf's own structs.

use std::ffi::{CStr, CString, c_char};
use std::path::Path;

const ERROR_BUFFER_LENGTH: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum AafError {
    #[error("AAF path is not valid UTF-8: {0}")]
    Path(String),
    #[error("libaaf could not read {path}{}", reason_suffix(.reason))]
    Open { path: String, reason: String },
}

fn reason_suffix(reason: &str) -> String {
    if reason.is_empty() {
        String::new()
    } else {
        format!(": {reason}")
    }
}

/// An AAF edit rate: units per second, as counted in AAF (audio is usually
/// counted in samples, picture in frames).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditRate {
    pub numerator: i32,
    pub denominator: i32,
}

impl EditRate {
    pub fn units_per_second(&self) -> Option<f64> {
        (self.numerator > 0 && self.denominator > 0)
            .then(|| f64::from(self.numerator) / f64::from(self.denominator))
    }
}

/// What one item on an AAF track is. Everything that is not a clip with a
/// source keeps its own kind so the caller can name it when it cannot be
/// conformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AafItemKind {
    AudioClip,
    VideoClip,
    Transition,
    ClipWithoutSource,
    Unknown,
}

/// One item on a track, in the order libaaf resolved it. `position`, `length`
/// and `source_offset` are all counted in `edit_rate`.
#[derive(Debug, Clone)]
pub struct AafItem {
    pub kind: AafItemKind,
    /// MasterMob name: the tape or file name the editor showed.
    pub source_name: String,
    /// URI from the source's network locator, not decoded. Empty when the AAF
    /// carries the essence itself.
    pub source_path: String,
    pub track_name: String,
    pub track_number: u32,
    pub position: i64,
    pub length: i64,
    /// Start of the clip inside its source, from SourceClip::StartTime.
    pub source_offset: i64,
    pub edit_rate: Option<EditRate>,
}

/// A composition read from an AAF file.
#[derive(Debug, Clone)]
pub struct AafComposition {
    pub name: String,
    /// Timecode start, counted in `start_rate`.
    pub start: i64,
    pub start_rate: Option<EditRate>,
    /// Picture edit rate, when the file declares one.
    pub frame_rate: Option<EditRate>,
    pub timecode_frames_per_second: u16,
    pub timecode_drop: bool,
    pub items: Vec<AafItem>,
}

impl AafComposition {
    pub fn read(path: &Path) -> Result<Self, AafError> {
        let text = path
            .to_str()
            .ok_or_else(|| AafError::Path(path.display().to_string()))?;
        let c_path = CString::new(text).map_err(|_| AafError::Path(path.display().to_string()))?;
        let mut error = [0 as c_char; ERROR_BUFFER_LENGTH];

        // SAFETY: c_path is NUL terminated and the buffer matches the length
        // passed alongside it.
        let reader =
            unsafe { ffi::aaf_shim_open(c_path.as_ptr(), error.as_mut_ptr(), ERROR_BUFFER_LENGTH) };
        if reader.is_null() {
            return Err(AafError::Open {
                path: path.display().to_string(),
                // SAFETY: the shim always NUL terminates the buffer.
                reason: unsafe { CStr::from_ptr(error.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
            });
        }
        let reader = Reader(reader);

        let mut composition = ffi::AafShimComposition::default();
        // SAFETY: reader is non-null and composition is a live local.
        unsafe { ffi::aaf_shim_composition(reader.0, &mut composition) };

        let items = (0..composition.item_count)
            .filter_map(|index| {
                // SAFETY: the shim returns null past the end of its item list,
                // and the pointer stays valid while reader is alive.
                let item = unsafe { ffi::aaf_shim_item(reader.0, index).as_ref()? };
                Some(AafItem {
                    kind: match item.kind {
                        0 => AafItemKind::AudioClip,
                        1 => AafItemKind::VideoClip,
                        2 => AafItemKind::Transition,
                        3 => AafItemKind::ClipWithoutSource,
                        _ => AafItemKind::Unknown,
                    },
                    source_name: owned(item.source_name),
                    source_path: owned(item.source_path),
                    track_name: owned(item.track_name),
                    track_number: item.track_number,
                    position: item.position,
                    length: item.length,
                    source_offset: item.source_offset,
                    edit_rate: rate(item.edit_rate_numerator, item.edit_rate_denominator),
                })
            })
            .collect();

        Ok(Self {
            name: owned(composition.name),
            start: composition.start,
            start_rate: rate(
                composition.start_rate_numerator,
                composition.start_rate_denominator,
            ),
            frame_rate: rate(
                composition.frame_rate_numerator,
                composition.frame_rate_denominator,
            ),
            timecode_frames_per_second: composition.timecode_fps,
            timecode_drop: composition.timecode_drop != 0,
            items,
        })
    }
}

fn rate(numerator: i32, denominator: i32) -> Option<EditRate> {
    let rate = EditRate {
        numerator,
        denominator,
    };
    rate.units_per_second().is_some().then_some(rate)
}

/// Copy a string libaaf owns. Its buffers die with the reader, so nothing here
/// keeps a borrow.
fn owned(text: *const c_char) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: libaaf NUL terminates every string it exposes, and the caller
    // holds the reader that owns it.
    unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned()
}

/// Owns the C reader so every early return still closes it.
struct Reader(*mut ffi::AafShimReader);

impl Drop for Reader {
    fn drop(&mut self) {
        // SAFETY: the pointer came from aaf_shim_open and is closed once.
        unsafe { ffi::aaf_shim_close(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// libaaf ships its own AAF corpus, so the fixtures are read from the
    /// submodule in place rather than copied in. Returns None when the
    /// submodule is not checked out.
    fn fixture(name: &str) -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extern/libaaf/test/aaf")
            .join(name);
        path.is_file().then_some(path)
    }

    #[test]
    fn reads_clips_with_sources_and_edit_rates() {
        let Some(path) = fixture("DR_Mono_Clip_Positioning.aaf") else {
            return;
        };
        let composition = AafComposition::read(&path).unwrap();

        assert_eq!(composition.name, "DR_Mono_Clip_Positioning");
        assert_eq!(composition.timecode_frames_per_second, 24);
        assert!(!composition.timecode_drop);
        assert_eq!(
            composition.frame_rate,
            Some(EditRate {
                numerator: 24,
                denominator: 1
            })
        );

        let clips: Vec<&AafItem> = composition
            .items
            .iter()
            .filter(|item| item.kind == AafItemKind::AudioClip)
            .collect();
        assert_eq!(clips.len(), 4);
        assert_eq!(clips[0].source_name, "1000hz-18dbs16b44");
        assert!(clips[0].source_path.ends_with("1000hz-18dbs16b44.1k.wav"));
        assert_eq!(clips[0].track_number, 1);
        // audio is counted in samples, and positions run from the composition
        // start rather than from the timecode start
        assert_eq!(
            clips[0].edit_rate,
            Some(EditRate {
                numerator: 48000,
                denominator: 1
            })
        );
        assert_eq!(clips[0].position, 0);
        assert_eq!(clips[0].length, 132300 * 48000 / 44100);
        assert_eq!(clips[0].source_offset, 0);
        assert_eq!(clips[1].position, 183750 * 48000 / 44100);

        assert_eq!(composition.start, 86400);
        assert_eq!(
            composition.start_rate,
            Some(EditRate {
                numerator: 24,
                denominator: 1
            })
        );
    }

    #[test]
    fn one_clip_per_source_file_when_channels_come_from_several() {
        let Some(path) = fixture("PT_Multichannel_stereo_multi_source.aaf") else {
            return;
        };
        let composition = AafComposition::read(&path).unwrap();

        let sources: Vec<&str> = composition
            .items
            .iter()
            .filter(|item| item.kind == AafItemKind::AudioClip)
            .map(|item| item.source_name.as_str())
            .collect();
        assert!(
            sources.len() >= 2,
            "a stereo clip built from two mono files should report both: {sources:?}"
        );
        assert_ne!(sources[0], sources[1]);
    }

    #[test]
    fn reading_a_file_that_is_not_aaf_fails_loud() {
        let directory = std::env::temp_dir().join("libaaf-sys-not-an-aaf");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("edit.aaf");
        std::fs::write(&path, b"\xd0\xcf\x11\xe0not really an aaf").unwrap();

        let error = AafComposition::read(&path).unwrap_err();
        assert!(
            matches!(error, AafError::Open { .. }),
            "unexpected error: {error}"
        );
        std::fs::remove_dir_all(&directory).ok();
    }
}

mod ffi {
    use std::ffi::c_char;

    #[repr(C)]
    pub struct AafShimReader {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    pub struct AafShimItem {
        pub kind: i32,
        pub source_name: *const c_char,
        pub source_path: *const c_char,
        pub track_name: *const c_char,
        pub track_number: u32,
        pub position: i64,
        pub length: i64,
        pub source_offset: i64,
        pub edit_rate_numerator: i32,
        pub edit_rate_denominator: i32,
    }

    #[repr(C)]
    pub struct AafShimComposition {
        pub name: *const c_char,
        pub start: i64,
        pub start_rate_numerator: i32,
        pub start_rate_denominator: i32,
        pub frame_rate_numerator: i32,
        pub frame_rate_denominator: i32,
        pub timecode_fps: u16,
        pub timecode_drop: u8,
        pub item_count: i32,
    }

    impl Default for AafShimComposition {
        fn default() -> Self {
            Self {
                name: std::ptr::null(),
                start: 0,
                start_rate_numerator: 0,
                start_rate_denominator: 0,
                frame_rate_numerator: 0,
                frame_rate_denominator: 0,
                timecode_fps: 0,
                timecode_drop: 0,
                item_count: 0,
            }
        }
    }

    unsafe extern "C" {
        pub fn aaf_shim_open(
            path: *const c_char,
            error_out: *mut c_char,
            error_length: usize,
        ) -> *mut AafShimReader;
        pub fn aaf_shim_close(reader: *mut AafShimReader);
        pub fn aaf_shim_composition(reader: *const AafShimReader, out: *mut AafShimComposition);
        pub fn aaf_shim_item(reader: *const AafShimReader, index: i32) -> *const AafShimItem;
    }
}
