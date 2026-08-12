//! Multi-version (multi-CPL) package assembly.
//!
//! One package, several CPLs sharing the same essence track files (picture, base
//! sound, atmos), differing by subtitle and/or audio track. The shared essence is
//! wrapped once (same UUIDs in every CPL); each manifest entry gets its own CPL,
//! its own subtitle MXF(s), and optionally its own sound MXF(s). The package
//! carries one PKL and one ASSETMAP over every asset, and with `--encrypt` one
//! KeyBundle file per CPL.

use crate::ContentType;
use crate::dcp::DcpConfig;
use crate::encrypt::{ContentKey, GeneratedKey, KeyType};
use crate::reel::{ReelRange, WavInfo, collect_frames, plan_reel_ranges, register_asset};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Dolby Atmos IAB bitstream data-essence UL, as used in real Atmos DCP AuxData.
const ATMOS_DATA_TYPE_UL: &str = "urn:smpte:ul:060e2b34.04010105.0e090604.00000000";

/// One language/subtitle version layered over the shared master.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionSpec {
    /// CPL title / ContentTitleText. Required, non-empty, unique in the manifest.
    pub title: String,
    /// Subtitle track for this version (SRT or supplied SMPTE timed-text XML).
    #[serde(default)]
    pub subtitle: Option<PathBuf>,
    /// Subtitle language code; falls back to the base `--subtitle-language`.
    #[serde(default)]
    pub subtitle_language: Option<String>,
    /// Closed-caption track for this version (SRT or supplied SMPTE XML). Same
    /// input formats as `subtitle`; emitted under the MainClosedCaption CPL role.
    #[serde(default)]
    pub ccap: Option<PathBuf>,
    /// Additional sound MXF for this version; replaces the base sound in its CPL.
    #[serde(default)]
    pub audio: Option<PathBuf>,
    /// Content kind abbreviation (FTR, TLR, ...); falls back to the base `--kind`.
    #[serde(default)]
    pub kind: Option<String>,
}

/// Parse and validate a versions manifest (JSON array of [`VersionSpec`]).
pub fn load_versions(path: &Path) -> Result<Vec<VersionSpec>, String> {
    let json = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read versions manifest {}: {e}", path.display()))?;
    let versions: Vec<VersionSpec> = serde_json::from_str(&json)
        .map_err(|e| format!("bad versions manifest {}: {e}", path.display()))?;
    validate_versions(&versions)?;
    Ok(versions)
}

/// Fail loud on an empty manifest, duplicate/empty titles, missing referenced
/// files, or an unknown content kind. No silent fallbacks.
fn validate_versions(versions: &[VersionSpec]) -> Result<(), String> {
    if versions.is_empty() {
        return Err("versions manifest is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for v in versions {
        if v.title.trim().is_empty() {
            return Err("version title must not be empty".into());
        }
        if !seen.insert(v.title.as_str()) {
            return Err(format!("duplicate version title: {}", v.title));
        }
        if let Some(p) = v.subtitle.as_ref().filter(|p| !p.exists()) {
            return Err(format!(
                "subtitle not found for version '{}': {}",
                v.title,
                p.display()
            ));
        }
        if let Some(p) = v.ccap.as_ref().filter(|p| !p.exists()) {
            return Err(format!(
                "closed caption not found for version '{}': {}",
                v.title,
                p.display()
            ));
        }
        if let Some(p) = v.audio.as_ref().filter(|p| !p.exists()) {
            return Err(format!(
                "audio not found for version '{}': {}",
                v.title,
                p.display()
            ));
        }
        if let Some(k) = v.kind.as_ref()
            && ContentType::from_abbrev(k).is_none()
        {
            return Err(format!(
                "unknown content kind '{k}' for version '{}'",
                v.title
            ));
        }
    }
    Ok(())
}

/// Per-reel shared essence: one picture MXF and (optionally) one base sound MXF
/// wrapped once and referenced by every version that does not override the sound.
struct ReelEssence {
    reel_frames: u64,
    picture_uuid: String,
    picture_key_id: Option<String>,
    picture_key_info: Option<ContentKey>,
    base_sound_uuid: Option<String>,
    base_sound_key_id: Option<String>,
    base_sound_key_info: Option<ContentKey>,
}

/// A wrapped sound track for one reel of one version.
pub(crate) struct SoundReel {
    pub(crate) uuid: String,
    pub(crate) key_id: Option<String>,
    pub(crate) key_info: Option<ContentKey>,
}

/// Build a multi-CPL package from `versions`. Called only when `--versions` is
/// set; the single-CPL [`crate::dcp::create_dcp`] path is untouched.
pub fn create_versioned_dcp(config: &DcpConfig, versions: &[VersionSpec]) -> i32 {
    if let Err(e) = validate_versions(versions) {
        tracing::error!("{e}");
        return -1;
    }

    let Some(j2k_dir) = config.j2k_dir.as_ref() else {
        tracing::error!("A J2K input directory is required");
        return -1;
    };
    if !j2k_dir.is_dir() {
        tracing::error!("J2K input directory does not exist: {}", j2k_dir.display());
        return -1;
    }
    // Reject a bad signer before anything is written, so signing cannot fail
    // once a CPL is on disk and leave a half-signed package.
    if let Some(signer) = config.signer.as_ref()
        && let Err(e) = signer.check_usable()
    {
        tracing::error!("{e}");
        return -1;
    }
    if let Err(e) = std::fs::create_dir_all(&config.output_dir) {
        tracing::error!("Failed to create output directory: {e}");
        return -1;
    }
    if config.encrypt && config.key_out.is_none() {
        tracing::error!(
            "--key-out is required when encrypting; keys are never written next to the DCP"
        );
        return -1;
    }
    if config.right_eye_dir.is_some() {
        tracing::error!("stereoscopic 3D is not supported with --versions");
        return -1;
    }
    if config.pad_head.is_some() || config.pad_tail.is_some() {
        tracing::error!("head/tail padding is not supported with --versions");
        return -1;
    }

    let fps = if config.frame_rate_num > 0 {
        config.frame_rate_num
    } else {
        24
    };
    let is_4k = config.resolution == crate::Resolution::FourK;
    if let Err(e) =
        crate::hfr::validate_fps_resolution(fps, is_4k, config.standard == crate::Standard::Smpte)
    {
        tracing::error!("{e}");
        return -1;
    }

    let frames = collect_frames(j2k_dir);
    if frames.is_empty() {
        tracing::error!("J2K input directory contains no codestreams");
        return -1;
    }
    let total_frames = frames.len() as u64;
    let ranges = plan_reel_ranges(total_frames, fps, config.reel_length_minutes);
    if config.atmos_path.is_some() && ranges.len() > 1 {
        tracing::error!("Atmos is not supported with reel splitting");
        return -1;
    }
    tracing::info!(
        "Building {} version(s) over {total_frames} frames in {} reel(s)",
        versions.len(),
        ranges.len()
    );

    let (pic_w, pic_h) = if config.container_width > 0 && config.container_height > 0 {
        (config.container_width, config.container_height)
    } else {
        (config.resolution.width(), config.resolution.height())
    };

    let mut pkl_entries = Vec::new();
    let mut am_entries = Vec::new();
    let mut temps: Vec<PathBuf> = Vec::new();

    // ── base sound: wrapped once, shared by every version without its own audio ──
    let base_needed = config.audio_path.is_some() && versions.iter().any(|v| v.audio.is_none());
    let base_audio = if base_needed {
        match prepare_audio(config, config.audio_path.as_ref().unwrap(), &mut temps) {
            Ok(prepared) => match crate::reel::parse_wav(&prepared) {
                Ok(info) => {
                    if !info.sample_rate.is_multiple_of(fps) {
                        tracing::error!(
                            "base audio {} Hz is not an integer number of samples per {fps} fps frame",
                            info.sample_rate
                        );
                        cleanup(&temps);
                        return -1;
                    }
                    Some((prepared, info))
                }
                Err(e) => {
                    tracing::error!("{e}");
                    cleanup(&temps);
                    return -1;
                }
            },
            Err(()) => {
                cleanup(&temps);
                return -1;
            }
        }
    } else {
        None
    };

    // ── shared essence: picture (+ base sound) per reel, wrapped once ──
    let mut essences: Vec<ReelEssence> = Vec::with_capacity(ranges.len());
    for (i, range) in ranges.iter().enumerate() {
        let reel_frames = range.frames();
        let slice = frames[range.start as usize..range.end as usize].to_vec();
        let picture_uuid = uuid::Uuid::new_v4();
        let picture_name = format!("picture_{picture_uuid}.mxf");
        let picture_path = config.output_dir.join(&picture_name);
        let picture_key = match mint_key(config, KeyType::Mdik, &picture_uuid.to_string()) {
            Ok(k) => k,
            Err(()) => {
                cleanup(&temps);
                return -1;
            }
        };
        if crate::mxf_wrap::wrap_mxf_files(
            slice,
            &picture_path,
            crate::mxf_wrap::MxfType::J2kPicture,
            fps,
            picture_key.as_ref().map(crate::reel::mxf_enc),
            None,
            Some(*picture_uuid.as_bytes()),
        )
        .is_none()
        {
            tracing::error!("Failed to wrap picture MXF for reel {}", i + 1);
            cleanup(&temps);
            return -1;
        }
        register_asset(
            &mut pkl_entries,
            &mut am_entries,
            &picture_uuid.to_string(),
            &picture_name,
            &picture_path,
        );

        let mut base_sound_uuid = None;
        let mut base_sound_key_id = None;
        let mut base_sound_key_info = None;
        if let Some((src, info)) = base_audio.as_ref() {
            match wrap_sound_reel(
                config,
                src,
                info,
                *range,
                fps,
                &mut pkl_entries,
                &mut am_entries,
                &mut temps,
            ) {
                Ok(sr) => {
                    base_sound_uuid = Some(sr.uuid);
                    base_sound_key_id = sr.key_id;
                    base_sound_key_info = sr.key_info;
                }
                Err(()) => {
                    cleanup(&temps);
                    return -1;
                }
            }
        }

        essences.push(ReelEssence {
            reel_frames,
            picture_uuid: picture_uuid.to_string(),
            picture_key_id: picture_key.as_ref().map(|k| k.info.key_id.clone()),
            picture_key_info: picture_key.map(|k| k.info),
            base_sound_uuid,
            base_sound_key_id,
            base_sound_key_info,
        });
    }

    // ── atmos: shared aux data, single reel only (guarded above) ──
    let mut aux_data: Option<crate::cpl::AuxData> = None;
    if let Some(atmos_path) = config.atmos_path.as_ref() {
        let atmos_uuid = uuid::Uuid::new_v4();
        let atmos_name = format!("atmos_{atmos_uuid}.mxf");
        let atmos_path_out = config.output_dir.join(&atmos_name);
        let Some(track) = crate::mxf_wrap::wrap_mxf_result(&crate::mxf_wrap::MxfWrapConfig {
            input_path: atmos_path.clone(),
            output_mxf: atmos_path_out.clone(),
            mxf_type: crate::mxf_wrap::MxfType::Atmos,
            frame_rate: fps,
            encryption: None,
            mca_config: None,
            asset_uuid: Some(*atmos_uuid.as_bytes()),
        }) else {
            tracing::error!("Failed to wrap Atmos MXF");
            cleanup(&temps);
            return -1;
        };
        if track.duration != essences[0].reel_frames {
            tracing::error!(
                "Atmos track is {} frames but the picture is {}",
                track.duration,
                essences[0].reel_frames
            );
            cleanup(&temps);
            return -1;
        }
        register_asset(
            &mut pkl_entries,
            &mut am_entries,
            &atmos_uuid.to_string(),
            &atmos_name,
            &atmos_path_out,
        );
        aux_data = Some(crate::cpl::AuxData {
            id: atmos_uuid.to_string(),
            edit_rate_num: fps,
            edit_rate_den: 1,
            duration: track.duration,
            entry_point: 0,
            key_id: None,
            data_type: ATMOS_DATA_TYPE_UL.to_string(),
        });
    }

    // ── per version: subtitle + optional own audio + one CPL ──
    // (cpl_id, sanitized title source, bundle keys) for the encrypted key files
    let mut cpl_bundles: Vec<(String, String, Vec<ContentKey>)> = Vec::new();
    for version in versions {
        // own audio (prepared + parsed once), overriding the base sound
        let own_audio = match version.audio.as_ref() {
            Some(path) => match prepare_audio(config, path, &mut temps) {
                Ok(prepared) => match crate::reel::parse_wav(&prepared) {
                    Ok(info) => {
                        if !info.sample_rate.is_multiple_of(fps) {
                            tracing::error!(
                                "audio {} Hz is not an integer number of samples per {fps} fps frame",
                                info.sample_rate
                            );
                            cleanup(&temps);
                            return -1;
                        }
                        Some((prepared, info))
                    }
                    Err(e) => {
                        tracing::error!("{e}");
                        cleanup(&temps);
                        return -1;
                    }
                },
                Err(()) => {
                    cleanup(&temps);
                    return -1;
                }
            },
            None => None,
        };

        let sub_lang = version
            .subtitle_language
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if config.subtitle_language.is_empty() {
                    "en".to_string()
                } else {
                    config.subtitle_language.clone()
                }
            });

        // subtitle source: authored SMPTE XML (single reel only) or SRT cues
        let sub_is_xml = version
            .subtitle
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("xml"));
        if sub_is_xml && ranges.len() > 1 {
            tracing::error!(
                "supplied SMPTE subtitle XML for version '{}' cannot be split across reels; supply SRT",
                version.title
            );
            cleanup(&temps);
            return -1;
        }
        let sub_cues = match version.subtitle.as_ref() {
            Some(path) if !sub_is_xml => match crate::subtitle::parse_srt_frames(path, fps) {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::error!("subtitle parse failed for version '{}': {e}", version.title);
                    cleanup(&temps);
                    return -1;
                }
            },
            _ => None,
        };

        // closed caption source: same rules as the subtitle (XML single-reel only).
        let ccap_is_xml = version
            .ccap
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("xml"));
        if ccap_is_xml && ranges.len() > 1 {
            tracing::error!(
                "supplied SMPTE closed-caption XML for version '{}' cannot be split across reels; supply SRT",
                version.title
            );
            cleanup(&temps);
            return -1;
        }
        let ccap_cues = match version.ccap.as_ref() {
            Some(path) if !ccap_is_xml => match crate::subtitle::parse_srt_frames(path, fps) {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::error!(
                        "closed-caption parse failed for version '{}': {e}",
                        version.title
                    );
                    cleanup(&temps);
                    return -1;
                }
            },
            _ => None,
        };

        let mut cpl_reels = Vec::with_capacity(ranges.len());
        let mut bundle_keys: Vec<ContentKey> = Vec::new();
        for (i, (range, ess)) in ranges.iter().zip(&essences).enumerate() {
            // sound: own audio overrides the shared base sound
            let (sound_uuid, sound_key_id, sound_key_info) = match own_audio.as_ref() {
                Some((src, info)) => {
                    match wrap_sound_reel(
                        config,
                        src,
                        info,
                        *range,
                        fps,
                        &mut pkl_entries,
                        &mut am_entries,
                        &mut temps,
                    ) {
                        Ok(sr) => (Some(sr.uuid), sr.key_id, sr.key_info),
                        Err(()) => {
                            cleanup(&temps);
                            return -1;
                        }
                    }
                }
                None => (
                    ess.base_sound_uuid.clone(),
                    ess.base_sound_key_id.clone(),
                    ess.base_sound_key_info.clone(),
                ),
            };

            // subtitle for this reel
            let mut subtitle_id = None;
            let mut subtitle_duration = 0u64;
            if sub_is_xml && i == 0 {
                match wrap_subtitle_xml(
                    "subtitle",
                    version.subtitle.as_ref().unwrap(),
                    config,
                    fps,
                    &mut pkl_entries,
                    &mut am_entries,
                ) {
                    Ok((id, dur)) => {
                        subtitle_id = Some(id);
                        subtitle_duration = dur;
                    }
                    Err(()) => {
                        cleanup(&temps);
                        return -1;
                    }
                }
            } else if let Some(cues) = sub_cues.as_ref() {
                let rebased = crate::reel::rebase_cues_for_reel(cues, *range);
                if !rebased.is_empty() {
                    match wrap_subtitle_cues(
                        "subtitle",
                        &rebased,
                        &sub_lang,
                        config,
                        fps,
                        &mut pkl_entries,
                        &mut am_entries,
                        &mut temps,
                    ) {
                        Ok((id, dur)) => {
                            subtitle_id = Some(id);
                            subtitle_duration = dur;
                        }
                        Err(()) => {
                            cleanup(&temps);
                            return -1;
                        }
                    }
                }
            }

            // closed caption for this reel (MainClosedCaption role)
            let mut ccap_id = None;
            let mut ccap_duration = 0u64;
            if ccap_is_xml && i == 0 {
                match wrap_subtitle_xml(
                    "ccap",
                    version.ccap.as_ref().unwrap(),
                    config,
                    fps,
                    &mut pkl_entries,
                    &mut am_entries,
                ) {
                    Ok((id, dur)) => {
                        ccap_id = Some(id);
                        ccap_duration = dur;
                    }
                    Err(()) => {
                        cleanup(&temps);
                        return -1;
                    }
                }
            } else if let Some(cues) = ccap_cues.as_ref() {
                let rebased = crate::reel::rebase_cues_for_reel(cues, *range);
                if !rebased.is_empty() {
                    match wrap_subtitle_cues(
                        "ccap",
                        &rebased,
                        &sub_lang,
                        config,
                        fps,
                        &mut pkl_entries,
                        &mut am_entries,
                        &mut temps,
                    ) {
                        Ok((id, dur)) => {
                            ccap_id = Some(id);
                            ccap_duration = dur;
                        }
                        Err(()) => {
                            cleanup(&temps);
                            return -1;
                        }
                    }
                }
            }

            if let Some(k) = ess.picture_key_info.as_ref() {
                bundle_keys.push(k.clone());
            }
            if let Some(k) = sound_key_info.as_ref() {
                bundle_keys.push(k.clone());
            }

            cpl_reels.push(crate::cpl::CplReel {
                reel_id: uuid::Uuid::new_v4().to_string(),
                picture_id: ess.picture_uuid.clone(),
                picture_width: pic_w,
                picture_height: pic_h,
                picture_edit_rate_num: fps,
                picture_edit_rate_den: 1,
                picture_duration: ess.reel_frames,
                picture_entry_point: 0,
                picture_key_id: ess.picture_key_id.clone(),
                sound_id: sound_uuid,
                sound_edit_rate_num: fps,
                sound_edit_rate_den: 1,
                sound_duration: ess.reel_frames,
                sound_entry_point: 0,
                sound_key_id,
                subtitle_id,
                subtitle_edit_rate_num: fps,
                subtitle_edit_rate_den: 1,
                subtitle_duration,
                subtitle_entry_point: 0,
                subtitle_language: subtitle_duration.gt(&0).then(|| sub_lang.clone()),
                ccap_id,
                ccap_edit_rate_num: fps,
                ccap_edit_rate_den: 1,
                ccap_duration,
                ccap_entry_point: 0,
                ccap_language: ccap_duration.gt(&0).then(|| sub_lang.clone()),
                stereoscopic: false,
                aux_data: aux_data.clone(),
            });
        }

        // sound layout for this version's CompositionMetadataAsset
        let main_sound = own_audio
            .as_ref()
            .map(|(p, _)| p)
            .or_else(|| base_audio.as_ref().map(|(p, _)| p))
            .and_then(|path| {
                let ch = crate::mxf_wrap::wav_channels(path).ok()? as u32;
                let configuration =
                    crate::cpl::main_sound_configuration(ch, config.hi_channel, config.vi_channel)?;
                let sample_rate = crate::mxf_wrap::wav_sample_rate(path).unwrap_or(48000);
                Some(crate::cpl::MainSound {
                    configuration,
                    sample_rate,
                })
            });

        let content_kind = version
            .kind
            .as_deref()
            .and_then(ContentType::from_abbrev)
            .unwrap_or(config.content_type)
            .as_cpl_kind()
            .to_string();

        let cpl_uuid = uuid::Uuid::new_v4().to_string();
        let cpl_path = config.output_dir.join(format!("CPL_{cpl_uuid}.xml"));
        let cpl_config = crate::cpl::CplConfig {
            title: version.title.clone(),
            content_kind,
            reels: cpl_reels,
            standard: config.standard,
            main_sound,
            ..Default::default()
        };
        if crate::cpl::generate_cpl(&cpl_config, &cpl_uuid, &cpl_path) != 0 {
            tracing::error!("Failed to generate CPL for version '{}'", version.title);
            cleanup(&temps);
            return -1;
        }
        // Sign before the PKL entry below hashes the file, otherwise the PKL
        // records the hash of the unsigned CPL.
        if !crate::package_signature::sign_if_configured(
            config.signer.as_ref(),
            &cpl_path,
            &format!("CPL for version '{}'", version.title),
        ) {
            cleanup(&temps);
            return -1;
        }
        pkl_entries.push(crate::pkl::PklEntry {
            id: cpl_uuid.clone(),
            asset_type: "text/xml".into(),
            file: cpl_path.clone(),
            hash: crate::hash::hash_file(&cpl_path).unwrap_or_default(),
            size: std::fs::metadata(&cpl_path).map(|m| m.len()).unwrap_or(0),
        });
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: cpl_uuid.clone(),
            path: cpl_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            packing_list: false,
        });
        cpl_bundles.push((cpl_uuid, version.title.clone(), bundle_keys));
    }

    // ── one PKL, one ASSETMAP over every asset ──
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_path = config.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    if crate::pkl::generate_pkl(&pkl_entries, &pkl_uuid, config.standard, None, &pkl_path) != 0 {
        tracing::error!("Failed to generate PKL");
        cleanup(&temps);
        return -1;
    }
    // Nothing hashes the PKL, so this can follow the write.
    if !crate::package_signature::sign_if_configured(config.signer.as_ref(), &pkl_path, "PKL") {
        cleanup(&temps);
        return -1;
    }
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
        cleanup(&temps);
        return -1;
    }

    // ── per-CPL key bundles ──
    if config.encrypt {
        let Some(base) = config.key_out.as_ref() else {
            tracing::error!("--key-out is required when encrypting");
            cleanup(&temps);
            return -1;
        };
        if let Some(parent) = base.parent().filter(|p| !p.as_os_str().is_empty())
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::error!("Failed to create key-out directory: {e}");
            cleanup(&temps);
            return -1;
        }
        for (i, (cpl_id, title, keys)) in cpl_bundles.iter().enumerate() {
            let out = key_file_path(base, i + 1, title);
            let bundle = crate::encrypt::KeyBundle {
                cpl_id: cpl_id.clone(),
                keys: keys.clone(),
            };
            if let Err(e) = bundle.write(&out) {
                tracing::error!("Failed to write keys file: {e}");
                cleanup(&temps);
                return -1;
            }
            tracing::warn!(
                "Wrote content keys for CPL {cpl_id} ('{title}') to {}: plaintext AES keys; keep secret, do not ship in the DCP.",
                out.display()
            );
        }
    }

    cleanup(&temps);
    tracing::info!(
        "Multi-version DCP created ({} CPLs): {}",
        cpl_bundles.len(),
        config.output_dir.display()
    );
    0
}

/// Wrap one reel's slice of a sound source into an MXF, registering it in the
/// PKL/ASSETMAP. Returns the asset uuid plus any content key.
#[allow(clippy::too_many_arguments)]
pub(crate) fn wrap_sound_reel(
    config: &DcpConfig,
    src: &Path,
    info: &WavInfo,
    range: ReelRange,
    fps: u32,
    pkl: &mut Vec<crate::pkl::PklEntry>,
    am: &mut Vec<crate::assetmap::AssetMapEntry>,
    temps: &mut Vec<PathBuf>,
) -> Result<SoundReel, ()> {
    let spf = (info.sample_rate / fps) as u64;
    let uuid = uuid::Uuid::new_v4();
    let wav_tmp = config.output_dir.join(format!("sound_{uuid}.wav"));
    if let Err(e) =
        crate::reel::write_reel_wav(src, info, range.start * spf, range.frames() * spf, &wav_tmp)
    {
        tracing::error!("audio split failed: {e}");
        return Err(());
    }
    let name = format!("sound_{uuid}.mxf");
    let path = config.output_dir.join(&name);
    let key = mint_key(config, KeyType::Mdak, &uuid.to_string())?;
    let wrapped = crate::mxf_wrap::wrap_mxf_files(
        vec![wav_tmp.clone()],
        &path,
        crate::mxf_wrap::MxfType::PcmAudio,
        fps,
        key.as_ref().map(crate::reel::mxf_enc),
        None,
        Some(*uuid.as_bytes()),
    );
    temps.push(wav_tmp);
    if wrapped.is_none() {
        tracing::error!("Failed to wrap sound MXF");
        return Err(());
    }
    register_asset(pkl, am, &uuid.to_string(), &name, &path);
    Ok(SoundReel {
        uuid: uuid.to_string(),
        key_id: key.as_ref().map(|k| k.info.key_id.clone()),
        key_info: key.map(|k| k.info),
    })
}

/// Wrap already-rebased SRT cues into a per-reel timed-text MXF. `prefix` names
/// the files ("subtitle" or "ccap"); the essence is identical either way.
#[allow(clippy::too_many_arguments)]
pub(crate) fn wrap_subtitle_cues(
    prefix: &str,
    cues: &[crate::subtitle::SubCue],
    lang: &str,
    config: &DcpConfig,
    fps: u32,
    pkl: &mut Vec<crate::pkl::PklEntry>,
    am: &mut Vec<crate::assetmap::AssetMapEntry>,
    temps: &mut Vec<PathBuf>,
) -> Result<(String, u64), ()> {
    let uuid = uuid::Uuid::new_v4();
    let dcst = config.output_dir.join(format!("{prefix}_{uuid}.xml"));
    if let Err(e) = crate::subtitle::write_dcst_frames(cues, lang, fps, &dcst) {
        tracing::error!("{prefix} write failed: {e}");
        return Err(());
    }
    let name = format!("{prefix}_{uuid}.mxf");
    let path = config.output_dir.join(&name);
    let wrapped = crate::mxf_wrap::wrap_mxf_result(&crate::mxf_wrap::MxfWrapConfig {
        input_path: dcst.clone(),
        output_mxf: path.clone(),
        mxf_type: crate::mxf_wrap::MxfType::TimedText,
        frame_rate: fps,
        encryption: None,
        mca_config: None,
        asset_uuid: Some(*uuid.as_bytes()),
    });
    temps.push(dcst);
    let Some(track) = wrapped else {
        tracing::error!("Failed to wrap {prefix} MXF");
        return Err(());
    };
    register_asset(pkl, am, &uuid.to_string(), &name, &path);
    Ok((uuid.to_string(), track.duration))
}

/// Wrap a supplied SMPTE timed-text XML into an MXF unchanged (single reel).
/// `prefix` names the file ("subtitle" or "ccap").
pub(crate) fn wrap_subtitle_xml(
    prefix: &str,
    xml_path: &Path,
    config: &DcpConfig,
    fps: u32,
    pkl: &mut Vec<crate::pkl::PklEntry>,
    am: &mut Vec<crate::assetmap::AssetMapEntry>,
) -> Result<(String, u64), ()> {
    let uuid = uuid::Uuid::new_v4();
    let name = format!("{prefix}_{uuid}.mxf");
    let path = config.output_dir.join(&name);
    let wrapped = crate::mxf_wrap::wrap_mxf_result(&crate::mxf_wrap::MxfWrapConfig {
        input_path: xml_path.to_path_buf(),
        output_mxf: path.clone(),
        mxf_type: crate::mxf_wrap::MxfType::TimedText,
        frame_rate: fps,
        encryption: None,
        mca_config: None,
        asset_uuid: Some(*uuid.as_bytes()),
    });
    let Some(track) = wrapped else {
        tracing::error!("Failed to wrap {prefix} MXF");
        return Err(());
    };
    register_asset(pkl, am, &uuid.to_string(), &name, &path);
    Ok((uuid.to_string(), track.duration))
}

/// Prepare an audio source to canonical DCP 5.1 layout when it is 5.1, else use
/// it as-is. A converted temp file is tracked in `temps` for cleanup.
pub(crate) fn prepare_audio(
    config: &DcpConfig,
    path: &Path,
    temps: &mut Vec<PathBuf>,
) -> Result<PathBuf, ()> {
    let out = config
        .output_dir
        .join(format!(".dcpwizard_vaudio_{}.wav", uuid::Uuid::new_v4()));
    match crate::mxf_wrap::prepare_51_audio(path, &out, config.audio_input_order) {
        Ok(true) => {
            temps.push(out.clone());
            Ok(out)
        }
        Ok(false) => Ok(path.to_path_buf()),
        Err(e) => {
            tracing::error!("audio preparation failed: {e}");
            Err(())
        }
    }
}

pub(crate) fn mint_key(
    config: &DcpConfig,
    kind: KeyType,
    uuid: &str,
) -> Result<Option<GeneratedKey>, ()> {
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

/// Derive a per-CPL key filename from `--key-out`: keys.json -> keys_1_<title>.json.
pub(crate) fn key_file_path(base: &Path, index: usize, title: &str) -> PathBuf {
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("keys");
    let ext = base.extension().and_then(|s| s.to_str());
    let sanitized: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let name = match ext {
        Some(ext) => format!("{stem}_{index}_{sanitized}.{ext}"),
        None => format!("{stem}_{index}_{sanitized}"),
    };
    base.with_file_name(name)
}

pub(crate) fn cleanup(temps: &[PathBuf]) {
    for t in temps {
        let _ = std::fs::remove_file(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_manifest() {
        assert!(validate_versions(&[]).is_err());
    }

    #[test]
    fn rejects_duplicate_titles() {
        let v = vec![
            VersionSpec {
                title: "A".into(),
                subtitle: None,
                subtitle_language: None,
                ccap: None,
                audio: None,
                kind: None,
            },
            VersionSpec {
                title: "A".into(),
                subtitle: None,
                subtitle_language: None,
                ccap: None,
                audio: None,
                kind: None,
            },
        ];
        assert!(validate_versions(&v).is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        let json = r#"[{"title":"A","bogus":true}]"#;
        let r: Result<Vec<VersionSpec>, _> = serde_json::from_str(json);
        assert!(r.is_err(), "deny_unknown_fields must reject typos");
    }

    #[test]
    fn key_file_path_derives_from_key_out() {
        let p = key_file_path(Path::new("out/keys.json"), 2, "French Sub");
        assert_eq!(p, Path::new("out/keys_2_French_Sub.json"));
        let p = key_file_path(Path::new("keys"), 1, "en");
        assert_eq!(p, Path::new("keys_1_en"));
    }
}
