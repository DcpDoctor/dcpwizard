//! Reel splitting.
//!
//! Splits an encoded J2K frame sequence, its audio, and its subtitles into
//! per-reel ranges and wraps one picture/sound/subtitle MXF per reel, then emits
//! a multi-reel CPL plus a shared PKL/ASSETMAP. postkit's wrapper always consumes
//! every file it is given (no frame-subrange option), so picture is wrapped from a
//! per-reel slice of the sorted frame list, audio is sliced sample-accurately into
//! a temp WAV per reel, and subtitles are re-split and rebased into a per-reel DCST.

use crate::dcp::DcpConfig;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// A half-open frame range `[start, end)` at the picture edit rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReelRange {
    pub start: u64,
    pub end: u64,
}

impl ReelRange {
    pub fn frames(&self) -> u64 {
        self.end - self.start
    }
}

/// Split `total_frames` into reels of at most `reel_length_minutes` each.
///
/// Every reel is at least 1 second (SMPTE ST 429-2): a trailing remainder shorter
/// than one second is merged into the previous reel. `reel_length_minutes == 0`,
/// a zero total, or content that fits in one reel yield a single reel.
pub fn plan_reel_ranges(total_frames: u64, fps: u32, reel_length_minutes: u32) -> Vec<ReelRange> {
    let fps = fps.max(1) as u64;
    let frames_per_reel = reel_length_minutes as u64 * 60 * fps;
    if frames_per_reel == 0 || total_frames <= frames_per_reel {
        return vec![ReelRange {
            start: 0,
            end: total_frames,
        }];
    }

    let mut ranges = Vec::new();
    let mut start = 0u64;
    while start < total_frames {
        let end = (start + frames_per_reel).min(total_frames);
        ranges.push(ReelRange { start, end });
        start = end;
    }

    // merge a sub-1-second trailing reel into its predecessor
    if ranges.len() >= 2 {
        let last = ranges[ranges.len() - 1];
        if last.frames() < fps {
            ranges.pop();
            let n = ranges.len();
            ranges[n - 1].end = last.end;
        }
    }
    ranges
}

/// Parse a timecode into a frame index at `fps`.
///
/// Accepts `HH:MM:SS` or `HH:MM:SS:FF` (frames). The frame field must be below
/// `fps`. Returns the absolute frame number for the timecode.
pub fn parse_timecode(tc: &str, fps: u32) -> Result<u64, String> {
    let fps = fps.max(1) as u64;
    let parts: Vec<&str> = tc.trim().split(':').collect();
    if parts.len() != 3 && parts.len() != 4 {
        return Err(format!("timecode '{tc}' must be HH:MM:SS or HH:MM:SS:FF"));
    }
    let num = |s: &str| -> Result<u64, String> {
        s.parse::<u64>()
            .map_err(|_| format!("invalid number '{s}' in timecode '{tc}'"))
    };
    let h = num(parts[0])?;
    let m = num(parts[1])?;
    let s = num(parts[2])?;
    if m >= 60 || s >= 60 {
        return Err(format!("timecode '{tc}' has minutes/seconds out of range"));
    }
    let f = if parts.len() == 4 { num(parts[3])? } else { 0 };
    if f >= fps {
        return Err(format!("timecode '{tc}' frame field must be < {fps}"));
    }
    Ok(((h * 60 + m) * 60 + s) * fps + f)
}

/// Parse chapter start frames from `ffprobe -show_chapters -print_format json`.
///
/// Each chapter's `start_time` (seconds) becomes a reel boundary at the nearest
/// frame. The frame-0 boundary (a chapter at the very start) is dropped; the rest
/// are sorted and de-duplicated. An empty chapter list is an error: the caller
/// asked to split by chapters but the source has none.
pub fn parse_chapter_starts(json: &str, fps: u32) -> Result<Vec<u64>, String> {
    let fps = fps.max(1) as f64;
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("cannot parse ffprobe chapters: {e}"))?;
    let chapters = v
        .get("chapters")
        .and_then(|c| c.as_array())
        .ok_or("ffprobe returned no chapters array")?;
    let mut frames: Vec<u64> = Vec::new();
    for ch in chapters {
        let start = ch
            .get("start_time")
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or("chapter is missing a numeric start_time")?;
        let frame = (start * fps).round() as u64;
        if frame > 0 {
            frames.push(frame);
        }
    }
    frames.sort_unstable();
    frames.dedup();
    if frames.is_empty() {
        return Err("source has no chapter marks to split on".into());
    }
    Ok(frames)
}

/// Build reel ranges from explicit split boundaries (frame numbers where a new
/// reel starts).
///
/// Boundaries are sorted and de-duplicated. Every resulting reel must be at least
/// one second (`fps` frames, SMPTE ST 429-2); a boundary that would make any reel
/// shorter, or one at/beyond the content, is a loud error.
pub fn plan_reel_ranges_explicit(
    total_frames: u64,
    fps: u32,
    boundaries: &[u64],
) -> Result<Vec<ReelRange>, String> {
    let fps = fps.max(1) as u64;
    let mut bounds: Vec<u64> = boundaries.to_vec();
    bounds.sort_unstable();
    bounds.dedup();
    if let Some(&b) = bounds.iter().find(|&&b| b == 0 || b >= total_frames) {
        return Err(format!(
            "split point at frame {b} is outside the content (1..{})",
            total_frames.saturating_sub(1)
        ));
    }

    let mut ranges = Vec::with_capacity(bounds.len() + 1);
    let mut start = 0u64;
    for &b in &bounds {
        ranges.push(ReelRange { start, end: b });
        start = b;
    }
    ranges.push(ReelRange {
        start,
        end: total_frames,
    });

    for r in &ranges {
        if r.frames() < fps {
            return Err(format!(
                "reel {}..{} is shorter than one second ({} frames < {fps}); adjust the split points",
                r.start,
                r.end,
                r.frames()
            ));
        }
    }
    Ok(ranges)
}

/// PCM WAV layout needed to slice the essence sample-accurately.
pub(crate) struct WavInfo {
    pub(crate) sample_rate: u32,
    pub(crate) block_align: u32,
    /// bytes copied verbatim before the data payload (RIFF + fmt + any pre-data chunks + "data"+size)
    pub(crate) header: Vec<u8>,
    /// offset of the 4-byte data chunk size field within `header`
    pub(crate) data_size_field_pos: usize,
    /// byte offset of the data payload in the source file
    pub(crate) data_offset: u64,
    pub(crate) data_size: u64,
}

pub(crate) fn parse_wav(path: &Path) -> Result<WavInfo, String> {
    let mut f =
        std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut head = vec![0u8; 4096];
    let n = f
        .read(&mut head)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let d = &head[..n];
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return Err(format!("{} is not a RIFF/WAVE file", path.display()));
    }
    let mut block_align = 0u32;
    let mut sample_rate = 0u32;
    let mut pos = 12usize;
    while pos + 8 <= d.len() {
        let id = &d[pos..pos + 4];
        let size = u32::from_le_bytes([d[pos + 4], d[pos + 5], d[pos + 6], d[pos + 7]]) as usize;
        let body = pos + 8;
        if id == b"fmt " && body + 16 <= d.len() {
            sample_rate = u32::from_le_bytes([d[body + 4], d[body + 5], d[body + 6], d[body + 7]]);
            block_align = u16::from_le_bytes([d[body + 12], d[body + 13]]) as u32;
        }
        if id == b"data" {
            return Ok(WavInfo {
                sample_rate,
                block_align,
                header: d[..body].to_vec(),
                data_size_field_pos: pos + 4,
                data_offset: body as u64,
                data_size: size as u64,
            });
        }
        pos = body + size + (size & 1);
    }
    Err(format!("no data chunk found in {}", path.display()))
}

/// Write a reel's WAV as `[start_sample, start_sample + sample_count)` sliced from
/// `src`, padding with silence if the source runs short so the reel's audio is
/// exactly `sample_count` samples (matching the reel's frame count).
pub(crate) fn write_reel_wav(
    src: &Path,
    info: &WavInfo,
    start_sample: u64,
    sample_count: u64,
    out: &Path,
) -> Result<(), String> {
    write_shifted_wav(src, info, 0, start_sample, sample_count, out)
}

/// Write `head_silence` samples of silence followed by `src`'s payload from
/// `start_sample`, for exactly `sample_count` samples in total. A source that
/// runs short is filled out with silence at the tail.
pub(crate) fn write_shifted_wav(
    src: &Path,
    info: &WavInfo,
    head_silence: u64,
    start_sample: u64,
    sample_count: u64,
    out: &Path,
) -> Result<(), String> {
    let ba = info.block_align as u64;
    let want_bytes = sample_count * ba;
    let start_byte = start_sample * ba;
    let head_bytes = (head_silence * ba).min(want_bytes);

    let mut header = info.header.clone();
    let riff_size = (info.header.len() as u64 - 8) + want_bytes;
    header[4..8].copy_from_slice(&(riff_size as u32).to_le_bytes());
    let p = info.data_size_field_pos;
    header[p..p + 4].copy_from_slice(&(want_bytes as u32).to_le_bytes());

    let mut w = std::fs::File::create(out).map_err(|e| format!("cannot create {out:?}: {e}"))?;
    w.write_all(&header).map_err(|e| e.to_string())?;

    let zeros = vec![0u8; 1 << 16];
    let mut lead = head_bytes;
    while lead > 0 {
        let take = lead.min(zeros.len() as u64) as usize;
        w.write_all(&zeros[..take]).map_err(|e| e.to_string())?;
        lead -= take as u64;
    }

    let avail = info
        .data_size
        .saturating_sub(start_byte)
        .min(want_bytes - head_bytes);
    if avail > 0 {
        let mut r = std::fs::File::open(src).map_err(|e| e.to_string())?;
        r.seek(SeekFrom::Start(info.data_offset + start_byte))
            .map_err(|e| e.to_string())?;
        let mut remaining = avail;
        let mut buf = vec![0u8; 1 << 16];
        while remaining > 0 {
            let take = remaining.min(buf.len() as u64) as usize;
            r.read_exact(&mut buf[..take]).map_err(|e| e.to_string())?;
            w.write_all(&buf[..take]).map_err(|e| e.to_string())?;
            remaining -= take as u64;
        }
    }
    // pad the shortfall with silence so the reel is exactly sample_count samples
    let mut pad = want_bytes - head_bytes - avail;
    while pad > 0 {
        let take = pad.min(zeros.len() as u64) as usize;
        w.write_all(&zeros[..take]).map_err(|e| e.to_string())?;
        pad -= take as u64;
    }
    Ok(())
}

/// Sorted J2K codestreams in `dir` (same order postkit's dir wrap would use).
pub(crate) fn collect_frames(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && matches!(
                    p.extension()
                        .and_then(|x| x.to_str())
                        .map(|x| x.to_lowercase())
                        .as_deref(),
                    Some("j2c") | Some("j2k")
                )
        })
        .collect();
    files.sort();
    files
}

/// Remove every codestream already in `dir`. The MXF wrapper takes the whole
/// directory listing, so frames from an earlier longer run would ship as extra
/// frames if a staging directory were reused without this.
pub(crate) fn clear_stale_frames(dir: &Path) -> Result<(), String> {
    for stale in collect_frames(dir) {
        std::fs::remove_file(&stale)
            .map_err(|e| format!("cannot clear stale {}: {e}", stale.display()))?;
    }
    Ok(())
}

/// Build a multi-reel DCP. Called by [`crate::dcp::create_dcp`] only when
/// `reel_length_minutes > 0`; the single-reel path is left untouched.
pub fn create_multi_reel_dcp(config: &DcpConfig, fps: u32) -> i32 {
    let Some(j2k_dir) = config.j2k_dir.as_ref() else {
        tracing::error!("A J2K input directory is required");
        return -1;
    };
    let frames = collect_frames(j2k_dir);
    if frames.is_empty() {
        tracing::error!("J2K input directory contains no codestreams");
        return -1;
    }
    let total_frames = frames.len() as u64;
    let ranges = if !config.reel_split_frames.is_empty() {
        match plan_reel_ranges_explicit(total_frames, fps, &config.reel_split_frames) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("{e}");
                return -1;
            }
        }
    } else {
        plan_reel_ranges(total_frames, fps, config.reel_length_minutes)
    };
    tracing::info!(
        "Splitting {total_frames} frames into {} reel(s) at {fps} fps",
        ranges.len()
    );

    // audio layout (sliced per reel), validated once up front
    let audio = config.audio_path.as_ref().filter(|p| p.exists());
    let wav = match audio {
        Some(path) => match parse_wav(path) {
            Ok(w) => {
                if let Err(e) = crate::pad::check_frame_aligned_sample_rate(w.sample_rate, fps) {
                    tracing::error!("{e}");
                    return -1;
                }
                Some((path, w))
            }
            Err(e) => {
                tracing::error!("{e}");
                return -1;
            }
        },
        None => None,
    };

    // subtitle cues (re-split per reel), parsed once up front. Any supported
    // format; wrap/RTL applied once and the embedded font shared across reels.
    let subtitle_plan = match config.subtitle_path.as_ref().filter(|p| p.exists()) {
        Some(path) => match crate::subtitle::plan_reel_subtitles(
            path,
            config.source_trim,
            fps,
            &config.subtitle_opts,
            &config.output_dir,
        ) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::error!("subtitle parse failed: {e}");
                return -1;
            }
        },
        None => None,
    };
    let subtitle_lang = if config.subtitle_language.is_empty() {
        "en"
    } else {
        &config.subtitle_language
    };

    // closed captions: same timed-text machinery as the open subtitle, but each
    // reel's track is emitted under the MainClosedCaption CPL role.
    let ccap_plan = match config.ccap_path.as_ref().filter(|p| p.exists()) {
        Some(path) => match crate::subtitle::plan_reel_subtitles(
            path,
            config.source_trim,
            fps,
            &crate::subtitle::SubtitleOptions::default(),
            &config.output_dir,
        ) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::error!("closed-caption parse failed: {e}");
                return -1;
            }
        },
        None => None,
    };
    let ccap_lang = if config.ccap_language.is_empty() {
        "en"
    } else {
        &config.ccap_language
    };

    // every reel is cut from the same encode, so one geometry read covers them all
    let geometry = match crate::cpl::picture_geometry(
        j2k_dir,
        config.container_width,
        config.container_height,
    ) {
        Ok(g) => g,
        Err(e) => {
            tracing::error!("{e}");
            return -1;
        }
    };

    let mut cpl_reels = Vec::new();
    let mut pkl_entries = Vec::new();
    let mut am_entries = Vec::new();
    let mut key_infos = Vec::new();
    // temp files to remove after packaging (sliced WAVs / loose DCSTs)
    let mut temps: Vec<PathBuf> = Vec::new();

    for (i, range) in ranges.iter().enumerate() {
        let reel_frames = range.frames();

        // ── picture ───────────────────────────────────────────────────
        let picture_uuid = uuid::Uuid::new_v4();
        let picture_name = format!("picture_{picture_uuid}.mxf");
        let picture_path = config.output_dir.join(&picture_name);
        let picture_key = match mint_key(
            config,
            crate::encrypt::KeyType::Mdik,
            &picture_uuid.to_string(),
        ) {
            Ok(k) => k,
            Err(()) => return -1,
        };
        let picture_key_id = picture_key.as_ref().map(|k| k.info.key_id.clone());
        let slice = frames[range.start as usize..range.end as usize].to_vec();
        if crate::mxf_wrap::wrap_mxf_files(
            slice,
            &picture_path,
            crate::mxf_wrap::MxfType::J2kPicture,
            fps,
            picture_key.as_ref().map(mxf_enc),
            None,
            Some(*picture_uuid.as_bytes()),
        )
        .is_none()
        {
            tracing::error!("Failed to wrap picture MXF for reel {}", i + 1);
            return -1;
        }
        register_asset(
            &mut pkl_entries,
            &mut am_entries,
            &picture_uuid.to_string(),
            &picture_name,
            &picture_path,
        );

        // ── sound ─────────────────────────────────────────────────────
        let mut sound_id = None;
        let mut sound_key_id = None;
        let mut sound_key = None;
        if let Some((src, info)) = &wav {
            let spf = (info.sample_rate / fps) as u64;
            let sound_uuid = uuid::Uuid::new_v4();
            let wav_tmp = config.output_dir.join(format!("sound_{sound_uuid}.wav"));
            if let Err(e) =
                write_reel_wav(src, info, range.start * spf, reel_frames * spf, &wav_tmp)
            {
                tracing::error!("audio split failed for reel {}: {e}", i + 1);
                return -1;
            }
            let sound_name = format!("sound_{sound_uuid}.mxf");
            let sound_path = config.output_dir.join(&sound_name);
            let key = match mint_key(
                config,
                crate::encrypt::KeyType::Mdak,
                &sound_uuid.to_string(),
            ) {
                Ok(k) => k,
                Err(()) => return -1,
            };
            sound_key_id = key.as_ref().map(|k| k.info.key_id.clone());
            let channels = match hound::WavReader::open(&wav_tmp) {
                Ok(reader) => reader.spec().channels as u32,
                Err(e) => {
                    tracing::error!("cannot read {}: {e}", wav_tmp.display());
                    return -1;
                }
            };
            let mca_config = crate::mxf_wrap::build_mca_config(
                channels,
                channels,
                config.hi_channel,
                config.vi_channel,
            )
            .map(|labels| postkit::mxf_wrap::McaConfig {
                labels,
                spoken_language: config.audio_language.clone(),
            });
            let wrapped = crate::mxf_wrap::wrap_mxf_files(
                vec![wav_tmp.clone()],
                &sound_path,
                crate::mxf_wrap::MxfType::PcmAudio,
                fps,
                key.as_ref().map(mxf_enc),
                mca_config,
                Some(*sound_uuid.as_bytes()),
            );
            temps.push(wav_tmp);
            if wrapped.is_none() {
                tracing::error!("Failed to wrap sound MXF for reel {}", i + 1);
                return -1;
            }
            register_asset(
                &mut pkl_entries,
                &mut am_entries,
                &sound_uuid.to_string(),
                &sound_name,
                &sound_path,
            );
            sound_id = Some(sound_uuid.to_string());
            sound_key = key;
        }

        // ── subtitle ──────────────────────────────────────────────────
        let mut sub = None;
        if let Some(plan) = &subtitle_plan {
            let mut rebased =
                crate::subtitle::rebase_styled_for_reel(&plan.cues, range.start, range.end, fps);
            // a reel no cue falls into carries a placeholder spanning it
            if rebased.is_empty() {
                rebased = crate::subtitle::placeholder_styled_cues(fps);
            }
            let subtitle_uuid = uuid::Uuid::new_v4();
            let dcst = config
                .output_dir
                .join(format!("subtitle_{subtitle_uuid}.xml"));
            let (xml, resources) = crate::subtitle::render_reel_dcst(
                &rebased,
                subtitle_lang,
                fps,
                &config.subtitle_opts,
                plan.font.as_ref(),
            );
            if let Err(e) = std::fs::write(&dcst, xml) {
                tracing::error!("subtitle write failed for reel {}: {e}", i + 1);
                return -1;
            }
            let sub_name = format!("subtitle_{subtitle_uuid}.mxf");
            let sub_path = config.output_dir.join(&sub_name);
            let wrapped = crate::mxf_wrap::wrap_timed_text_resources(
                &dcst,
                &resources,
                &sub_path,
                fps,
                Some(*subtitle_uuid.as_bytes()),
                Some(reel_frames as u32),
            );
            temps.push(dcst);
            let Some(track) = wrapped else {
                tracing::error!("Failed to wrap subtitle MXF for reel {}", i + 1);
                return -1;
            };
            register_asset(
                &mut pkl_entries,
                &mut am_entries,
                &subtitle_uuid.to_string(),
                &sub_name,
                &sub_path,
            );
            sub = Some(crate::versions::WrappedTimedText {
                id: subtitle_uuid.to_string(),
                duration: track.duration,
                hash: crate::hash::hash_file(&sub_path).unwrap_or_default(),
            });
        }

        // ── closed caption ────────────────────────────────────────────
        let mut ccap = None;
        if let Some(plan) = &ccap_plan {
            let mut rebased =
                crate::subtitle::rebase_styled_for_reel(&plan.cues, range.start, range.end, fps);
            // a reel no cue falls into carries a placeholder spanning it
            if rebased.is_empty() {
                rebased = crate::subtitle::placeholder_styled_cues(fps);
            }
            let ccap_uuid = uuid::Uuid::new_v4();
            let dcst = config.output_dir.join(format!("ccap_{ccap_uuid}.xml"));
            let (xml, resources) = crate::subtitle::render_reel_dcst(
                &rebased,
                ccap_lang,
                fps,
                &crate::subtitle::SubtitleOptions::default(),
                plan.font.as_ref(),
            );
            if let Err(e) = std::fs::write(&dcst, xml) {
                tracing::error!("closed-caption write failed for reel {}: {e}", i + 1);
                return -1;
            }
            let ccap_name = format!("ccap_{ccap_uuid}.mxf");
            let ccap_path = config.output_dir.join(&ccap_name);
            let wrapped = crate::mxf_wrap::wrap_timed_text_resources(
                &dcst,
                &resources,
                &ccap_path,
                fps,
                Some(*ccap_uuid.as_bytes()),
                Some(reel_frames as u32),
            );
            temps.push(dcst);
            let Some(track) = wrapped else {
                tracing::error!("Failed to wrap closed-caption MXF for reel {}", i + 1);
                return -1;
            };
            register_asset(
                &mut pkl_entries,
                &mut am_entries,
                &ccap_uuid.to_string(),
                &ccap_name,
                &ccap_path,
            );
            ccap = Some(crate::versions::WrappedTimedText {
                id: ccap_uuid.to_string(),
                duration: track.duration,
                hash: crate::hash::hash_file(&ccap_path).unwrap_or_default(),
            });
        }

        if let Some(k) = picture_key {
            key_infos.push(k.info);
        }
        if let Some(k) = sound_key {
            key_infos.push(k.info);
        }

        cpl_reels.push(crate::cpl::CplReel {
            reel_id: uuid::Uuid::new_v4().to_string(),
            picture_id: picture_uuid.to_string(),
            picture_width: geometry.stored_width,
            picture_height: geometry.stored_height,
            picture_active_width: geometry.active_width,
            picture_active_height: geometry.active_height,
            picture_edit_rate_num: fps,
            picture_edit_rate_den: 1,
            picture_duration: reel_frames,
            picture_entry_point: 0,
            picture_key_id,
            sound_id,
            sound_edit_rate_num: fps,
            sound_edit_rate_den: 1,
            sound_duration: reel_frames,
            sound_entry_point: 0,
            sound_key_id,
            subtitle_id: sub.as_ref().map(|t| t.id.clone()),
            subtitle_edit_rate_num: fps,
            subtitle_edit_rate_den: 1,
            subtitle_duration: sub.as_ref().map(|t| t.duration).unwrap_or(0),
            subtitle_entry_point: 0,
            subtitle_language: sub.as_ref().map(|_| subtitle_lang.to_string()),
            subtitle_hash: sub.as_ref().map(|t| t.hash.clone()),
            ccap_id: ccap.as_ref().map(|t| t.id.clone()),
            ccap_edit_rate_num: fps,
            ccap_edit_rate_den: 1,
            ccap_duration: ccap.as_ref().map(|t| t.duration).unwrap_or(0),
            ccap_entry_point: 0,
            ccap_language: ccap.as_ref().map(|_| ccap_lang.to_string()),
            ccap_hash: ccap.as_ref().map(|t| t.hash.clone()),
            stereoscopic: false,
            aux_data: None,
            markers: Vec::new(),
            ..Default::default()
        });
    }

    // explicit --marker specs are refused for a split composition, so the reels
    // carry the default pair
    crate::cpl::apply_default_markers(&mut cpl_reels);

    // the shared embedded font is now inside each reel's MXF
    if let Some(plan) = &subtitle_plan
        && let Some((font_file, _)) = &plan.font
    {
        let _ = std::fs::remove_file(font_file);
    }
    if let Some(plan) = &ccap_plan
        && let Some((font_file, _)) = &plan.font
    {
        let _ = std::fs::remove_file(font_file);
    }

    // sound layout for the SMPTE CompositionMetadataAsset, from the packaged audio
    let main_sound = wav.as_ref().and_then(|(path, info)| {
        let ch = crate::mxf_wrap::wav_channels(path).ok()? as u32;
        let configuration =
            crate::cpl::main_sound_configuration(ch, ch, config.hi_channel, config.vi_channel)?;
        Some(crate::cpl::MainSound {
            configuration,
            sample_rate: info.sample_rate,
        })
    });

    let code = write_packaging(
        config,
        cpl_reels,
        pkl_entries,
        am_entries,
        key_infos,
        main_sound,
    );
    for t in temps {
        let _ = std::fs::remove_file(t);
    }
    code
}

/// Cues whose start falls in `[range.start, range.end)`, rebased to a reel-local
/// timeline (frame 0 = reel start) and clamped to the reel's end. A cue starting
/// in this reel that overruns into the next is truncated at the reel boundary.
pub(crate) fn rebase_cues_for_reel(
    cues: &[crate::subtitle::SubCue],
    range: ReelRange,
) -> Vec<crate::subtitle::SubCue> {
    cues.iter()
        .filter(|c| c.start_frame >= range.start && c.start_frame < range.end)
        .filter_map(|c| {
            let start = c.start_frame - range.start;
            let end = c.end_frame.min(range.end) - range.start;
            (end > start).then(|| crate::subtitle::SubCue {
                start_frame: start,
                end_frame: end,
                text: c.text.clone(),
            })
        })
        .collect()
}

pub(crate) fn mxf_enc(k: &crate::encrypt::GeneratedKey) -> postkit::mxf_wrap::MxfEncryption {
    postkit::mxf_wrap::MxfEncryption {
        content_key: k.key,
        key_id: k.key_id,
    }
}

fn mint_key(
    config: &DcpConfig,
    kind: crate::encrypt::KeyType,
    uuid: &str,
) -> Result<Option<crate::encrypt::GeneratedKey>, ()> {
    if !config.encrypt {
        return Ok(None);
    }
    match crate::encrypt::generate_content_key(kind, uuid) {
        Ok(k) => Ok(Some(k)),
        Err(e) => {
            tracing::error!("content key generation failed: {e}");
            Err(())
        }
    }
}

pub(crate) fn register_asset(
    pkl: &mut Vec<crate::pkl::PklEntry>,
    am: &mut Vec<crate::assetmap::AssetMapEntry>,
    id: &str,
    name: &str,
    path: &Path,
) {
    pkl.push(crate::pkl::PklEntry {
        id: id.to_string(),
        asset_type: "application/mxf".into(),
        file: path.to_path_buf(),
        hash: crate::hash::hash_file(path).unwrap_or_default(),
        size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
    });
    am.push(crate::assetmap::AssetMapEntry {
        id: id.to_string(),
        path: name.to_string(),
        packing_list: false,
    });
}

fn write_packaging(
    config: &DcpConfig,
    cpl_reels: Vec<crate::cpl::CplReel>,
    mut pkl_entries: Vec<crate::pkl::PklEntry>,
    mut am_entries: Vec<crate::assetmap::AssetMapEntry>,
    key_infos: Vec<crate::encrypt::ContentKey>,
    main_sound: Option<crate::cpl::MainSound>,
) -> i32 {
    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_uuid = uuid::Uuid::new_v4().to_string();

    let cpl_path = config.output_dir.join(format!("CPL_{cpl_uuid}.xml"));
    let cpl_config = crate::cpl::CplConfig {
        title: config.title.clone(),
        content_kind: config.content_type.as_cpl_kind().into(),
        ratings: config.ratings.clone(),
        content_version_label: config.content_versions.first().cloned(),
        reels: cpl_reels,
        standard: config.standard,
        main_sound,
        ..Default::default()
    };
    if crate::cpl::generate_cpl(&cpl_config, &cpl_uuid, &cpl_path) != 0 {
        tracing::error!("Failed to generate CPL");
        return -1;
    }

    pkl_entries.insert(
        0,
        crate::pkl::PklEntry {
            id: cpl_uuid.clone(),
            asset_type: "text/xml".into(),
            file: cpl_path.clone(),
            hash: crate::hash::hash_file(&cpl_path).unwrap_or_default(),
            size: std::fs::metadata(&cpl_path).map(|m| m.len()).unwrap_or(0),
        },
    );
    let pkl_path = config.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    if crate::pkl::generate_pkl(&pkl_entries, &pkl_uuid, config.standard, None, &pkl_path) != 0 {
        tracing::error!("Failed to generate PKL");
        return -1;
    }

    am_entries.insert(
        0,
        crate::assetmap::AssetMapEntry {
            id: cpl_uuid.clone(),
            path: cpl_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            packing_list: false,
        },
    );
    am_entries.insert(
        0,
        crate::assetmap::AssetMapEntry {
            id: pkl_uuid,
            path: pkl_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            packing_list: true,
        },
    );
    if crate::assetmap::generate_assetmap(&am_entries, &config.output_dir, config.standard, None)
        != 0
    {
        tracing::error!("Failed to generate ASSETMAP");
        return -1;
    }

    if config.encrypt {
        let Some(keys_path) = config.key_out.as_ref() else {
            tracing::error!("--key-out is required when encrypting");
            return -1;
        };
        if let Some(parent) = keys_path.parent().filter(|p| !p.as_os_str().is_empty())
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::error!("Failed to create key-out directory: {e}");
            return -1;
        }
        let bundle = crate::encrypt::KeyBundle {
            cpl_id: cpl_uuid.clone(),
            keys: key_infos,
        };
        if let Err(e) = bundle.write(keys_path) {
            tracing::error!("Failed to write keys file: {e}");
            return -1;
        }
        tracing::warn!(
            "Wrote content keys to {}: plaintext AES keys; keep secret, do not ship in the DCP.",
            keys_path.display()
        );
    }

    tracing::info!("Multi-reel DCP created: {}", config.output_dir.display());
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_reel_when_flag_absent() {
        let r = plan_reel_ranges(48000, 24, 0);
        assert_eq!(
            r,
            vec![ReelRange {
                start: 0,
                end: 48000
            }]
        );
    }

    #[test]
    fn single_reel_when_content_fits() {
        // 20 min at 24fps = 28800 frames; content is shorter -> one reel
        let r = plan_reel_ranges(10000, 24, 20);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].end, 10000);
    }

    #[test]
    fn splits_on_minute_boundaries() {
        // 48000 frames at 24fps = 2000s; 20-min reels = 28800 frames each
        let r = plan_reel_ranges(48000, 24, 20);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0],
            ReelRange {
                start: 0,
                end: 28800
            }
        );
        assert_eq!(
            r[1],
            ReelRange {
                start: 28800,
                end: 48000
            }
        );
        // ranges are contiguous and cover everything
        assert_eq!(r[0].end, r[1].start);
        assert_eq!(r[1].end, 48000);
    }

    #[test]
    fn merges_sub_one_second_trailing_reel() {
        // 28810 frames at 24fps, 20-min reels: second reel would be 10 frames
        // (<1s) so it merges back into the first.
        let r = plan_reel_ranges(28810, 24, 20);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].end, 28810);
    }

    #[test]
    fn every_reel_at_least_one_second() {
        for total in [24_000u64, 28_900, 57_601, 100_000] {
            let r = plan_reel_ranges(total, 24, 20);
            for reel in &r {
                assert!(reel.frames() >= 24, "reel {reel:?} shorter than 1s");
            }
        }
    }

    #[test]
    fn samples_per_frame_is_exact_for_dcp_rates() {
        // every legal DCP rate divides 48 kHz (and 96 kHz) evenly, so audio
        // splits land on exact sample boundaries at frame edges.
        for fps in [24u32, 25, 30, 48, 50, 60, 96, 100, 120] {
            assert_eq!(48_000 % fps, 0, "{fps} not sample-exact at 48kHz");
            assert_eq!(96_000 % fps, 0, "{fps} not sample-exact at 96kHz");
        }
    }

    #[test]
    fn timecode_parses_to_frames() {
        assert_eq!(parse_timecode("00:00:00", 24).unwrap(), 0);
        assert_eq!(parse_timecode("00:00:01", 24).unwrap(), 24);
        assert_eq!(parse_timecode("00:01:00", 24).unwrap(), 1440);
        assert_eq!(parse_timecode("01:00:00:00", 24).unwrap(), 86400);
        assert_eq!(parse_timecode("00:00:02:12", 24).unwrap(), 60);
    }

    #[test]
    fn timecode_rejects_bad_input() {
        assert!(parse_timecode("1:2", 24).is_err());
        assert!(parse_timecode("00:60:00", 24).is_err());
        assert!(parse_timecode("00:00:00:24", 24).is_err()); // frame >= fps
        assert!(parse_timecode("aa:bb:cc", 24).is_err());
    }

    #[test]
    fn explicit_split_builds_contiguous_ranges() {
        // 2000 frames at 24fps, split at frame 1000
        let r = plan_reel_ranges_explicit(2000, 24, &[1000]).unwrap();
        assert_eq!(
            r,
            vec![
                ReelRange {
                    start: 0,
                    end: 1000
                },
                ReelRange {
                    start: 1000,
                    end: 2000
                },
            ]
        );
    }

    #[test]
    fn explicit_split_sorts_and_dedups() {
        let r = plan_reel_ranges_explicit(3000, 24, &[2000, 1000, 1000]).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].end, 1000);
        assert_eq!(r[1].end, 2000);
        assert_eq!(r[2].end, 3000);
    }

    #[test]
    fn explicit_split_rejects_sub_one_second_reel() {
        // split at frame 10 with 24fps makes a 10-frame first reel: reject loud
        assert!(plan_reel_ranges_explicit(2000, 24, &[10]).is_err());
        // split near the end makes a short last reel
        assert!(plan_reel_ranges_explicit(2000, 24, &[1990]).is_err());
    }

    #[test]
    fn chapter_starts_parse_from_ffprobe_json() {
        let json = r#"{"chapters":[
            {"id":0,"start_time":"0.000000","end_time":"60.0"},
            {"id":1,"start_time":"60.000000","end_time":"120.0"},
            {"id":2,"start_time":"120.000000","end_time":"180.0"}
        ]}"#;
        // 60s and 120s at 24fps -> frames 1440, 2880; the frame-0 chapter is dropped
        assert_eq!(parse_chapter_starts(json, 24).unwrap(), vec![1440, 2880]);
    }

    #[test]
    fn chapter_starts_error_when_none() {
        let json = r#"{"chapters":[]}"#;
        assert!(parse_chapter_starts(json, 24).is_err());
        // a single chapter at frame 0 also yields no usable boundary
        let one = r#"{"chapters":[{"id":0,"start_time":"0.0","end_time":"10.0"}]}"#;
        assert!(parse_chapter_starts(one, 24).is_err());
    }

    #[test]
    fn explicit_split_rejects_out_of_range_boundary() {
        assert!(plan_reel_ranges_explicit(2000, 24, &[0]).is_err());
        assert!(plan_reel_ranges_explicit(2000, 24, &[2000]).is_err());
        assert!(plan_reel_ranges_explicit(2000, 24, &[5000]).is_err());
    }

    #[test]
    fn subtitle_cues_assigned_and_rebased_per_reel() {
        use crate::subtitle::SubCue;
        let cue = |s, e| SubCue {
            start_frame: s,
            end_frame: e,
            text: "x".into(),
        };
        // reels: [0,1440) and [1440,2000)
        let cues = vec![
            cue(120, 192),   // reel 1, 5s..8s
            cue(1560, 1632), // reel 2, global 65s..68s
            cue(1430, 1500), // starts in reel 1, overruns into reel 2 -> clamped
        ];
        let r1 = rebase_cues_for_reel(
            &cues,
            ReelRange {
                start: 0,
                end: 1440,
            },
        );
        assert_eq!(r1.len(), 2);
        assert_eq!((r1[0].start_frame, r1[0].end_frame), (120, 192));
        // the boundary-crossing cue is truncated at the reel end
        assert_eq!((r1[1].start_frame, r1[1].end_frame), (1430, 1440));

        let r2 = rebase_cues_for_reel(
            &cues,
            ReelRange {
                start: 1440,
                end: 2000,
            },
        );
        assert_eq!(r2.len(), 1);
        // global 1560 rebased to reel-local 120 (5s at 24fps)
        assert_eq!((r2[0].start_frame, r2[0].end_frame), (120, 192));
    }

    #[test]
    fn audio_split_is_sample_accurate_and_padded() {
        // build a 2-channel 16-bit 48k WAV with a known ramp, split at a frame
        // boundary, and check byte offsets line up.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.wav");
        let sample_rate = 48_000u32;
        let channels = 2u16;
        let bits = 16u16;
        let block_align = (bits / 8) * channels; // 4 bytes/sample
        let n_samples = 4000u64; // < 1s, arbitrary
        let mut data = Vec::new();
        for s in 0..n_samples {
            for _ch in 0..channels {
                data.extend_from_slice(&((s & 0xffff) as u16).to_le_bytes());
            }
        }
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&channels.to_le_bytes());
        w.extend_from_slice(&sample_rate.to_le_bytes());
        w.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
        w.extend_from_slice(&block_align.to_le_bytes());
        w.extend_from_slice(&bits.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        std::fs::write(&src, &w).unwrap();

        let info = parse_wav(&src).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(info.block_align, 4);

        // reel 1: samples [0, 2000); reel 2: [2000, 4000) plus 1000 padded silence
        let out1 = dir.path().join("r1.wav");
        write_reel_wav(&src, &info, 0, 2000, &out1).unwrap();
        let out2 = dir.path().join("r2.wav");
        write_reel_wav(&src, &info, 2000, 3000, &out2).unwrap();

        let i1 = parse_wav(&out1).unwrap();
        assert_eq!(i1.data_size, 2000 * 4);
        let i2 = parse_wav(&out2).unwrap();
        assert_eq!(i2.data_size, 3000 * 4);

        // reel 2's first sample equals source sample 2000
        let mut f = std::fs::File::open(&out2).unwrap();
        f.seek(SeekFrom::Start(i2.data_offset)).unwrap();
        let mut first = [0u8; 2];
        f.read_exact(&mut first).unwrap();
        assert_eq!(u16::from_le_bytes(first), 2000u16);

        // the padded tail (source ran out at 4000) is silence
        f.seek(SeekFrom::Start(i2.data_offset + i2.data_size - 2))
            .unwrap();
        let mut last = [0u8; 2];
        f.read_exact(&mut last).unwrap();
        assert_eq!(u16::from_le_bytes(last), 0);
    }
}
