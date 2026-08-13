//! Read an AAF composition into a conform [`Timeline`], the same shape the EDL
//! and XML paths produce.
//!
//! AAF counts audio in samples and picture in frames, and clip positions run
//! from the composition start rather than from the timecode start, so every
//! value is converted to frames of the composition rate and offset by the
//! timecode start. That leaves record times reading as timecode, like an EDL.

use libaaf_sys::{AafComposition, AafItem, AafItemKind};
use postkit::conform::{ConformError, EditEvent, Timeline, TimelineFormat};
use std::path::Path;

const FALLBACK_FRAME_RATE: f64 = 24.0;
const UNNAMED_REEL_NAME: &str = "AX";

pub fn read_timeline(file: &Path) -> Result<Timeline, ConformError> {
    let composition = AafComposition::read(file).map_err(|e| ConformError::Aaf(e.to_string()))?;
    let frame_rate = frame_rate(&composition);
    let timecode_start = composition
        .start_rate
        .and_then(|rate| rate.units_per_second())
        .map(|units| to_frames(composition.start, units, frame_rate))
        .unwrap_or(0);

    let mut events = Vec::new();
    let mut skipped = Vec::new();
    let mut transitions: Vec<(String, u32)> = Vec::new();
    for item in &composition.items {
        let units_per_second = item
            .edit_rate
            .and_then(|rate| rate.units_per_second())
            .unwrap_or(frame_rate);
        let record_in = timecode_start + to_frames(item.position, units_per_second, frame_rate);
        let source_in = to_frames(item.source_offset, units_per_second, frame_rate);
        let length = to_frames(item.length, units_per_second, frame_rate);

        let track_type = match item.kind {
            AafItemKind::VideoClip => "V".to_string(),
            AafItemKind::AudioClip => format!("A{}", item.track_number),
            AafItemKind::Transition => {
                count_transition(&mut transitions, track_label(item));
                continue;
            }
            AafItemKind::ClipWithoutSource => {
                skip(
                    &mut skipped,
                    format!(
                        "{} at frame {record_in}: clip has no source essence libaaf could \
                         resolve, so there is nothing to conform",
                        track_label(item)
                    ),
                );
                continue;
            }
            AafItemKind::Unknown => {
                skip(
                    &mut skipped,
                    format!(
                        "{}: libaaf resolved no clip for one of its timeline items",
                        track_label(item)
                    ),
                );
                continue;
            }
        };
        events.push(EditEvent {
            event_number: 0,
            reel_name: reel_name(item),
            track_type,
            source_in,
            source_out: source_in + length,
            record_in,
            record_out: record_in + length,
            transition: "CUT".to_string(),
            comment: item.source_path.clone(),
            lane: 0,
        });
    }

    for (track, count) in transitions {
        skip(
            &mut skipped,
            format!(
                "{track}: {count} transition(s) dropped, a transition is not a source clip and \
                 conform assembles cuts only"
            ),
        );
    }

    if events.is_empty() {
        return Err(ConformError::NoEvents);
    }
    events.sort_by(|a, b| {
        a.record_in
            .cmp(&b.record_in)
            .then_with(|| a.track_type.cmp(&b.track_type))
    });
    for (index, event) in events.iter_mut().enumerate() {
        event.event_number = index as u32 + 1;
    }

    Ok(Timeline {
        title: composition.name,
        frame_rate,
        format: TimelineFormat::Aaf,
        events,
        skipped,
    })
}

/// The composition's picture rate. AAF states NTSC rates exactly (30000/1001),
/// so the drop flag needs no arithmetic of its own.
fn frame_rate(composition: &AafComposition) -> f64 {
    composition
        .frame_rate
        .and_then(|rate| rate.units_per_second())
        .or_else(|| {
            (composition.timecode_frames_per_second > 0)
                .then(|| f64::from(composition.timecode_frames_per_second))
        })
        .unwrap_or(FALLBACK_FRAME_RATE)
}

fn to_frames(units: i64, units_per_second: f64, frame_rate: f64) -> u32 {
    if units_per_second <= 0.0 {
        return 0;
    }
    ((units as f64 / units_per_second) * frame_rate)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32
}

/// The name a reel plan can match against media on disk: the tape or file name
/// the editor recorded, falling back to the file the source URI points at.
fn reel_name(item: &AafItem) -> String {
    if !item.source_name.is_empty() {
        return item.source_name.clone();
    }
    Path::new(&item.source_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(UNNAMED_REEL_NAME)
        .to_string()
}

fn track_label(item: &AafItem) -> String {
    match item.track_name.is_empty() {
        true => format!("track {}", item.track_number),
        false => format!("track \"{}\"", item.track_name),
    }
}

/// libaaf leaves the position of a transition at zero, so they are reported per
/// track rather than one note per fade at a frame that is not real.
fn count_transition(transitions: &mut Vec<(String, u32)>, track: String) {
    match transitions.iter_mut().find(|(name, _)| *name == track) {
        Some((_, count)) => *count += 1,
        None => transitions.push((track, 1)),
    }
}

fn skip(skipped: &mut Vec<String>, note: String) {
    tracing::warn!("aaf: skipped {note}");
    skipped.push(note);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// libaaf ships its own AAF corpus, read from the submodule in place. The
    /// test passes when the submodule is not checked out.
    fn fixture(name: &str) -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extern/libaaf/test/aaf")
            .join(name);
        path.is_file().then_some(path)
    }

    #[test]
    fn reads_a_composition_into_edit_events() {
        let Some(path) = fixture("DR_Mono_Clip_Positioning.aaf") else {
            return;
        };
        let timeline = read_timeline(&path).unwrap();

        assert_eq!(timeline.format, TimelineFormat::Aaf);
        assert_eq!(timeline.title, "DR_Mono_Clip_Positioning");
        assert_eq!(timeline.frame_rate, 24.0);
        assert_eq!(timeline.events.len(), 4);

        // the timecode start is one hour, and clip one sits at the top of it
        let first = &timeline.events[0];
        assert_eq!(first.event_number, 1);
        assert_eq!(first.reel_name, "1000hz-18dbs16b44");
        assert_eq!(first.track_type, "A1");
        assert_eq!(first.record_in, 86400);
        assert_eq!(first.record_out, 86472);
        assert_eq!(first.source_in, 0);
        assert_eq!(first.source_out, 72);
        assert!(first.comment.ends_with("1000hz-18dbs16b44.1k.wav"));

        assert!(
            timeline
                .events
                .windows(2)
                .all(|w| w[0].record_in <= w[1].record_in),
            "events should be in record order"
        );
    }

    #[test]
    fn ntsc_composition_keeps_its_exact_rate() {
        let Some(path) = fixture("MC_TC_29.97_DF.aaf") else {
            return;
        };
        let timeline = read_timeline(&path).unwrap();
        assert!((timeline.frame_rate - 30000.0 / 1001.0).abs() < 1e-9);
        // one clip, three seconds long, ten seconds past an hour of timecode
        assert_eq!(timeline.events[0].reel_name, "1000hz-18dbs16b44.1k.wav");
        assert_eq!(timeline.events[0].record_in, 107892 + 300);
        assert_eq!(timeline.events[0].record_out, 107892 + 390);
    }

    #[test]
    fn transitions_are_skipped_out_loud() {
        let Some(path) = fixture("MC_Fades.aaf") else {
            return;
        };
        let timeline = read_timeline(&path).unwrap();
        // one note per track, naming how many fades it dropped
        assert!(
            timeline
                .skipped
                .iter()
                .any(|note| note.contains("4 transition(s) dropped")),
            "{:?}",
            timeline.skipped
        );
    }

    #[test]
    fn a_file_libaaf_cannot_read_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.aaf");
        std::fs::write(&path, b"\xd0\xcf\x11\xe0not really an aaf").unwrap();
        let error = read_timeline(&path).unwrap_err();
        assert!(matches!(error, ConformError::Aaf(_)), "{error}");
    }

    #[test]
    fn a_composition_without_clips_fails_loud() {
        let Some(path) = fixture("DR_Empty.aaf") else {
            return;
        };
        assert!(matches!(
            read_timeline(&path),
            Err(ConformError::NoEvents) | Err(ConformError::Aaf(_))
        ));
    }
}
