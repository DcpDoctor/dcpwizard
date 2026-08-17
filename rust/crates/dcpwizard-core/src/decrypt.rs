//! Decrypt an encrypted DCP into a cleartext DCP with the same structure.
//!
//! Each encrypted essence MXF is read through asdcplib's AES decrypt + HMAC
//! context (keyed by the MXF's own cryptographic_key_id, i.e. the CPL KeyId) and
//! re-wrapped as cleartext; a bad content key fails the per-frame MIC. Picture,
//! sound and Atmos go frame by frame, timed text by document plus ancillary
//! resources. Non-encrypted assets copy byte-identical. The output CPL keeps the
//! reel structure and durations but drops all KeyId/encryption elements, so it
//! gets a new CPL id, and PKL/ASSETMAP/VOLINDEX are regenerated with fresh
//! hashes.
//!
//! Key material lives only in memory: an UnwrappedKdm (recipient-key path, keys
//! zeroed on drop) or a dcpwizard KEYS.json. Keys never reach logs, errors, or
//! temp files, and no key-bearing struct is Debug-printed.

use asdcplib::crypto::{AesDecContext, HmacContext};
use asdcplib::{jp2k, pcm};
use postkit::certificate::UnwrappedKdm;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 16 MB covers a single 4K J2K frame or one PCM edit unit.
const MAX_FRAME_BUF: usize = 16 * 1024 * 1024;

/// Decrypt an encrypted DCP into a cleartext DCP of the same structure. Keys
/// come from either a KDM + recipient private key, or a dcpwizard KEYS.json.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DcpDecryptConfig {
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    /// KDM XML; used with `recipient_key` to recover the content keys.
    pub kdm: Option<PathBuf>,
    /// Recipient RSA private key (PEM) matching the KDM.
    pub recipient_key: Option<PathBuf>,
    /// dcpwizard KEYS.json (written by `create --encrypt`), an alternative to a KDM.
    pub keys: Option<PathBuf>,
}

/// In-memory content keys, keyed by KeyId. Either an unwrapped KDM (whose keys
/// zero on drop) or the raw keys from a KEYS.json. Never logged. Shared with the
/// transcode-dcp KDM path.
pub(crate) enum KeySource {
    Kdm(UnwrappedKdm),
    Keys(HashMap<uuid::Uuid, [u8; 16]>),
}

impl KeySource {
    /// The 16-byte AES-128 content key for `key_id`, if held. The copy is
    /// short-lived (handed straight to an AesDecContext) and never logged.
    pub(crate) fn content_key(&self, key_id: &uuid::Uuid) -> Option<[u8; 16]> {
        match self {
            KeySource::Kdm(k) => k.content_key(key_id).copied(),
            KeySource::Keys(m) => m.get(key_id).copied(),
        }
    }

    /// Build the AES + HMAC contexts for an encrypted essence, keyed by the MXF's
    /// own cryptographic_key_id. Fails loud when no held key covers that KeyId.
    pub(crate) fn contexts(
        &self,
        info: &asdcplib::WriterInfo,
        what: &str,
    ) -> Result<(AesDecContext, HmacContext), String> {
        let key_id = uuid::Uuid::from_bytes(info.cryptographic_key_id);
        let key = self
            .content_key(&key_id)
            .ok_or_else(|| format!("KDM/keys do not cover {what} KeyId {key_id}"))?;
        let mut dec = AesDecContext::new();
        dec.init_key(&key)
            .map_err(|e| format!("AES key init failed: {e}"))?;
        let mut hmac = HmacContext::new();
        hmac.init_key(&key, info.label_set)
            .map_err(|e| format!("HMAC key init failed: {e}"))?;
        Ok((dec, hmac))
    }
}

/// Resolve a key source from any of `--keys` / `--kdm` + `--recipient-key`.
/// Returns `Ok(None)` when no key material was supplied.
pub(crate) fn key_source_opt(
    keys: &Option<PathBuf>,
    kdm: &Option<PathBuf>,
    recipient_key: &Option<PathBuf>,
) -> Result<Option<KeySource>, String> {
    if let Some(keys) = keys {
        let bundle = crate::encrypt::KeyBundle::read(keys)?;
        let mut map = HashMap::new();
        for k in &bundle.keys {
            let (_key_type, key_id, key) = k.to_raw()?;
            map.insert(key_id, key);
        }
        return Ok(Some(KeySource::Keys(map)));
    }
    match (kdm, recipient_key) {
        (Some(kdm), Some(rk)) => Ok(Some(KeySource::Kdm(postkit::certificate::unwrap_kdm_file(
            kdm, rk,
        )?))),
        (None, None) => Ok(None),
        _ => Err("decrypting needs both --kdm and --recipient-key (or use --keys)".into()),
    }
}

/// Resolve the key source for decrypt: `--keys` KEYS.json, or `--kdm` + `--recipient-key`.
fn build_key_source(config: &DcpDecryptConfig) -> Result<KeySource, String> {
    key_source_opt(&config.keys, &config.kdm, &config.recipient_key)?.ok_or_else(|| {
        "decrypt needs either --keys KEYS.json or both --kdm and --recipient-key".into()
    })
}

/// One MXF shipped in the output DCP (declared in CPL/PKL/ASSETMAP).
pub(crate) struct ShippedAsset {
    pub(crate) id: String,
    pub(crate) filename: String,
    pub(crate) hash: String,
    pub(crate) size: u64,
}

/// Picture geometry needed to rebuild the CPL reel.
struct PicInfo {
    width: u32,
    height: u32,
    edit_num: u32,
    edit_den: u32,
    duration: u64,
}

/// Decrypt an encrypted DCP: rewrap every encrypted essence as cleartext, copy
/// non-encrypted assets byte-identical, drop CPL encryption, and regenerate
/// PKL/ASSETMAP/VOLINDEX. Fails loud on a missing key or a MIC mismatch.
pub fn decrypt_dcp(config: &DcpDecryptConfig) -> i32 {
    match decrypt_dcp_inner(config) {
        Ok(reels) => {
            tracing::info!(
                "Decrypted DCP to {} ({} reel(s))",
                config.output_dir.display(),
                reels
            );
            0
        }
        Err(e) => {
            tracing::error!("{e}");
            -1
        }
    }
}

fn decrypt_dcp_inner(config: &DcpDecryptConfig) -> Result<usize, String> {
    if !config.input_dir.exists() {
        return Err(format!(
            "input DCP not found: {}",
            config.input_dir.display()
        ));
    }
    if config.input_dir == config.output_dir {
        return Err("output must differ from input".into());
    }

    let cpls = crate::multi_cpl::list_cpls(&config.input_dir);
    let cpl = cpls
        .first()
        .ok_or_else(|| format!("no CPL found in {}", config.input_dir.display()))?;
    let cpl_path = config.input_dir.join(&cpl.file_path);
    let cpl_content = std::fs::read_to_string(&cpl_path).unwrap_or_default();
    let timeline = crate::multi_cpl::get_timeline(&cpl_path);
    if timeline.is_empty() {
        return Err("CPL has no reels".into());
    }

    let standard = if cpl_content.contains("digicine.com") {
        crate::Standard::Interop
    } else {
        crate::Standard::Smpte
    };

    let keys = build_key_source(config)?;

    std::fs::create_dir_all(&config.output_dir)
        .map_err(|e| format!("cannot create output directory: {e}"))?;

    let mut cpl_reels: Vec<crate::cpl::CplReel> = Vec::new();
    let mut shipped: Vec<ShippedAsset> = Vec::new();
    // the output is the same composition, so it keeps the source's markers
    let source_markers = crate::markers::markers_from_cpl(&cpl_content);

    for (reel_index, entry) in timeline.iter().enumerate() {
        let src_pic = PathBuf::from(&entry.picture_file);
        if entry.picture_file.is_empty() || !src_pic.exists() {
            return Err(format!("reel {} picture MXF not found", entry.reel_number));
        }
        let (pic_asset, pic) =
            process_picture(&src_pic, &entry.picture_asset_id, &keys, &config.output_dir)?;
        let fps = (pic.edit_num as f64 / pic.edit_den as f64).round() as u32;

        let sound = process_sound(
            &entry.sound_file,
            &entry.sound_asset_id,
            &keys,
            fps,
            &config.output_dir,
        )?;

        let subtitle = process_timed_text(
            &entry.subtitle_file,
            &entry.subtitle_asset_id,
            "subtitle",
            Some(&keys),
            &config.output_dir,
        )?;
        let ccap = process_timed_text(
            &entry.ccap_file,
            &entry.ccap_asset_id,
            "ccap",
            Some(&keys),
            &config.output_dir,
        )?;
        let aux = process_aux_data(
            &entry.aux_data_file,
            &entry.aux_data_asset_id,
            &entry.aux_data_type,
            Some(&keys),
            &config.output_dir,
        )?;

        let subtitle_lang =
            (!entry.subtitle_language.is_empty()).then(|| entry.subtitle_language.clone());
        let ccap_lang = (!entry.ccap_language.is_empty()).then(|| entry.ccap_language.clone());

        cpl_reels.push(crate::cpl::CplReel {
            reel_id: uuid::Uuid::new_v4().to_string(),
            picture_id: pic_asset.id.clone(),
            picture_width: pic.width,
            picture_height: pic.height,
            picture_edit_rate_num: pic.edit_num,
            picture_edit_rate_den: pic.edit_den,
            picture_duration: pic.duration,
            picture_entry_point: 0,
            picture_key_id: None,
            sound_id: sound.as_ref().map(|s| s.id.clone()),
            sound_edit_rate_num: pic.edit_num,
            sound_edit_rate_den: pic.edit_den,
            sound_duration: if sound.is_some() { pic.duration } else { 0 },
            sound_entry_point: 0,
            sound_key_id: None,
            subtitle_id: subtitle.as_ref().map(|(s, _)| s.id.clone()),
            subtitle_edit_rate_num: pic.edit_num,
            subtitle_edit_rate_den: pic.edit_den,
            subtitle_duration: subtitle.as_ref().map(|(_, d)| *d).unwrap_or(0),
            subtitle_entry_point: 0,
            subtitle_language: subtitle_lang,
            ccap_id: ccap.as_ref().map(|(c, _)| c.id.clone()),
            ccap_edit_rate_num: pic.edit_num,
            ccap_edit_rate_den: pic.edit_den,
            ccap_duration: ccap.as_ref().map(|(_, d)| *d).unwrap_or(0),
            ccap_entry_point: 0,
            ccap_language: ccap_lang,
            stereoscopic: false,
            aux_data: aux.as_ref().map(|(_, a)| a.clone()),
            markers: source_markers.get(reel_index).cloned().unwrap_or_default(),
            ..Default::default()
        });

        shipped.push(ShippedAsset {
            id: pic_asset.id,
            filename: pic_asset.filename,
            hash: pic_asset.hash,
            size: pic_asset.size,
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

    // ── CPL (new id: KeyIds are removed, so the bytes always change) ─────
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
        main_sound: None,
        sign_language: None,
        ..Default::default()
    };
    if crate::cpl::generate_cpl(&cpl_config, &cpl_uuid, &out_cpl_path) != 0 {
        return Err("failed to generate CPL".into());
    }

    // ── PKL ─────────────────────────────────────────────────────────────
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
        return Err("failed to generate PKL".into());
    }

    // ── ASSETMAP + VOLINDEX ─────────────────────────────────────────────
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
        return Err("failed to generate ASSETMAP".into());
    }

    Ok(timeline.len())
}

/// Read the picture MXF's geometry, then either decrypt+rewrap it (encrypted) or
/// copy it byte-identical (cleartext), returning the shipped asset and geometry.
fn process_picture(
    src: &Path,
    asset_id: &str,
    keys: &KeySource,
    out_dir: &Path,
) -> Result<(ShippedAsset, PicInfo), String> {
    let mut reader = jp2k::MxfReader::new();
    reader
        .open_read(&src.to_string_lossy())
        .map_err(|e| format!("open picture MXF {}: {e}", src.display()))?;
    let desc = reader
        .picture_descriptor()
        .map_err(|e| format!("read picture descriptor: {e}"))?;
    let info = reader
        .writer_info()
        .map_err(|e| format!("read picture writer info: {e}"))?;

    let frame_count = desc.container_duration;
    if frame_count == 0 {
        return Err(format!("picture MXF {} has no frames", src.display()));
    }
    let pic = PicInfo {
        width: desc.stored_width,
        height: desc.stored_height,
        edit_num: desc.edit_rate.numerator.max(1) as u32,
        edit_den: desc.edit_rate.denominator.max(1) as u32,
        duration: frame_count as u64,
    };
    let fps = (pic.edit_num as f64 / pic.edit_den as f64).round() as u32;

    if !info.encrypted_essence {
        let asset = copy_cleartext(src, asset_id, "picture", out_dir)?;
        return Ok((asset, pic));
    }

    let (mut dec, mut hmac) = keys.contexts(&info, "picture")?;

    let work = out_dir.join(format!(".decrypt_{}", uuid::Uuid::new_v4()));
    let j2k_dir = work.join("j2k");
    std::fs::create_dir_all(&j2k_dir).map_err(|e| format!("cannot create work dir: {e}"))?;

    let mut buf = vec![0u8; MAX_FRAME_BUF];
    for i in 0..frame_count {
        let n = reader
            .read_frame(i, &mut buf, Some(&mut dec), Some(&mut hmac))
            .map_err(|e| {
                let _ = std::fs::remove_dir_all(&work);
                format!("decrypt picture frame {i} (wrong key or MIC mismatch): {e}")
            })?;
        if let Err(e) = std::fs::write(j2k_dir.join(format!("frame_{i:07}.j2c")), &buf[..n]) {
            let _ = std::fs::remove_dir_all(&work);
            return Err(format!("write decrypted picture frame {i}: {e}"));
        }
    }

    let asset_id = uuid::Uuid::new_v4();
    let filename = format!("picture_{asset_id}.mxf");
    let out_mxf = out_dir.join(&filename);
    let track = crate::mxf_wrap::wrap_mxf_files(
        sorted_files(&j2k_dir),
        &out_mxf,
        crate::mxf_wrap::MxfType::J2kPicture,
        fps,
        None,
        None,
        Some(*asset_id.as_bytes()),
    );
    let _ = std::fs::remove_dir_all(&work);
    let track = track.ok_or("rewrap of decrypted picture failed")?;

    Ok((ship(&out_mxf, track.uuid, filename)?, pic))
}

/// Decrypt+rewrap or byte-copy the sound MXF. `None` when there is no sound track.
pub(crate) fn process_sound(
    src_file: &str,
    asset_id: &str,
    keys: &KeySource,
    fps: u32,
    out_dir: &Path,
) -> Result<Option<ShippedAsset>, String> {
    if src_file.is_empty() || asset_id.is_empty() {
        return Ok(None);
    }
    let src = PathBuf::from(src_file);
    if !src.exists() {
        return Err(format!("sound MXF not found: {src_file}"));
    }

    let mut reader = pcm::MxfReader::new();
    reader
        .open_read(&src.to_string_lossy())
        .map_err(|e| format!("open sound MXF {}: {e}", src.display()))?;
    let info = reader
        .writer_info()
        .map_err(|e| format!("read sound writer info: {e}"))?;

    if !info.encrypted_essence {
        return Ok(Some(copy_cleartext(&src, asset_id, "sound", out_dir)?));
    }

    let ad = reader
        .audio_descriptor()
        .map_err(|e| format!("read audio descriptor: {e}"))?;
    let (mut dec, mut hmac) = keys.contexts(&info, "sound")?;

    let mut pcm_data = Vec::new();
    let mut buf = vec![0u8; MAX_FRAME_BUF];
    for i in 0..ad.container_duration {
        let n = reader
            .read_frame(i, &mut buf, Some(&mut dec), Some(&mut hmac))
            .map_err(|e| format!("decrypt sound frame {i} (wrong key or MIC mismatch): {e}"))?;
        pcm_data.extend_from_slice(&buf[..n]);
    }

    let work = out_dir.join(format!(".decrypt_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work).map_err(|e| format!("cannot create work dir: {e}"))?;
    let wav_path = work.join("sound.wav");
    let sample_rate = ad.audio_sampling_rate.numerator.max(1) as u32;
    if let Err(e) = write_wav(
        &wav_path,
        ad.channel_count as u16,
        ad.quantization_bits as u16,
        sample_rate,
        &pcm_data,
    ) {
        let _ = std::fs::remove_dir_all(&work);
        return Err(e);
    }

    let asset_id = uuid::Uuid::new_v4();
    let filename = format!("sound_{asset_id}.mxf");
    let out_mxf = out_dir.join(&filename);
    let track = crate::mxf_wrap::wrap_mxf_files(
        vec![wav_path],
        &out_mxf,
        crate::mxf_wrap::MxfType::PcmAudio,
        fps,
        None,
        None,
        Some(*asset_id.as_bytes()),
    );
    let _ = std::fs::remove_dir_all(&work);
    let track = track.ok_or("rewrap of decrypted sound failed")?;

    Ok(Some(ship(&out_mxf, track.uuid, filename)?))
}

/// Decrypt+rewrap or byte-copy a timed-text MXF (subtitle or closed caption),
/// with the frame count the essence declares, which is what the rebuilt reel has
/// to declare for it. The rewrap keeps the asset id, the document (so its
/// ResourceID survives), the duration and every ancillary resource under its own
/// id. `None` when there is no such track.
pub(crate) fn process_timed_text(
    src_file: &str,
    asset_id: &str,
    prefix: &str,
    keys: Option<&KeySource>,
    out_dir: &Path,
) -> Result<Option<(ShippedAsset, u64)>, String> {
    if src_file.is_empty() || asset_id.is_empty() {
        return Ok(None);
    }
    let src = PathBuf::from(src_file);
    if !src.exists() {
        return Err(format!("{prefix} MXF not found: {src_file}"));
    }

    let mut reader = asdcplib::timed_text::MxfReader::new();
    reader
        .open_read(&src.to_string_lossy())
        .map_err(|e| format!("open {prefix} MXF {}: {e}", src.display()))?;
    let info = reader
        .writer_info()
        .map_err(|e| format!("read {prefix} writer info: {e}"))?;
    let desc = reader
        .descriptor()
        .map_err(|e| format!("read {prefix} descriptor: {e}"))?;
    let duration = desc.container_duration as u64;

    if !info.encrypted_essence {
        return Ok(Some((
            copy_cleartext(&src, asset_id, prefix, out_dir)?,
            duration,
        )));
    }

    let asset_uuid = uuid::Uuid::parse_str(asset_id)
        .map_err(|e| format!("{prefix} asset id '{asset_id}' is not a UUID: {e}"))?;
    let keys = keys.ok_or_else(|| {
        format!("{prefix} asset {asset_id} is encrypted and no key material was supplied")
    })?;
    let (mut dec, mut hmac) = keys.contexts(&info, prefix)?;

    // no resource is bigger than the file carrying it, so one buffer this size
    // always holds whichever the reader hands back
    let capacity = std::fs::metadata(&src)
        .map(|m| m.len() as usize)
        .map_err(|e| format!("cannot size {prefix} MXF {}: {e}", src.display()))?;
    let mut buf = vec![0u8; capacity];

    let work = out_dir.join(format!(".decrypt_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work).map_err(|e| format!("cannot create work dir: {e}"))?;
    let outcome = write_decrypted_timed_text(&mut reader, &mut dec, &mut hmac, &mut buf, &work);
    let decrypted = match outcome {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(format!("decrypt {prefix} essence: {e}"));
        }
    };

    let fps = (desc.edit_rate.numerator.max(1) as f64 / desc.edit_rate.denominator.max(1) as f64)
        .round() as u32;
    let filename = format!("{prefix}_{asset_id}.mxf");
    let out_mxf = out_dir.join(&filename);
    let track = crate::mxf_wrap::wrap_timed_text_resources(
        &decrypted.document,
        &decrypted.resources,
        &out_mxf,
        fps,
        Some(*asset_uuid.as_bytes()),
        Some(desc.container_duration),
        None,
    );
    let _ = std::fs::remove_dir_all(&work);
    track.ok_or_else(|| format!("rewrap of decrypted {prefix} failed"))?;

    Ok(Some((
        ship(&out_mxf, asset_id.to_string(), filename)?,
        duration,
    )))
}

/// A decrypted timed-text track on disk: the document, plus each ancillary
/// resource paired with the id it was stored under, which is what the rewrap
/// has to write them back as.
struct DecryptedTimedText {
    document: PathBuf,
    resources: Vec<(PathBuf, [u8; 16])>,
}

/// Decrypt the timed-text document and every ancillary resource into `work`.
fn write_decrypted_timed_text(
    reader: &mut asdcplib::timed_text::MxfReader,
    dec: &mut AesDecContext,
    hmac: &mut HmacContext,
    buf: &mut [u8],
    work: &Path,
) -> Result<DecryptedTimedText, String> {
    let n = reader
        .read_timed_text_resource(buf, Some(dec), Some(hmac))
        .map_err(|e| format!("wrong key or MIC mismatch: {e}"))?;
    let document = work.join("document.xml");
    std::fs::write(&document, &buf[..n]).map_err(|e| format!("write decrypted document: {e}"))?;

    let count = reader
        .ancillary_resource_count()
        .map_err(|e| format!("read ancillary resource count: {e}"))?;
    let mut resources = Vec::with_capacity(count);
    for index in 0..count {
        let resource = reader
            .ancillary_resource_info(index)
            .map_err(|e| format!("read ancillary resource {index}: {e}"))?;
        let n = reader
            .read_ancillary_resource(&resource.uuid, buf, Some(dec), Some(hmac))
            .map_err(|e| format!("decrypt ancillary resource {index}: {e}"))?;
        // the rewrap picks the stored MIME type off the extension, so the file
        // has to be named for the type it was embedded under
        let extension = match resource.mime_type {
            asdcplib::timed_text::MimeType::OpenType => "ttf",
            asdcplib::timed_text::MimeType::Png => "png",
            asdcplib::timed_text::MimeType::Binary => "bin",
        };
        let path = work.join(format!("resource_{index:03}.{extension}"));
        std::fs::write(&path, &buf[..n])
            .map_err(|e| format!("write decrypted ancillary resource {index}: {e}"))?;
        resources.push((path, resource.uuid));
    }
    Ok(DecryptedTimedText {
        document,
        resources,
    })
}

/// Decrypt+rewrap or byte-copy an auxiliary-data (Atmos / ST 429-18) MXF,
/// keeping its asset id, and describe it for the rebuilt reel. `None` when there
/// is no such track.
pub(crate) fn process_aux_data(
    src_file: &str,
    asset_id: &str,
    data_type: &str,
    keys: Option<&KeySource>,
    out_dir: &Path,
) -> Result<Option<(ShippedAsset, crate::cpl::AuxData)>, String> {
    if src_file.is_empty() || asset_id.is_empty() {
        return Ok(None);
    }
    let src = PathBuf::from(src_file);
    if !src.exists() {
        return Err(format!("aux data MXF not found: {src_file}"));
    }

    let mut reader = asdcplib::atmos::MxfReader::new();
    reader
        .open_read(&src.to_string_lossy())
        .map_err(|e| format!("open aux data MXF {}: {e}", src.display()))?;
    let info = reader
        .writer_info()
        .map_err(|e| format!("read aux data writer info: {e}"))?;
    let desc = reader
        .atmos_descriptor()
        .map_err(|e| format!("read aux data descriptor: {e}"))?;

    let aux = crate::cpl::AuxData {
        id: asset_id.to_string(),
        edit_rate_num: desc.edit_rate.numerator.max(1) as u32,
        edit_rate_den: desc.edit_rate.denominator.max(1) as u32,
        duration: desc.container_duration as u64,
        entry_point: 0,
        key_id: None,
        data_type: data_type.to_string(),
    };

    if !info.encrypted_essence {
        return Ok(Some((
            copy_cleartext(&src, asset_id, "atmos", out_dir)?,
            aux,
        )));
    }

    let asset_uuid = uuid::Uuid::parse_str(asset_id)
        .map_err(|e| format!("aux data asset id '{asset_id}' is not a UUID: {e}"))?;
    let keys = keys.ok_or_else(|| {
        format!("aux data asset {asset_id} is encrypted and no key material was supplied")
    })?;
    let (mut dec, mut hmac) = keys.contexts(&info, "aux data")?;

    let work = out_dir.join(format!(".decrypt_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work).map_err(|e| format!("cannot create work dir: {e}"))?;

    let mut buf = vec![0u8; MAX_FRAME_BUF];
    for i in 0..desc.container_duration {
        let n = reader
            .read_frame(i, &mut buf, Some(&mut dec), Some(&mut hmac))
            .map_err(|e| {
                let _ = std::fs::remove_dir_all(&work);
                format!("decrypt aux data frame {i} (wrong key or MIC mismatch): {e}")
            })?;
        if let Err(e) = std::fs::write(work.join(format!("atmos_{i:07}.bin")), &buf[..n]) {
            let _ = std::fs::remove_dir_all(&work);
            return Err(format!("write decrypted aux data frame {i}: {e}"));
        }
    }

    let filename = format!("atmos_{asset_id}.mxf");
    let out_mxf = out_dir.join(&filename);
    let track = crate::mxf_wrap::wrap_mxf_files(
        sorted_files(&work),
        &out_mxf,
        crate::mxf_wrap::MxfType::Atmos,
        aux.edit_rate_num,
        None,
        None,
        Some(*asset_uuid.as_bytes()),
    );
    let _ = std::fs::remove_dir_all(&work);
    track.ok_or("rewrap of decrypted aux data failed")?;

    Ok(Some((ship(&out_mxf, asset_id.to_string(), filename)?, aux)))
}

/// Copy an MXF into the output DCP unchanged, keeping its asset id.
fn copy_cleartext(
    src: &Path,
    asset_id: &str,
    prefix: &str,
    out_dir: &Path,
) -> Result<ShippedAsset, String> {
    let filename = format!("{prefix}_{asset_id}.mxf");
    let dst = out_dir.join(&filename);
    std::fs::copy(src, &dst).map_err(|e| format!("failed to copy {prefix} MXF: {e}"))?;
    ship(&dst, asset_id.to_string(), filename)
}

/// Hash + size an output MXF into a ShippedAsset with the given id/filename.
fn ship(path: &Path, id: String, filename: String) -> Result<ShippedAsset, String> {
    let hash = crate::hash::hash_file(path)?;
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(ShippedAsset {
        id,
        filename,
        hash,
        size,
    })
}

/// Write a canonical 44-byte-header PCM WAV from raw interleaved sample bytes.
fn write_wav(
    path: &Path,
    channels: u16,
    bits: u16,
    sample_rate: u32,
    data: &[u8],
) -> Result<(), String> {
    let block_align = (bits / 8) * channels;
    let byte_rate = sample_rate * block_align as u32;
    let mut w = Vec::with_capacity(44 + data.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&byte_rate.to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(data);
    std::fs::write(path, &w).map_err(|e| format!("cannot write WAV {}: {e}", path.display()))
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
