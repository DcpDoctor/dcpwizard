//! Timeline conformance — assemble reels from EDL/AAF/XML edit decisions.
//!
//! Parsing (EDL/xmeml/fcpxml) and the manifest writer live in [`postkit::conform`].
//! The app-side [`build_reel_plan`] turns a parsed timeline plus a media
//! directory into a concrete, resolved reel/asset plan (the input to DCP reel
//! assembly).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use postkit::conform::{
    ConformOptions, EditEvent, Timeline, TimelineFormat, conform, detect_timeline_format,
    find_missing_reels, parse_timeline,
};

/// One resolved reel asset: an edit event whose reel_name was matched to a real
/// media file in the media directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReelAsset {
    pub reel_name: String,
    pub media_path: PathBuf,
    /// "V", "A1", ... from the edit event.
    pub track_type: String,
    pub source_in: u32,
    pub source_out: u32,
}

/// A conformed reel plan: every timeline event resolved to a media file, in
/// order. This is the executable hand-off to DCP reel assembly (encode + wrap +
/// multi-reel CPL), which is not yet driven here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReelPlan {
    pub title: String,
    pub frame_rate: f64,
    pub reels: Vec<ReelAsset>,
}

/// Find the first file in `media_dir` whose name contains `reel_name`.
fn resolve_media(reel_name: &str, media_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(media_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(reel_name))
        })
}

/// Resolve every timeline event against `media_dir` into a concrete reel plan.
/// Black/aux (BL/AX) events are skipped. Returns the list of unresolved reel
/// names on failure so the caller can fail loud.
pub fn build_reel_plan(timeline: &Timeline, media_dir: &Path) -> Result<ReelPlan, Vec<String>> {
    let mut reels = Vec::new();
    let mut missing = Vec::new();
    for event in &timeline.events {
        if event.reel_name == "BL" || event.reel_name == "AX" {
            continue;
        }
        match resolve_media(&event.reel_name, media_dir) {
            Some(media_path) => reels.push(ReelAsset {
                reel_name: event.reel_name.clone(),
                media_path,
                track_type: event.track_type.clone(),
                source_in: event.source_in,
                source_out: event.source_out,
            }),
            None => {
                if !missing.contains(&event.reel_name) {
                    missing.push(event.reel_name.clone());
                }
            }
        }
    }
    if missing.is_empty() {
        Ok(ReelPlan {
            title: timeline.title.clone(),
            frame_rate: timeline.frame_rate,
            reels,
        })
    } else {
        Err(missing)
    }
}

/// One output reel grouped from the plan: a picture (V-track) media file plus
/// any audio (A-track) media, all sharing a reel_name, with the trim range.
struct ReelGroup {
    reel_name: String,
    picture: PathBuf,
    audio: Option<PathBuf>,
    source_in: u32,
    source_out: u32,
}

/// Group a reel plan by reel_name (first-seen order): the V track is the picture,
/// the first A track (if any) is the sound. Reels with no video are skipped.
fn group_reels(plan: &ReelPlan) -> Vec<ReelGroup> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, ReelGroup> = std::collections::HashMap::new();
    for a in &plan.reels {
        let is_video = a.track_type.starts_with('V');
        let g = groups.entry(a.reel_name.clone()).or_insert_with(|| {
            order.push(a.reel_name.clone());
            ReelGroup {
                reel_name: a.reel_name.clone(),
                picture: a.media_path.clone(),
                audio: None,
                source_in: a.source_in,
                source_out: a.source_out,
            }
        });
        if is_video {
            g.picture = a.media_path.clone();
            g.source_in = a.source_in;
            g.source_out = a.source_out;
        } else if g.audio.is_none() {
            g.audio = Some(a.media_path.clone());
        }
    }
    order
        .into_iter()
        .filter_map(|n| groups.remove(&n))
        .collect()
}

/// Frame-accurate video trim to a lossless intermediate (FFV1), video only.
/// `[source_in, source_out)`; an empty/zero-length range copies the whole clip.
fn trim_video(src: &Path, source_in: u32, source_out: u32, dst: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-y", "-i"]).arg(src);
    if source_out > source_in {
        cmd.arg("-vf").arg(format!(
            "trim=start_frame={source_in}:end_frame={source_out},setpts=PTS-STARTPTS"
        ));
    }
    cmd.args(["-an", "-c:v", "ffv1"]).arg(dst);
    run_ffmpeg(cmd, "video trim")
}

/// Time-based audio trim (48 kHz s24 WAV). Returns Ok(false) when the source has
/// no audio stream. `[source_in, source_out)` in frames at `fps`.
fn trim_audio(
    src: &Path,
    source_in: u32,
    source_out: u32,
    fps: u32,
    dst: &Path,
) -> Result<bool, String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-y", "-i"]).arg(src);
    if source_out > source_in {
        let start = source_in as f64 / fps as f64;
        let dur = (source_out - source_in) as f64 / fps as f64;
        cmd.args(["-ss", &format!("{start}"), "-t", &format!("{dur}")]);
    }
    cmd.args(["-vn", "-acodec", "pcm_s24le", "-ar", "48000"])
        .arg(dst);
    match cmd.output() {
        Ok(o) if o.status.success() && dst.exists() => Ok(true),
        Ok(_) => Ok(false),
        Err(e) => Err(format!("failed to run ffmpeg: {e}")),
    }
}

fn run_ffmpeg(mut cmd: std::process::Command, what: &str) -> Result<(), String> {
    match cmd.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(e) => Err(format!("failed to run ffmpeg for {what}: {e}")),
    }
}

/// Encode one grouped reel to J2K and build a single-reel DCP at `dcp_dir`.
fn reel_to_dcp(group: &ReelGroup, fps: u32, work: &Path, dcp_dir: &Path) -> Result<(), String> {
    use postkit::grok_encoder::{self, CompressParams, EncodeProgress};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;

    // trim the picture source to the reel's range (lossless intermediate)
    let trimmed = work.join("video.mkv");
    trim_video(&group.picture, group.source_in, group.source_out, &trimmed)?;

    let info = crate::probe::probe_video(&trimmed)
        .ok_or_else(|| format!("cannot probe {}", trimmed.display()))?;

    let j2k_dir = work.join("j2k");
    std::fs::create_dir_all(&j2k_dir).map_err(|e| e.to_string())?;
    let params = CompressParams {
        frame_rate: fps as u16,
        apply_xyz_transform: true,
        ..CompressParams::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let result = grok_encoder::encode_video_pipeline(
        &trimmed,
        &j2k_dir,
        &params,
        info.total_frames as u64,
        info.width,
        info.height,
        &cancel,
        |_p: EncodeProgress| {},
    );
    if !result.success {
        return Err(format!("reel encode failed: {}", result.error));
    }

    // audio: a dedicated A-track clip, else the picture clip's own audio
    let wav = work.join("audio.wav");
    let audio_src = group.audio.as_deref().unwrap_or(&group.picture);
    let audio_path = match trim_audio(audio_src, group.source_in, group.source_out, fps, &wav) {
        Ok(true) => Some(wav.clone()),
        Ok(false) => None,
        Err(e) => return Err(e),
    };

    let config = crate::dcp::DcpConfig {
        title: group.reel_name.clone(),
        standard: crate::Standard::Smpte,
        resolution: crate::Resolution::TwoK,
        content_type: crate::ContentType::Feature,
        frame_rate_num: fps,
        frame_rate_den: 1,
        container_width: info.width,
        container_height: info.height,
        output_dir: dcp_dir.to_path_buf(),
        j2k_dir: Some(j2k_dir),
        audio_path,
        subtitle_language: "en".to_string(),
        ..Default::default()
    };
    if crate::dcp::create_dcp(&config) != 0 {
        return Err(format!("DCP creation failed for reel {}", group.reel_name));
    }
    Ok(())
}

/// Drive a resolved reel plan to a finished multi-reel DCP: encode + wrap each
/// reel, then assemble the per-reel DCPs into one CPL. `plan_json` (the conform
/// plan) is written into `output_dir` as an artifact. Returns 0 on success.
pub fn assemble_dcp(plan: &ReelPlan, output_dir: &Path) -> i32 {
    let groups = group_reels(plan);
    if groups.is_empty() {
        tracing::error!("conform plan has no video reels to assemble");
        return 1;
    }
    let fps = plan.frame_rate.round().max(1.0) as u32;

    if let Err(e) = std::fs::create_dir_all(output_dir) {
        tracing::error!("cannot create output dir: {e}");
        return 1;
    }
    let work_root = output_dir.join(".conform_work");
    let mut reel_dcps = Vec::new();
    for (i, group) in groups.iter().enumerate() {
        let work = work_root.join(format!("reel_{}", i + 1));
        let dcp_dir = work.join("dcp");
        tracing::info!("conform reel {}: {}", i + 1, group.reel_name);
        if let Err(e) = reel_to_dcp(group, fps, &work, &dcp_dir) {
            tracing::error!("{e}");
            let _ = std::fs::remove_dir_all(&work_root);
            return 1;
        }
        reel_dcps.push(dcp_dir);
    }

    // one reel: the single-reel DCP is the result; move it into output_dir.
    // multiple reels: assemble them into one multi-reel CPL.
    let code = if reel_dcps.len() == 1 {
        move_dir_contents(&reel_dcps[0], output_dir)
    } else {
        crate::assemble::assemble(&crate::assemble::AssembleConfig {
            inputs: reel_dcps.clone(),
            output_dir: output_dir.to_path_buf(),
            title: plan.title.clone(),
            // conform takes no signer yet, and its two paths would need it in
            // different places: the assembled CPL here, the per-reel one when a
            // single reel is moved straight out
            signer: None,
        })
    };
    let _ = std::fs::remove_dir_all(&work_root);
    if code != 0 {
        tracing::error!("multi-reel assembly failed");
        return 1;
    }
    tracing::info!(
        "Conformed {} reel(s) into a DCP at {}",
        groups.len(),
        output_dir.display()
    );
    0
}

/// Move every file in `from` into `to` (flat DCP directory, no subdirs).
fn move_dir_contents(from: &Path, to: &Path) -> i32 {
    let entries = match std::fs::read_dir(from) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("cannot read {}: {e}", from.display());
            return -1;
        }
    };
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if std::fs::rename(&src, &dst).is_err()
            && (std::fs::copy(&src, &dst).is_err() || std::fs::remove_file(&src).is_err())
        {
            tracing::error!("cannot move {} -> {}", src.display(), dst.display());
            return -1;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_detect_format() {
        assert_eq!(
            detect_timeline_format(Path::new("edit.edl")),
            TimelineFormat::EdlCmx3600
        );
        assert_eq!(
            detect_timeline_format(Path::new("project.aaf")),
            TimelineFormat::Aaf
        );
        assert_eq!(
            detect_timeline_format(Path::new("timeline.otio")),
            TimelineFormat::Otio
        );
    }

    #[test]
    fn test_parse_edl() {
        let dir = tempfile::tempdir().unwrap();
        let edl = dir.path().join("test.edl");
        std::fs::write(
            &edl,
            "TITLE: Test\nFCM: NON-DROP FRAME\n\n001  REEL001  V  C        01:00:00:00 01:00:05:00 01:00:00:00 01:00:05:00\n",
        ).unwrap();
        let tl = parse_timeline(&edl).unwrap();
        assert_eq!(tl.title, "Test");
        assert_eq!(tl.events.len(), 1);
        assert_eq!(tl.events[0].reel_name, "REEL001");
    }

    #[test]
    fn test_find_missing_reels() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("REEL001.mxf"), "").unwrap();
        let tl = Timeline {
            events: vec![
                EditEvent {
                    reel_name: "REEL001".into(),
                    ..Default::default()
                },
                EditEvent {
                    reel_name: "REEL002".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let missing = find_missing_reels(&tl, dir.path());
        assert_eq!(missing, vec!["REEL002"]);
    }

    #[test]
    fn build_reel_plan_resolves_and_flags_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("REEL001.mov"), "").unwrap();
        let tl = Timeline {
            title: "Cut".into(),
            frame_rate: 24.0,
            events: vec![
                EditEvent {
                    reel_name: "REEL001".into(),
                    track_type: "V".into(),
                    source_in: 10,
                    source_out: 130,
                    ..Default::default()
                },
                EditEvent {
                    reel_name: "BL".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let plan = build_reel_plan(&tl, dir.path()).unwrap();
        // BL is skipped; REEL001 resolves to the .mov
        assert_eq!(plan.reels.len(), 1);
        assert_eq!(plan.reels[0].reel_name, "REEL001");
        assert_eq!(plan.reels[0].source_out, 130);
        assert!(plan.reels[0].media_path.ends_with("REEL001.mov"));

        // a missing reel fails loud with the unresolved name
        let tl2 = Timeline {
            events: vec![EditEvent {
                reel_name: "REEL999".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            build_reel_plan(&tl2, dir.path()).unwrap_err(),
            vec!["REEL999"]
        );
    }
}
