use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use asdcplib::jp2k::MxfReader;
use postkit::grok_encoder::RawFrame;

/// 16 MB covers a single 4K J2K frame.
const MAX_FRAME_BUF: usize = 16 * 1024 * 1024;

/// Re-encode an existing DCP's picture essence at a different bandwidth (and,
/// optionally, resolution), copying audio and subtitle tracks unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DcpTranscodeConfig {
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    /// target picture bandwidth in Mbit/s (required; 0 is rejected)
    pub target_bitrate_mbps: u32,
    /// optional target resolution; 0 keeps the source dimensions
    pub target_width: u32,
    pub target_height: u32,
    /// KDM XML for an encrypted source (with `recipient_key`). Each source frame
    /// is decrypted and decoded in memory; the re-encoded output is cleartext.
    pub kdm: Option<PathBuf>,
    /// Recipient RSA private key (PEM) matching `kdm`.
    pub recipient_key: Option<PathBuf>,
    /// dcpwizard KEYS.json, an alternative key source to `kdm`.
    pub keys: Option<PathBuf>,
}

/// One MXF that ships in the output DCP (declared in CPL/PKL/ASSETMAP).
struct ShippedAsset {
    id: String,
    filename: String,
    hash: String,
    size: u64,
}

/// Result of re-encoding one reel's picture track.
struct NewPicture {
    id: String,
    filename: String,
    hash: String,
    size: u64,
    duration: u64,
    width: u32,
    height: u32,
    edit_rate_num: u32,
    edit_rate_den: u32,
}

/// Transcode an existing DCP: re-encode every reel's picture essence to the
/// target bandwidth, copy audio/subtitle tracks verbatim, and emit a fresh
/// CPL/PKL/ASSETMAP. Fails loud on encrypted input.
pub fn transcode_dcp(config: &DcpTranscodeConfig) -> i32 {
    if config.target_bitrate_mbps == 0 {
        tracing::error!("--video-bit-rate is required and must be > 0");
        return -1;
    }
    if !config.input_dir.exists() {
        tracing::error!("Input DCP not found: {}", config.input_dir.display());
        return -1;
    }
    if config.input_dir == config.output_dir {
        tracing::error!("output must differ from input");
        return -1;
    }

    let cpls = crate::multi_cpl::list_cpls(&config.input_dir);
    let Some(cpl) = cpls.first() else {
        tracing::error!("No CPL found in {}", config.input_dir.display());
        return -1;
    };
    let cpl_path = config.input_dir.join(&cpl.file_path);
    let cpl_content = std::fs::read_to_string(&cpl_path).unwrap_or_default();
    let timeline = crate::multi_cpl::get_timeline(&cpl_path);
    if timeline.is_empty() {
        tracing::error!("CPL has no reels");
        return -1;
    }

    // encrypted input needs key material: with a KDM+recipient key or KEYS.json
    // each source frame is decrypted in memory before decode; without it we
    // cannot re-encode what we cannot decode, so fail loud.
    let key_source =
        match crate::decrypt::key_source_opt(&config.keys, &config.kdm, &config.recipient_key) {
            Ok(k) => k,
            Err(e) => {
                tracing::error!("{e}");
                return -1;
            }
        };
    if cpl_content.contains("<KeyId>") && key_source.is_none() {
        tracing::error!(
            "input DCP is encrypted; supply --kdm + --recipient-key or --keys to transcode it"
        );
        return -1;
    }

    let standard = if cpl_content.contains("digicine.com") {
        crate::Standard::Interop
    } else {
        crate::Standard::Smpte
    };

    if let Err(e) = std::fs::create_dir_all(&config.output_dir) {
        tracing::error!("Failed to create output directory: {e}");
        return -1;
    }

    let mut cpl_reels: Vec<crate::cpl::CplReel> = Vec::new();
    let mut shipped: Vec<ShippedAsset> = Vec::new();
    // re-encoding keeps the frame count, so marker offsets carry over unchanged
    let source_markers = crate::markers::markers_from_cpl(&cpl_content);

    for (reel_index, entry) in timeline.iter().enumerate() {
        let src_pic = PathBuf::from(&entry.picture_file);
        if entry.picture_file.is_empty() || !src_pic.exists() {
            tracing::error!("reel {} picture MXF not found", entry.reel_number);
            return -1;
        }
        let Some(pic) = transcode_picture(&src_pic, config, key_source.as_ref()) else {
            return -1;
        };

        // every non-picture track: cleartext copies verbatim (asset id preserved);
        // with a key source, an encrypted one is decrypted and rewrapped so the
        // cleartext output stays coherent.
        let fps_snd = (pic.edit_rate_num as f64 / pic.edit_rate_den as f64).round() as u32;
        let sound = match sound_track(
            &entry.sound_file,
            &entry.sound_asset_id,
            key_source.as_ref(),
            fps_snd,
            &config.output_dir,
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("{e}");
                return -1;
            }
        };
        let subtitle = match timed_text_track(
            &entry.subtitle_file,
            &entry.subtitle_asset_id,
            "subtitle",
            key_source.as_ref(),
            &config.output_dir,
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("{e}");
                return -1;
            }
        };
        let ccap = match timed_text_track(
            &entry.ccap_file,
            &entry.ccap_asset_id,
            "ccap",
            key_source.as_ref(),
            &config.output_dir,
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("{e}");
                return -1;
            }
        };
        let aux = match aux_data_track(entry, key_source.as_ref(), &config.output_dir) {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("{e}");
                return -1;
            }
        };

        let subtitle_lang = if entry.subtitle_language.is_empty() {
            None
        } else {
            Some(entry.subtitle_language.clone())
        };
        let ccap_lang = if entry.ccap_language.is_empty() {
            None
        } else {
            Some(entry.ccap_language.clone())
        };

        cpl_reels.push(crate::cpl::CplReel {
            reel_id: uuid::Uuid::new_v4().to_string(),
            picture_id: pic.id.clone(),
            picture_width: pic.width,
            picture_height: pic.height,
            picture_edit_rate_num: pic.edit_rate_num,
            picture_edit_rate_den: pic.edit_rate_den,
            picture_duration: pic.duration,
            picture_entry_point: 0,
            picture_key_id: None,
            sound_id: sound.as_ref().map(|s| s.id.clone()),
            sound_edit_rate_num: pic.edit_rate_num,
            sound_edit_rate_den: pic.edit_rate_den,
            sound_duration: if sound.is_some() { pic.duration } else { 0 },
            sound_entry_point: 0,
            sound_key_id: None,
            subtitle_id: subtitle.as_ref().map(|(s, _)| s.id.clone()),
            subtitle_edit_rate_num: pic.edit_rate_num,
            subtitle_edit_rate_den: pic.edit_rate_den,
            subtitle_duration: subtitle.as_ref().map(|(_, d)| *d).unwrap_or(0),
            subtitle_entry_point: 0,
            subtitle_language: subtitle_lang,
            ccap_id: ccap.as_ref().map(|(s, _)| s.id.clone()),
            ccap_edit_rate_num: pic.edit_rate_num,
            ccap_edit_rate_den: pic.edit_rate_den,
            ccap_duration: ccap.as_ref().map(|(_, d)| *d).unwrap_or(0),
            ccap_entry_point: 0,
            ccap_language: ccap_lang,
            stereoscopic: false,
            aux_data: aux.as_ref().map(|(_, a)| a.clone()),
            markers: source_markers.get(reel_index).cloned().unwrap_or_default(),
            ..Default::default()
        });

        shipped.push(ShippedAsset {
            id: pic.id,
            filename: pic.filename,
            hash: pic.hash,
            size: pic.size,
        });
        if let Some(s) = sound {
            shipped.push(s);
        }
        if let Some((s, _)) = subtitle {
            shipped.push(s);
        }
        if let Some((s, _)) = ccap {
            shipped.push(s);
        }
        if let Some((s, _)) = aux {
            shipped.push(s);
        }
    }

    // ── CPL ────────────────────────────────────────────────────────────
    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let out_cpl_path = config.output_dir.join(format!("CPL_{cpl_uuid}.xml"));
    let content_kind = if cpl.content_kind.is_empty() {
        "feature".to_string()
    } else {
        cpl.content_kind.clone()
    };
    let cpl_config = crate::cpl::CplConfig {
        title: cpl.content_title.clone(),
        content_kind,
        rating: String::new(),
        reels: cpl_reels,
        standard,
        // only the picture is re-encoded, so the source's sound layout still holds
        main_sound: crate::cpl::main_sound_from_cpl(&cpl_content),
        sign_language: None,
        ..Default::default()
    };
    if crate::cpl::generate_cpl(&cpl_config, &cpl_uuid, &out_cpl_path) != 0 {
        tracing::error!("Failed to generate CPL");
        return -1;
    }

    // ── PKL ────────────────────────────────────────────────────────────
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let cpl_hash = crate::hash::hash_file(&out_cpl_path).unwrap_or_default();
    let cpl_size = std::fs::metadata(&out_cpl_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let mut pkl_entries = vec![crate::pkl::PklEntry {
        id: cpl_uuid.clone(),
        asset_type: "text/xml".into(),
        file: out_cpl_path.clone(),
        hash: cpl_hash,
        size: cpl_size,
    }];
    for a in &shipped {
        pkl_entries.push(crate::pkl::PklEntry {
            id: a.id.clone(),
            asset_type: "application/mxf".into(),
            file: config.output_dir.join(&a.filename),
            hash: a.hash.clone(),
            size: a.size,
        });
    }
    let pkl_path = config.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    if crate::pkl::generate_pkl(&pkl_entries, &pkl_uuid, standard, None, &pkl_path) != 0 {
        tracing::error!("Failed to generate PKL");
        return -1;
    }

    // ── ASSETMAP ───────────────────────────────────────────────────────
    let mut am_entries = vec![
        crate::assetmap::AssetMapEntry {
            id: pkl_uuid,
            path: file_name(&pkl_path),
            packing_list: true,
        },
        crate::assetmap::AssetMapEntry {
            id: cpl_uuid,
            path: file_name(&out_cpl_path),
            packing_list: false,
        },
    ];
    for a in &shipped {
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: a.id.clone(),
            path: a.filename.clone(),
            packing_list: false,
        });
    }
    if crate::assetmap::generate_assetmap(&am_entries, &config.output_dir, standard, None) != 0 {
        tracing::error!("Failed to generate ASSETMAP");
        return -1;
    }

    tracing::info!(
        "Transcoded DCP to {} ({} reel(s) re-encoded at {} Mbps)",
        config.output_dir.display(),
        timeline.len(),
        config.target_bitrate_mbps
    );
    0
}

/// Decode one picture MXF in memory and re-encode it at the bandwidth's bytes a frame.
fn transcode_picture(
    src_mxf: &Path,
    config: &DcpTranscodeConfig,
    key_source: Option<&crate::decrypt::KeySource>,
) -> Option<NewPicture> {
    let mut reader = MxfReader::new();
    if let Err(e) = reader.open_read(&src_mxf.to_string_lossy()) {
        tracing::error!("Failed to open picture MXF {}: {e}", src_mxf.display());
        return None;
    }
    let desc = match reader.picture_descriptor() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to read picture descriptor: {e}");
            return None;
        }
    };
    // encrypted source: build the AES/HMAC contexts from the key source, keyed by
    // this MXF's own KeyId, so every read_frame below decrypts in memory.
    let info = match reader.writer_info() {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("Failed to read picture writer info: {e}");
            return None;
        }
    };
    let key_source = key_source.filter(|_| info.encrypted_essence);
    if info.encrypted_essence {
        let Some(ks) = key_source else {
            tracing::error!(
                "picture MXF {} is encrypted and no key source was given",
                src_mxf.display()
            );
            return None;
        };
        if let Err(e) = ks.contexts(&info, "picture") {
            tracing::error!("{e}");
            return None;
        }
    }
    let frame_count = desc.container_duration;
    if frame_count == 0 {
        tracing::error!("picture MXF {} has no frames", src_mxf.display());
        return None;
    }
    let src_w = desc.stored_width;
    let src_h = desc.stored_height;
    let edit_num = desc.edit_rate.numerator.max(1) as u32;
    let edit_den = desc.edit_rate.denominator.max(1) as u32;
    let fps = (edit_num as f64 / edit_den as f64).round() as u32;

    let resize = config.target_width > 0 && config.target_height > 0;
    let (out_w, out_h) = if resize {
        (config.target_width, config.target_height)
    } else {
        (src_w, src_h)
    };

    let target_codestream_bytes =
        crate::encode::video_codestream_byte_cap(fps, config.target_bitrate_mbps, false);

    let work = config
        .output_dir
        .join(format!(".transcode_{}", uuid::Uuid::new_v4()));
    let j2k_dir = work.join("j2k");
    if std::fs::create_dir_all(&j2k_dir).is_err() {
        tracing::error!("Failed to create transcode work dir");
        return None;
    }

    let resize_to = resize.then_some((out_w, out_h));
    let work_dir = work.as_path();
    let writer_info = &info;
    let open_loader = || -> Result<postkit::encode::FrameLoader<'_>, String> {
        let mut reader = MxfReader::new();
        reader
            .open_read(&src_mxf.to_string_lossy())
            .map_err(|e| format!("Failed to open picture MXF {}: {e}", src_mxf.display()))?;
        let mut crypto = match key_source {
            Some(ks) => Some(ks.contexts(writer_info, "picture")?),
            None => None,
        };
        let mut buf = vec![0u8; MAX_FRAME_BUF];
        Ok(Box::new(move |frame_index: u64| {
            decode_source_frame(
                &mut reader,
                crypto.as_mut(),
                frame_index as u32,
                &mut buf,
                resize_to,
                work_dir,
            )
        }))
    };
    // AlreadyPq is the source colour that compresses untransformed
    let options = postkit::encode::StreamEncodeOptions {
        output_dir: j2k_dir.clone(),
        compression_ratio: crate::encode::DEFAULT_COMPRESSION_RATIO,
        target_codestream_bytes: Some(target_codestream_bytes),
        fps: postkit::encode::FrameRate::whole(fps),
        source_colour: postkit::encode::SourceColour::AlreadyPq,
        ..Default::default()
    };
    let result = postkit::encode::encode_loaded_frames(
        u64::from(frame_count),
        open_loader,
        &options,
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(AtomicBool::new(false)),
        None,
        |_| {},
    );
    if !result.success {
        tracing::error!("re-encode failed: {}", result.error);
        let _ = std::fs::remove_dir_all(&work);
        return None;
    }

    let id = uuid::Uuid::new_v4();
    let filename = format!("picture_{id}.mxf");
    let out_mxf = config.output_dir.join(&filename);
    let track = crate::mxf_wrap::wrap_mxf_files(
        sorted_files(&j2k_dir),
        &out_mxf,
        crate::mxf_wrap::MxfType::J2kPicture,
        fps,
        None,
        None,
        Some(*id.as_bytes()),
    );
    let _ = std::fs::remove_dir_all(&work);
    let track = track?;

    let hash = crate::hash::hash_file(&out_mxf).ok()?;
    let size = std::fs::metadata(&out_mxf).map(|m| m.len()).unwrap_or(0);
    Some(NewPicture {
        id: track.uuid,
        filename,
        hash,
        size,
        duration: if track.duration > 0 {
            track.duration
        } else {
            frame_count as u64
        },
        width: out_w,
        height: out_h,
        edit_rate_num: edit_num,
        edit_rate_den: edit_den,
    })
}

/// Copy an essence MXF into the output DCP unchanged, keeping its asset id.
/// Returns `Ok(None)` when there is no such track.
fn copy_track(
    src_file: &str,
    asset_id: &str,
    prefix: &str,
    out_dir: &Path,
) -> Result<Option<ShippedAsset>, String> {
    if src_file.is_empty() || asset_id.is_empty() {
        return Ok(None);
    }
    let src = PathBuf::from(src_file);
    if !src.exists() {
        return Err(format!("{prefix} MXF not found: {src_file}"));
    }
    let filename = format!("{prefix}_{asset_id}.mxf");
    let dst = out_dir.join(&filename);
    std::fs::copy(&src, &dst).map_err(|e| format!("failed to copy {prefix} MXF: {e}"))?;
    let hash = crate::hash::hash_file(&dst)?;
    let size = std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
    Ok(Some(ShippedAsset {
        id: asset_id.to_string(),
        filename,
        hash,
        size,
    }))
}

/// Resolve the sound track for the output: without a key source, copy verbatim
/// (asset id preserved); with one, an encrypted sound is decrypted and rewrapped
/// as cleartext (via the shared decrypt path) so the output CPL stays coherent.
fn sound_track(
    src_file: &str,
    asset_id: &str,
    key_source: Option<&crate::decrypt::KeySource>,
    fps: u32,
    out_dir: &Path,
) -> Result<Option<ShippedAsset>, String> {
    match key_source {
        Some(ks) => Ok(
            crate::decrypt::process_sound(src_file, asset_id, ks, fps, out_dir)?.map(from_decrypt),
        ),
        None => copy_track(src_file, asset_id, "sound", out_dir),
    }
}

/// Resolve a timed-text track (subtitle or closed caption) for the output, with
/// the frame count the essence declares: a cleartext track copies verbatim
/// (asset id preserved), an encrypted one is decrypted and rewrapped as
/// cleartext.
fn timed_text_track(
    src_file: &str,
    asset_id: &str,
    prefix: &str,
    key_source: Option<&crate::decrypt::KeySource>,
    out_dir: &Path,
) -> Result<Option<(ShippedAsset, u64)>, String> {
    Ok(
        crate::decrypt::process_timed_text(src_file, asset_id, prefix, key_source, out_dir)?
            .map(|(asset, duration)| (from_decrypt(asset), duration)),
    )
}

/// Resolve the Atmos / DCData auxiliary track, decrypting it when the source is
/// encrypted. A cleartext copy still goes through the decrypt path, since the
/// rebuilt reel declares the track's edit rate and duration off its descriptor.
fn aux_data_track(
    entry: &crate::multi_cpl::TimelineEntry,
    key_source: Option<&crate::decrypt::KeySource>,
    out_dir: &Path,
) -> Result<Option<(ShippedAsset, crate::cpl::AuxData)>, String> {
    Ok(crate::decrypt::process_aux_data(
        &entry.aux_data_file,
        &entry.aux_data_asset_id,
        &entry.aux_data_type,
        key_source,
        out_dir,
    )?
    .map(|(s, a)| (from_decrypt(s), a)))
}

/// Map a shared decrypt-path asset onto this module's ShippedAsset.
fn from_decrypt(s: crate::decrypt::ShippedAsset) -> ShippedAsset {
    ShippedAsset {
        id: s.id,
        filename: s.filename,
        hash: s.hash,
        size: s.size,
    }
}

/// Read one source frame, decrypting it when the essence is encrypted, decode
/// it in memory, and hand it to the encoder as planar components. A resize goes
/// through ffmpeg, which reads and writes a TIFF in `work`.
fn decode_source_frame(
    reader: &mut MxfReader,
    crypto: Option<&mut (
        asdcplib::crypto::AesDecContext,
        asdcplib::crypto::HmacContext,
    )>,
    index: u32,
    buf: &mut [u8],
    resize_to: Option<(u32, u32)>,
    work: &Path,
) -> Result<RawFrame, String> {
    let (dec, hmac) = match crypto {
        Some((d, h)) => (Some(d), Some(h)),
        None => (None, None),
    };
    let n = reader
        .read_frame(index, buf, dec, hmac)
        .map_err(|e| format!("Failed to read frame {index} (wrong key or MIC mismatch): {e}"))?;
    let decoded = postkit::grok_decoder::decode(buf[..n].to_vec(), 0)
        .map_err(|e| format!("cannot decode frame {index}: {e}"))?;
    let precision = decoded.precision;
    let (width, height) = (decoded.width, decoded.height);
    let components: [Vec<i32>; 3] = decoded
        .components
        .try_into()
        .map_err(|_| format!("frame {index} does not carry three components"))?;

    let Some((out_w, out_h)) = resize_to else {
        return Ok(RawFrame::Planar {
            components,
            width,
            height,
            precision,
            index: u64::from(index),
        });
    };
    let tif = work.join(format!("frame_{index:07}.tif"));
    let decoded_frame = postkit::grok_decoder::DecodedFrame {
        width,
        height,
        precision,
        components: components.to_vec(),
        chroma_subsampled: false,
    };
    postkit::grok::write_tiff_rgb(
        &tif,
        width,
        height,
        precision,
        &decoded_frame.interleaved_samples()?,
    )?;
    if !scale_tiff(&tif, out_w, out_h) {
        return Err(format!("cannot scale frame {index} to {out_w}x{out_h}"));
    }
    let scaled = crate::grok::load_tiff(&tif);
    let _ = std::fs::remove_file(&tif);
    let scaled = scaled?;
    Ok(RawFrame::Planar {
        components: scaled.components,
        width: scaled.width,
        height: scaled.height,
        precision: scaled.precision,
        index: u64::from(index),
    })
}

/// Scale a TIFF in place to the target dimensions using ffmpeg.
fn scale_tiff(tif: &Path, w: u32, h: u32) -> bool {
    let tmp = tif.with_extension("scaled.tif");
    let out = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(tif)
        .arg("-vf")
        .arg(format!("scale={w}:{h}"))
        .arg(&tmp)
        .output();
    match out {
        Ok(o) if o.status.success() => std::fs::rename(&tmp, tif).is_ok(),
        Ok(o) => {
            tracing::error!(
                "ffmpeg scale failed: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            false
        }
        Err(e) => {
            tracing::error!("Failed to run ffmpeg: {e}");
            false
        }
    }
}

fn sorted_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    v
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_track_absent_is_none() {
        let dir = tempfile::tempdir().unwrap();
        // no file and no id -> no track to ship
        assert!(copy_track("", "", "sound", dir.path()).unwrap().is_none());
        assert!(
            copy_track("", "some-id", "sound", dir.path())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn copy_track_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let r = copy_track("/nope/missing.mxf", "abc", "sound", dir.path());
        assert!(r.is_err());
    }

    #[test]
    fn copy_track_copies_and_keeps_id() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.mxf");
        std::fs::write(&src, b"essence bytes").unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let a = copy_track(&src.to_string_lossy(), "the-id", "sound", &out)
            .unwrap()
            .unwrap();
        assert_eq!(a.id, "the-id");
        assert_eq!(a.filename, "sound_the-id.mxf");
        assert!(out.join(&a.filename).exists());
        assert_eq!(a.size, 13);
    }

    #[test]
    fn transcode_rejects_zero_bitrate() {
        let dir = tempfile::tempdir().unwrap();
        let config = DcpTranscodeConfig {
            input_dir: dir.path().to_path_buf(),
            output_dir: dir.path().join("out"),
            target_bitrate_mbps: 0,
            ..Default::default()
        };
        assert_eq!(transcode_dcp(&config), -1);
    }
}
