//! Conform a library item to the job it is joined onto and wrap it as its own
//! reel.
//!
//! An item is arbitrary media, so nothing about it matches the feature until
//! this runs: it is fitted onto the feature's coded raster, decoded at the
//! feature's edit rate, encoded the way the feature was, and its sound is
//! placed into the same number of lanes at the same sample rate, filled with
//! silence where the item carries nothing. The feature's own essence and reels
//! are never touched; head items become reels before them and tail items after.

use crate::dcp::DcpConfig;
use crate::library::{AttachedItem, item_frames};
use std::path::Path;

/// Items are ordinary Rec.709 deliverables, so the compressor runs its own DCI
/// transform over them, as it does for a `--source-colourspace rec709` feature.
const ITEM_COLOUR_ROUTE: crate::encode::XyzRoute = crate::encode::XyzRoute::CompressorTransform;

/// The sound every reel of the composition has to agree on, read off the
/// feature's packaged track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobSound {
    pub channels: u32,
    pub sample_rate: u32,
    pub bits: u16,
}

impl JobSound {
    /// The PCM encoder that writes samples of this width.
    fn ffmpeg_codec(&self) -> Result<&'static str, String> {
        match self.bits {
            16 => Ok("pcm_s16le"),
            24 => Ok("pcm_s24le"),
            other => Err(format!(
                "the feature's sound is {other}-bit, which a library item cannot be conformed to: \
                 package the sound as 16- or 24-bit PCM"
            )),
        }
    }
}

/// What the item reels have to match, read off the feature once.
#[derive(Debug, Clone)]
pub struct JobFormat {
    pub fps: u32,
    pub geometry: crate::cpl::PictureGeometry,
    /// None when the composition carries no sound at all, and then neither do
    /// the item reels.
    pub sound: Option<JobSound>,
}

/// One conformed item's reel and everything the package has to record about it.
#[derive(Debug, Default)]
pub struct ItemReels {
    pub reels: Vec<crate::cpl::CplReel>,
    pub pkl: Vec<crate::pkl::PklEntry>,
    pub assetmap: Vec<crate::assetmap::AssetMapEntry>,
    pub keys: Vec<crate::encrypt::ContentKey>,
}

impl ItemReels {
    fn absorb(&mut self, other: ItemReels) {
        self.reels.extend(other.reels);
        self.pkl.extend(other.pkl);
        self.assetmap.extend(other.assetmap);
        self.keys.extend(other.keys);
    }
}

/// Conform and wrap every item in `items`, in the order given. The MXFs land in
/// `config.output_dir` beside the feature's; everything else is cleaned up.
pub fn build_item_reels(
    config: &DcpConfig,
    items: &[AttachedItem],
    format: &JobFormat,
) -> Result<ItemReels, String> {
    let mut built = ItemReels::default();
    if items.is_empty() {
        return Ok(built);
    }
    let work_root = config
        .output_dir
        .join(format!(".dcpwizard_items_{}", uuid::Uuid::new_v4()));
    let result = (|| {
        for item in items {
            let work = work_root.join(&item.item.name);
            std::fs::create_dir_all(&work)
                .map_err(|e| format!("cannot create {}: {e}", work.display()))?;
            built.absorb(build_one_item_reel(config, item, format, &work)?);
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work_root);
    result.map(|()| built)
}

fn build_one_item_reel(
    config: &DcpConfig,
    item: &AttachedItem,
    format: &JobFormat,
    work: &Path,
) -> Result<ItemReels, String> {
    let name = &item.item.name;
    tracing::info!(
        "Conforming library item '{name}' ({}) to {}x{} at {} fps",
        item.item.kind,
        format.geometry.stored_width,
        format.geometry.stored_height,
        format.fps
    );

    let j2k_dir = work.join("j2k");
    let frames = encode_item_picture(item, format, config.max_bitrate_mbps, &j2k_dir)?;

    let picture_uuid = uuid::Uuid::new_v4();
    let picture_name = format!("item_picture_{picture_uuid}.mxf");
    let picture_path = config.output_dir.join(&picture_name);
    let picture_key = mint_item_key(config, crate::encrypt::KeyType::Mdik, &picture_uuid)?;
    if crate::mxf_wrap::wrap_mxf_files(
        crate::reel::collect_frames(&j2k_dir),
        &picture_path,
        crate::mxf_wrap::MxfType::J2kPicture,
        format.fps,
        picture_key.as_ref().map(crate::reel::mxf_enc),
        None,
        Some(*picture_uuid.as_bytes()),
    )
    .is_none()
    {
        return Err(format!(
            "cannot wrap the picture MXF for library item '{name}'"
        ));
    }

    let mut built = ItemReels::default();
    crate::reel::register_asset(
        &mut built.pkl,
        &mut built.assetmap,
        &picture_uuid.to_string(),
        &picture_name,
        &picture_path,
    );

    let mut sound_id = None;
    let mut sound_key_id = None;
    let mut sound_hash = None;
    // a reel cannot carry sound the rest of the composition has no track for
    if format.sound.is_none() && item.item.has_audio {
        tracing::warn!(
            "library item '{name}' carries sound, and this composition has no sound track for it \
             to sit in: its reel is picture only"
        );
    }
    if let Some(sound) = format.sound {
        let wav = work.join("sound.wav");
        build_item_sound(item, &sound, format.fps, frames, work, &wav)?;
        let sound_uuid = uuid::Uuid::new_v4();
        let sound_name = format!("item_sound_{sound_uuid}.mxf");
        let sound_path = config.output_dir.join(&sound_name);
        let key = mint_item_key(config, crate::encrypt::KeyType::Mdak, &sound_uuid)?;
        let mca_config =
            crate::mxf_wrap::build_mca_config(sound.channels, sound.channels, None, None).map(
                |labels| postkit::mxf_wrap::McaConfig {
                    labels,
                    spoken_language: config.audio_language.clone(),
                },
            );
        if crate::mxf_wrap::wrap_mxf_files(
            vec![wav],
            &sound_path,
            crate::mxf_wrap::MxfType::PcmAudio,
            format.fps,
            key.as_ref().map(crate::reel::mxf_enc),
            mca_config,
            Some(*sound_uuid.as_bytes()),
        )
        .is_none()
        {
            return Err(format!(
                "cannot wrap the sound MXF for library item '{name}'"
            ));
        }
        crate::reel::register_asset(
            &mut built.pkl,
            &mut built.assetmap,
            &sound_uuid.to_string(),
            &sound_name,
            &sound_path,
        );
        sound_hash = crate::hash::hash_file(&sound_path).ok();
        sound_id = Some(sound_uuid.to_string());
        sound_key_id = key.as_ref().map(|k| k.info.key_id.clone());
        if let Some(key) = key {
            built.keys.push(key.info);
        }
    }

    built.reels.push(crate::cpl::CplReel {
        reel_id: uuid::Uuid::new_v4().to_string(),
        picture_id: picture_uuid.to_string(),
        picture_hash: crate::hash::hash_file(&picture_path).ok(),
        picture_width: format.geometry.stored_width,
        picture_height: format.geometry.stored_height,
        picture_active_width: format.geometry.active_width,
        picture_active_height: format.geometry.active_height,
        picture_edit_rate_num: format.fps,
        picture_edit_rate_den: 1,
        picture_duration: frames,
        picture_entry_point: 0,
        picture_key_id: picture_key.as_ref().map(|k| k.info.key_id.clone()),
        sound_id,
        sound_edit_rate_num: format.fps,
        sound_edit_rate_den: 1,
        sound_duration: frames,
        sound_entry_point: 0,
        sound_key_id,
        sound_hash,
        ..Default::default()
    });
    if let Some(key) = picture_key {
        built.keys.push(key.info);
    }
    tracing::info!("Library item '{name}': {frames} frame(s)");
    Ok(built)
}

fn mint_item_key(
    config: &DcpConfig,
    kind: crate::encrypt::KeyType,
    asset: &uuid::Uuid,
) -> Result<Option<crate::encrypt::GeneratedKey>, String> {
    if !config.encrypt {
        return Ok(None);
    }
    crate::encrypt::generate_content_key(kind, &asset.to_string())
        .map(Some)
        .map_err(|e| format!("content key generation failed: {e}"))
}

/// Fit the item onto the job's coded raster and encode it to J2K. Returns the
/// frame count the essence actually holds, which is what the reel declares.
fn encode_item_picture(
    item: &AttachedItem,
    format: &JobFormat,
    bitrate_mbps: u32,
    out_dir: &Path,
) -> Result<u64, String> {
    let (width, height) = (format.geometry.stored_width, format.geometry.stored_height);
    let frames = item_frames(&item.item, format.fps);
    let geometry = crate::source_picture::EncodeGeometry {
        forced_raster: Some((width, height)),
        container: Some((format.geometry.active_width, format.geometry.active_height)),
    };
    let resolved = crate::source_picture::resolve_picture(
        &crate::source_picture::SourcePictureOptions::default(),
        &item.media,
        item.item.width,
        item.item.height,
        &geometry,
        false,
    )?;
    if (resolved.encode_width, resolved.encode_height) != (width, height) {
        return Err(format!(
            "library item '{}' fits onto {}x{}, not the job's {width}x{height}",
            item.item.name, resolved.encode_width, resolved.encode_height
        ));
    }
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;

    let rate = postkit::encode::FrameRate::whole(format.fps);
    if postkit::still::is_still_image(&item.media) {
        postkit::still::build_still_frames(&postkit::still::StillHold {
            image: &item.media,
            frames,
            fps: rate,
            width,
            height,
            filters: &resolved.plan.filters,
            apply_xyz_transform: ITEM_COLOUR_ROUTE.compressor_transform(),
            rsiz: postkit::encode::default_rsiz(),
            colour_transform: ITEM_COLOUR_ROUTE.frame_transform()?,
            burn: None,
            out_dir,
        })?;
    } else {
        use postkit::grok_encoder::{self, CompressParams, EncodeProgress};
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        // the item runs at its own rate, so the decode is resampled onto the
        // job's; the plan says where in the chain that has to happen
        let mut filters = resolved.plan.filters.clone();
        filters.insert(
            resolved.plan.fps_position,
            format!("fps={}", rate.ffmpeg_filter_value()),
        );
        let params = CompressParams {
            compression_ratio: crate::encode::DEFAULT_COMPRESSION_RATIO,
            target_codestream_bytes: (bitrate_mbps > 0)
                .then(|| crate::encode::video_codestream_byte_cap(format.fps, bitrate_mbps, false)),
            edit_rate: rate,
            apply_xyz_transform: ITEM_COLOUR_ROUTE.compressor_transform(),
            ..CompressParams::default()
        };
        grok_encoder::initialize(0);
        let cancel = Arc::new(AtomicBool::new(false));
        let result = grok_encoder::encode_video_pipeline_resumable(
            &item.media,
            out_dir,
            &params,
            frames,
            width,
            height,
            &cancel,
            false,
            Some(&filters.join(",")),
            None,
            |_progress: EncodeProgress| {},
        );
        if !result.success {
            return Err(format!(
                "library item '{}' failed to encode: {}",
                item.item.name, result.error
            ));
        }
        for finding in result.picture_findings.describe(rate.as_f64()) {
            tracing::warn!("library item '{}': {finding}", item.item.name);
        }
    }

    let encoded = crate::reel::collect_frames(out_dir).len() as u64;
    if encoded == 0 {
        return Err(format!(
            "library item '{}' encoded no frames",
            item.item.name
        ));
    }
    Ok(encoded)
}

/// The item's sound, placed into the job's lanes and cut to exactly the reel's
/// length. An item with no audio, or one shorter than its picture, is filled
/// with silence.
fn build_item_sound(
    item: &AttachedItem,
    sound: &JobSound,
    fps: u32,
    frames: u64,
    work: &Path,
    out: &Path,
) -> Result<(), String> {
    crate::pad::check_frame_aligned_sample_rate(sound.sample_rate, fps)?;
    let samples = frames * (sound.sample_rate / fps.max(1)) as u64;
    let codec = sound.ffmpeg_codec()?;
    let raw = work.join("sound_lanes.wav");

    // one pan lands the item's channels on the job's lanes and leaves the rest
    // silent, whatever either count is
    let source_channels = item
        .item
        .has_audio
        .then(|| source_audio_channels(&item.media))
        .transpose()?
        .unwrap_or(0);
    let carried = source_channels.min(sound.channels);
    let mut lanes = vec![format!("pan={}c", sound.channels)];
    lanes.extend((0..carried).map(|c| format!("c{c}=c{c}")));

    let mut command = std::process::Command::new("ffmpeg");
    command.arg("-y");
    if carried == 0 {
        let seconds = frames as f64 / fps.max(1) as f64;
        command
            .args(["-f", "lavfi", "-i"])
            .arg(format!("anullsrc=r={}:cl=mono", sound.sample_rate))
            .args(["-t", &format!("{seconds}")]);
    } else {
        command.arg("-i").arg(&item.media);
    }
    command
        .args(["-vn", "-af", &lanes.join("|"), "-ar"])
        .arg(sound.sample_rate.to_string())
        .args(["-c:a", codec])
        .arg(&raw);
    match command.output() {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            return Err(format!(
                "cannot read the sound of library item '{}': {}",
                item.item.name,
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .next_back()
                    .unwrap_or("ffmpeg failed")
            ));
        }
        Err(e) => return Err(format!("cannot run ffmpeg for library item sound: {e}")),
    }

    let info = crate::reel::parse_wav(&raw)?;
    let expected_align = sound.channels * u32::from(sound.bits) / 8;
    if info.block_align != expected_align || info.sample_rate != sound.sample_rate {
        return Err(format!(
            "library item '{}' conformed to {} Hz / {} bytes a sample, not the job's {} Hz / \
             {expected_align}",
            item.item.name, info.sample_rate, info.block_align, sound.sample_rate
        ));
    }
    crate::reel::write_reel_wav(&raw, &info, 0, samples, out)?;
    let _ = std::fs::remove_file(&raw);
    Ok(())
}

/// Channels the source's first audio stream carries. Zero for a source ffprobe
/// finds no audio stream in.
fn source_audio_channels(path: &Path) -> Result<u32, String> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("cannot run ffprobe on {}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.trim().parse::<u32>().unwrap_or(0))
}

/// The sound the item reels have to match, read off the feature's packaged WAV.
pub fn job_sound(packaged_audio: &Path) -> Result<JobSound, String> {
    let info = crate::reel::parse_wav(packaged_audio)?;
    let channels = u32::from(crate::mxf_wrap::wav_channels(packaged_audio)?);
    if channels == 0 || info.block_align == 0 {
        return Err(format!("{} declares no channels", packaged_audio.display()));
    }
    Ok(JobSound {
        channels,
        sample_rate: info.sample_rate,
        bits: (info.block_align * 8 / channels) as u16,
    })
}

/// A composition's reels with the job's library items folded around the
/// feature's, plus what the package has to record for the extra essence.
#[derive(Debug, Default)]
pub struct JoinedComposition {
    pub reels: Vec<crate::cpl::CplReel>,
    pub pkl: Vec<crate::pkl::PklEntry>,
    pub assetmap: Vec<crate::assetmap::AssetMapEntry>,
    pub keys: Vec<crate::encrypt::ContentKey>,
}

/// Conform the job's head and tail items and fold them around `feature`: head
/// reels first, then the feature's untouched, then the tail's. Markers are left
/// to the caller, which places them over the whole run once this has moved
/// which reel comes first.
pub fn join_library_items(
    config: &DcpConfig,
    format: &JobFormat,
    feature: Vec<crate::cpl::CplReel>,
) -> Result<JoinedComposition, String> {
    let head = build_item_reels(config, &config.head_items, format)?;
    let tail = build_item_reels(config, &config.tail_items, format)?;
    Ok(fold_around_feature(head, feature, tail))
}

fn fold_around_feature(
    head: ItemReels,
    feature: Vec<crate::cpl::CplReel>,
    tail: ItemReels,
) -> JoinedComposition {
    let mut joined = JoinedComposition {
        reels: head.reels,
        pkl: head.pkl,
        assetmap: head.assetmap,
        keys: head.keys,
    };
    joined.reels.extend(feature);
    joined.reels.extend(tail.reels);
    joined.pkl.extend(tail.pkl);
    joined.assetmap.extend(tail.assetmap);
    joined.keys.extend(tail.keys);
    joined
}

/// Resolve `names` against the library into attachments, in the order given.
pub fn attach_by_name(
    library: &crate::library::Library,
    names: &[String],
) -> Result<Vec<AttachedItem>, String> {
    names.iter().map(|name| library.attach(name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpl::CplReel;
    use crate::library::{LibraryItem, LibraryItemKind};
    use std::path::PathBuf;

    fn item(name: &str, seconds: f64, has_audio: bool) -> AttachedItem {
        AttachedItem {
            item: LibraryItem {
                name: name.into(),
                kind: LibraryItemKind::HeadIdent,
                file: format!("{name}.mov"),
                seconds,
                width: 1920,
                height: 1080,
                has_audio,
            },
            media: PathBuf::from(format!("/library/{name}.mov")),
        }
    }

    fn reel(id: &str) -> CplReel {
        CplReel {
            reel_id: id.into(),
            ..Default::default()
        }
    }

    fn item_reels(ids: &[&str]) -> ItemReels {
        ItemReels {
            reels: ids.iter().map(|id| reel(id)).collect(),
            pkl: ids
                .iter()
                .map(|id| crate::pkl::PklEntry {
                    id: (*id).into(),
                    ..Default::default()
                })
                .collect(),
            assetmap: ids
                .iter()
                .map(|id| crate::assetmap::AssetMapEntry {
                    id: (*id).into(),
                    path: format!("{id}.mxf"),
                    packing_list: false,
                })
                .collect(),
            keys: Vec::new(),
        }
    }

    #[test]
    fn head_reels_run_before_the_feature_and_tail_reels_after() {
        let joined = fold_around_feature(
            item_reels(&["head-1", "head-2"]),
            vec![reel("feature-a"), reel("feature-b")],
            item_reels(&["tail-1"]),
        );
        let ids: Vec<&str> = joined.reels.iter().map(|r| r.reel_id.as_str()).collect();
        assert_eq!(
            ids,
            ["head-1", "head-2", "feature-a", "feature-b", "tail-1"]
        );
        // every item's essence is registered, the feature's by its own path
        let registered: Vec<&str> = joined.pkl.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(registered, ["head-1", "head-2", "tail-1"]);
        assert_eq!(joined.assetmap.len(), 3);
    }

    #[test]
    fn nothing_attached_builds_nothing() {
        let config = DcpConfig::default();
        let format = JobFormat {
            fps: 24,
            geometry: crate::cpl::PictureGeometry {
                stored_width: 2048,
                stored_height: 1080,
                active_width: 2048,
                active_height: 1080,
            },
            sound: None,
        };
        let built = build_item_reels(&config, &[], &format).unwrap();
        assert!(built.reels.is_empty());
        assert!(built.pkl.is_empty());
        assert!(built.assetmap.is_empty());
    }

    #[test]
    fn a_sound_width_the_wrapper_cannot_write_is_refused_by_name() {
        let sound = JobSound {
            channels: 6,
            sample_rate: 48_000,
            bits: 32,
        };
        let error = sound.ffmpeg_codec().unwrap_err();
        assert!(error.contains("32-bit"), "{error}");
        assert_eq!(
            JobSound { bits: 24, ..sound }.ffmpeg_codec().unwrap(),
            "pcm_s24le"
        );
        assert_eq!(
            JobSound { bits: 16, ..sound }.ffmpeg_codec().unwrap(),
            "pcm_s16le"
        );
    }

    #[test]
    fn the_job_sound_is_read_back_off_a_packaged_wav() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("sound.wav");
        let spec = hound::WavSpec {
            channels: 6,
            sample_rate: 48_000,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        for _ in 0..(48_000 * 6) {
            writer.write_sample(0i32).unwrap();
        }
        writer.finalize().unwrap();
        assert_eq!(
            job_sound(&wav).unwrap(),
            JobSound {
                channels: 6,
                sample_rate: 48_000,
                bits: 24,
            }
        );
    }

    #[test]
    fn an_items_length_is_taken_at_the_jobs_rate_not_its_own() {
        // 8 seconds of ident is 192 frames in a 24 fps job and 200 in a 25 fps one
        let ident = item("Studio Ident", 8.0, true);
        assert_eq!(item_frames(&ident.item, 24), 192);
        assert_eq!(item_frames(&ident.item, 25), 200);
    }
}
