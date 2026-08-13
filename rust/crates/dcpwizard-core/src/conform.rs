//! Timeline conformance — assemble reels from EDL/AAF/XML edit decisions.
//!
//! EDL/xmeml/fcpxml parsing lives in [`postkit::conform`], AAF in
//! [`crate::aaf_import`], and [`parse_timeline`] routes between them. The
//! app-side [`build_reel_plan`] turns a parsed timeline plus a media directory
//! into a concrete, resolved reel/asset plan (the input to DCP reel assembly).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use postkit::conform::{
    ConformError, ConformOptions, EditEvent, Timeline, TimelineFormat, conform,
    detect_timeline_format, find_missing_reels,
};

/// How close to unity a gain multiplier has to be for applying it to be a no-op.
pub(crate) const UNITY_GAIN_EPSILON: f64 = 1e-9;

/// Parse a timeline file. AAF is read through libaaf here, every other format
/// is postkit's.
pub fn parse_timeline(file: &Path) -> Result<Timeline, ConformError> {
    match detect_timeline_format(file) {
        TimelineFormat::Aaf => crate::aaf_import::read_timeline(file),
        _ => postkit::conform::parse_timeline(file),
    }
}

/// Write the parsed timeline next to the reel plan, the artifact downstream
/// tools read. Same file postkit's `conform` writes, from a timeline already
/// parsed rather than parsed a second time.
pub fn write_conform_manifest(timeline: &Timeline, output_dir: &Path) -> std::io::Result<PathBuf> {
    let path = output_dir.join("conform_manifest.json");
    let json = serde_json::to_string_pretty(timeline)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, json)?;
    Ok(path)
}

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
    /// Fixed clip gain as a linear amplitude multiplier, None for unity. Plans
    /// written before conform carried gain have no field at all.
    #[serde(default)]
    pub gain_factor: Option<f64>,
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
                gain_factor: event.gain_factor,
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
    /// Gain of whichever event supplies this reel's audio.
    gain_factor: Option<f64>,
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
                gain_factor: None,
            }
        });
        if is_video {
            g.picture = a.media_path.clone();
            g.source_in = a.source_in;
            g.source_out = a.source_out;
            // the picture's own audio is only used while no A track has claimed
            // the reel, so its gain only counts under the same condition
            if g.audio.is_none() {
                g.gain_factor = a.gain_factor;
            }
        } else if g.audio.is_none() {
            g.audio = Some(a.media_path.clone());
            g.gain_factor = a.gain_factor;
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

/// The ffmpeg filter that applies a clip gain, or None when there is nothing to
/// apply. A factor of zero is a real level: it is what a muted clip means.
fn volume_filter(gain_factor: Option<f64>) -> Option<String> {
    gain_factor
        .filter(|factor| (factor - 1.0).abs() > UNITY_GAIN_EPSILON)
        .map(|factor| format!("volume={factor}"))
}

/// Time-based audio trim (48 kHz s24 WAV). Returns Ok(false) when the source has
/// no audio stream. `[source_in, source_out)` in frames at `fps`.
fn trim_audio(
    src: &Path,
    source_in: u32,
    source_out: u32,
    fps: u32,
    gain_factor: Option<f64>,
    dst: &Path,
) -> Result<bool, String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-y", "-i"]).arg(src);
    if source_out > source_in {
        let start = source_in as f64 / fps as f64;
        let dur = (source_out - source_in) as f64 / fps as f64;
        cmd.args(["-ss", &format!("{start}"), "-t", &format!("{dur}")]);
    }
    if let Some(filter) = volume_filter(gain_factor) {
        cmd.args(["-af", &filter]);
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
/// `signer` is set only when this reel's DCP is the delivered package.
fn reel_to_dcp(
    group: &ReelGroup,
    fps: u32,
    work: &Path,
    dcp_dir: &Path,
    signer: Option<&crate::package_signature::PackageSigner>,
) -> Result<(), String> {
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
    let audio_path = match trim_audio(
        audio_src,
        group.source_in,
        group.source_out,
        fps,
        group.gain_factor,
        &wav,
    ) {
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
        signer: signer.cloned(),
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
/// Signs whichever package it delivers: the per-reel DCP with one reel, the
/// assembled CPL and PKL with more.
pub fn assemble_dcp(
    plan: &ReelPlan,
    output_dir: &Path,
    signer: Option<&crate::package_signature::PackageSigner>,
) -> i32 {
    let groups = group_reels(plan);
    if groups.is_empty() {
        tracing::error!("conform plan has no video reels to assemble");
        return 1;
    }
    // reject a bad signer before the encode, not after every reel is encoded
    if let Some(signer) = signer
        && let Err(e) = signer.check_usable()
    {
        tracing::error!("unusable signer: {e}");
        return 1;
    }
    // with more reels the delivered CPL is the assembled one below, so these
    // per-reel DCPs are intermediates and signing them buys nothing
    let reel_signer = if groups.len() == 1 { signer } else { None };
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
        if let Err(e) = reel_to_dcp(group, fps, &work, &dcp_dir, reel_signer) {
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
            signer: signer.cloned(),
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
    fn manifest_keeps_the_events_and_the_skipped_constructs() {
        let dir = tempfile::tempdir().unwrap();
        let timeline = Timeline {
            title: "Cut".into(),
            events: vec![EditEvent {
                reel_name: "REEL001".into(),
                ..Default::default()
            }],
            skipped: vec!["<title> \"Card\": no source media".into()],
            ..Default::default()
        };
        let path = write_conform_manifest(&timeline, dir.path()).unwrap();
        let written: Timeline =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written.events.len(), 1);
        assert_eq!(written.skipped, timeline.skipped);
    }

    #[test]
    fn aaf_builds_the_same_reel_plan_shape_as_an_edl() {
        let aaf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extern/libaaf/test/aaf/DR_Mono_Clip_Positioning.aaf");
        if !aaf.is_file() {
            return;
        }
        let timeline = parse_timeline(&aaf).unwrap();
        assert_eq!(timeline.format, TimelineFormat::Aaf);

        let media = tempfile::tempdir().unwrap();
        for event in &timeline.events {
            std::fs::write(media.path().join(format!("{}.wav", event.reel_name)), "").unwrap();
        }
        let plan = build_reel_plan(&timeline, media.path()).unwrap();
        assert_eq!(plan.reels.len(), timeline.events.len());
        assert_eq!(plan.reels[0].reel_name, timeline.events[0].reel_name);
        assert_eq!(plan.reels[0].track_type, "A1");
        assert!(plan.reels[0].media_path.is_file());
    }

    #[test]
    fn a_gain_becomes_an_ffmpeg_volume_filter() {
        assert_eq!(volume_filter(None), None);
        assert_eq!(volume_filter(Some(1.0)), None);
        assert_eq!(volume_filter(Some(1.0 + 1e-12)), None);
        assert_eq!(volume_filter(Some(0.5)), Some("volume=0.5".to_string()));
        // a muted clip is a gain of zero, which is a level like any other
        assert_eq!(volume_filter(Some(0.0)), Some("volume=0".to_string()));
    }

    #[test]
    fn a_reel_takes_the_gain_of_whichever_event_gives_it_audio() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("REEL001.mov"), "").unwrap();
        std::fs::write(dir.path().join("REEL001_sound.wav"), "").unwrap();
        let timeline = Timeline {
            events: vec![
                EditEvent {
                    reel_name: "REEL001".into(),
                    track_type: "V".into(),
                    gain_factor: Some(4.0),
                    ..Default::default()
                },
                EditEvent {
                    reel_name: "REEL001".into(),
                    track_type: "A1".into(),
                    gain_factor: Some(0.5),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let plan = build_reel_plan(&timeline, dir.path()).unwrap();
        assert_eq!(plan.reels[0].gain_factor, Some(4.0));
        assert_eq!(plan.reels[1].gain_factor, Some(0.5));

        // the A track supplies the sound, so its gain wins over the picture's
        let groups = group_reels(&plan);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].gain_factor, Some(0.5));
        assert!(groups[0].audio.is_some());
    }

    #[test]
    fn a_picture_only_reel_keeps_the_gain_of_its_own_audio() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("REEL001.mov"), "").unwrap();
        let timeline = Timeline {
            events: vec![EditEvent {
                reel_name: "REEL001".into(),
                track_type: "V".into(),
                gain_factor: Some(0.25),
                ..Default::default()
            }],
            ..Default::default()
        };
        let groups = group_reels(&build_reel_plan(&timeline, dir.path()).unwrap());
        assert!(groups[0].audio.is_none());
        assert_eq!(groups[0].gain_factor, Some(0.25));
    }

    #[test]
    fn a_plan_written_before_conform_carried_gain_still_reads() {
        let plan: ReelPlan = serde_json::from_str(
            r#"{"title":"Cut","frame_rate":24.0,"reels":[{"reel_name":"REEL001",
               "media_path":"/media/REEL001.mov","track_type":"V",
               "source_in":0,"source_out":120}]}"#,
        )
        .unwrap();
        assert_eq!(plan.reels[0].gain_factor, None);
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
