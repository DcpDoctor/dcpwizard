//! The subtitle file the embedded preview hands mpv.
//!
//! libass reads SRT, ASS/SSA and WebVTT and nothing else, so every other format
//! the job takes, and the timed text a built DCP packages, is written out as SRT
//! before the preview can show it. Times stay source-relative for a source file
//! and composition-relative for a package, which is what the preview plays.

use std::path::{Path, PathBuf};

use crate::subtitle_extract::{Cue, PackagedTrack, extract_track_cues, to_srt};

/// What libass reads as it stands, so the preview opens the file the job
/// packages rather than a copy of it.
const PLAYABLE_EXTENSIONS: [&str; 5] = ["srt", "ass", "ssa", "vtt", "webvtt"];

/// What `subtitle_extract` reads: ST 428-7 DCST or Interop DCSubtitle, loose or
/// MXF wrapped.
const PACKAGED_EXTENSIONS: [&str; 2] = ["xml", "mxf"];

/// One file per preview slot, so a second preview replaces the track it wrote
/// last time instead of filling the work dir.
const SUBTITLE_PREVIEW_FILE: &str = "preview-subtitle.srt";
const CAPTION_PREVIEW_FILE: &str = "preview-closed-caption.srt";

/// A subtitle file the preview player can render, writing the cues out as SRT
/// under `work_dir` when mpv cannot read the input as it stands.
///
/// `input` is a subtitle file the job packages or a built DCP directory, and
/// `track` picks which timed-text track of a DCP is read. `fps` reads the
/// frame-form times of the formats that carry them.
pub fn playable_subtitle_file(
    input: &Path,
    track: PackagedTrack,
    fps: u32,
    work_dir: &Path,
) -> Result<PathBuf, String> {
    let extension = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if input.is_file() && PLAYABLE_EXTENSIONS.contains(&extension.as_str()) {
        return Ok(input.to_path_buf());
    }

    let cues = preview_cues(input, track, fps, &extension)?;
    if cues.is_empty() {
        return Err(format!(
            "no cues the preview can show in {}",
            input.display()
        ));
    }
    std::fs::create_dir_all(work_dir)
        .map_err(|e| format!("cannot create {}: {e}", work_dir.display()))?;
    let output = work_dir.join(match track {
        PackagedTrack::Subtitle => SUBTITLE_PREVIEW_FILE,
        PackagedTrack::ClosedCaption => CAPTION_PREVIEW_FILE,
    });
    std::fs::write(&output, to_srt(&cues))
        .map_err(|e| format!("cannot write {}: {e}", output.display()))?;
    Ok(output)
}

fn preview_cues(
    input: &Path,
    track: PackagedTrack,
    fps: u32,
    extension: &str,
) -> Result<Vec<Cue>, String> {
    if input.is_dir() || PACKAGED_EXTENSIONS.contains(&extension) {
        return extract_track_cues(input, track);
    }
    let styled = crate::subtitle::load_styled_cues(input, fps)?;
    Ok(styled
        .iter()
        .filter_map(|cue| {
            let text = cue
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>();
            (!text.trim().is_empty()).then_some(Cue {
                start_ms: cue.start_ms,
                end_ms: cue.end_ms,
                text,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: u32 = 24;

    #[test]
    fn dcst_xml_becomes_srt_the_preview_can_show() {
        let dir = tempfile::tempdir().unwrap();
        let xml = dir.path().join("subs.xml");
        std::fs::write(
            &xml,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:TimeCodeRate>24</dcst:TimeCodeRate>
  <dcst:SubtitleList>
    <dcst:Font>
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:01:12" TimeOut="00:00:03:00">
        <dcst:Text>First line</dcst:Text>
        <dcst:Text>second line</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#,
        )
        .unwrap();
        let work = dir.path().join("preview-subtitles");

        let output = playable_subtitle_file(&xml, PackagedTrack::Subtitle, FPS, &work).unwrap();

        assert_eq!(output, work.join(SUBTITLE_PREVIEW_FILE));
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            "1\n00:00:01,500 --> 00:00:03,000\nFirst line\nsecond line\n\n"
        );
    }

    #[test]
    fn a_file_mpv_reads_itself_is_handed_over_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let srt = dir.path().join("cues.SRT");
        std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nhi\n").unwrap();

        let output =
            playable_subtitle_file(&srt, PackagedTrack::Subtitle, FPS, &dir.path().join("work"))
                .unwrap();

        assert_eq!(output, srt);
    }

    #[test]
    fn a_caption_track_is_written_beside_the_subtitle_one() {
        let dir = tempfile::tempdir().unwrap();
        let xml = dir.path().join("captions.xml");
        std::fs::write(
            &xml,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:TimeCodeRate>24</dcst:TimeCodeRate>
  <dcst:SubtitleList>
    <dcst:Font>
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:02:00" TimeOut="00:00:04:00">
        <dcst:Text>A caption</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#,
        )
        .unwrap();
        let work = dir.path().join("preview-subtitles");

        let output =
            playable_subtitle_file(&xml, PackagedTrack::ClosedCaption, FPS, &work).unwrap();

        assert_eq!(output, work.join(CAPTION_PREVIEW_FILE));
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            "1\n00:00:02,000 --> 00:00:04,000\nA caption\n\n"
        );
    }

    #[test]
    fn a_format_with_no_cue_reader_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let scc = dir.path().join("captions.scc");
        std::fs::write(&scc, "Scenarist_SCC V1.0\n").unwrap();

        let error =
            playable_subtitle_file(&scc, PackagedTrack::Subtitle, FPS, &dir.path().join("work"))
                .expect_err("no reader");

        assert!(error.contains(".scc"), "got: {error}");
    }
}
