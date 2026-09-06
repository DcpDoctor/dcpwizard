use crate::ContentType;
use crate::dcp::DcpConfig;
use crate::encrypt::ContentKey;
use crate::reel::{collect_frames, plan_reel_ranges, register_asset};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A CPL entry found in a DCP.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CplEntry {
    pub id: String,
    pub file_path: String,
    pub content_title: String,
    pub content_kind: String,
}

/// Timeline entry representing a reel within a CPL.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub reel_id: String,
    pub reel_number: u32,
    pub duration_frames: u64,
    pub entry_point: u64,
    pub edit_rate: String,
    pub picture_asset_id: String,
    pub sound_asset_id: String,
    pub subtitle_asset_id: String,
    #[serde(default)]
    pub ccap_asset_id: String,
    /// Atmos / DCData auxiliary track (ST 429-18), the reel's AuxData asset.
    #[serde(default)]
    pub aux_data_asset_id: String,
    pub picture_file: String,
    pub sound_file: String,
    pub subtitle_file: String,
    #[serde(default)]
    pub ccap_file: String,
    #[serde(default)]
    pub aux_data_file: String,
    pub subtitle_language: String,
    #[serde(default)]
    pub ccap_language: String,
    /// SMPTE data-essence UL the AuxData asset declares as its DataType.
    #[serde(default)]
    pub aux_data_type: String,
}

/// List all CPLs in a DCP by parsing the ASSETMAP.
pub fn list_cpls(dcp_dir: &Path) -> Vec<CplEntry> {
    let assetmap_path = find_assetmap(dcp_dir);
    let assetmap_path = match assetmap_path {
        Some(p) => p,
        None => {
            tracing::error!("No ASSETMAP found in {}", dcp_dir.display());
            return Vec::new();
        }
    };

    let content = match std::fs::read_to_string(&assetmap_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to read ASSETMAP: {e}");
            return Vec::new();
        }
    };

    let mut cpls = Vec::new();

    // Parse ASSETMAP XML to find CPL assets
    let mut in_asset = false;
    let mut current_id = String::new();
    let mut current_path = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("<Asset>") || trimmed.starts_with("<Asset ") {
            in_asset = true;
            current_id.clear();
            current_path.clear();
        } else if trimmed == "</Asset>" {
            if in_asset && !current_id.is_empty() && !current_path.is_empty() {
                // Check if this is a CPL by reading the file
                let full_path = dcp_dir.join(&current_path);
                if is_xml_file(&full_path)
                    && let Ok(file_content) = std::fs::read_to_string(&full_path)
                    && file_content.contains("CompositionPlaylist")
                {
                    let title = extract_xml_value(&file_content, "ContentTitleText")
                        .or_else(|| extract_xml_value(&file_content, "ContentTitle"))
                        .unwrap_or_default();
                    let kind = extract_xml_value(&file_content, "ContentKind").unwrap_or_default();

                    cpls.push(CplEntry {
                        id: current_id.clone(),
                        file_path: current_path.clone(),
                        content_title: title,
                        content_kind: kind,
                    });
                }
            }
            in_asset = false;
        } else if in_asset {
            if let Some(id) = extract_xml_value(trimmed, "Id") {
                current_id = id.replace("urn:uuid:", "");
            }
            if let Some(path) = extract_xml_value(trimmed, "Path") {
                current_path = path;
            }
        }
    }

    cpls
}

/// Get the timeline (reel structure) for a specific CPL, resolving asset file paths from the DCP directory.
pub fn get_timeline(cpl_path: &Path) -> Vec<TimelineEntry> {
    let dcp_dir = cpl_path.parent().unwrap_or(Path::new("."));
    let content = match std::fs::read_to_string(cpl_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to read CPL: {e}");
            return Vec::new();
        }
    };

    // Build asset UUID → file path map from ASSETMAP
    let asset_map = parse_assetmap(dcp_dir);

    let mut entries = Vec::new();
    let mut reel_number = 0u32;

    // Simple XML parser for reel info
    let mut in_reel = false;
    let mut reel_id = String::new();
    let mut duration = 0u64;
    let mut entry_point = 0u64;
    let mut edit_rate = String::new();
    let mut picture_id = String::new();
    let mut sound_id = String::new();
    let mut subtitle_id = String::new();
    let mut subtitle_lang = String::new();
    let mut ccap_id = String::new();
    let mut ccap_lang = String::new();
    let mut aux_data_id = String::new();
    let mut aux_data_type = String::new();
    let mut in_picture = false;
    let mut in_sound = false;
    let mut in_subtitle = false;
    let mut in_ccap = false;
    let mut in_aux_data = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("<Reel>") || trimmed.contains("<Reel ") {
            in_reel = true;
            reel_number += 1;
            reel_id.clear();
            duration = 0;
            entry_point = 0;
            edit_rate.clear();
            picture_id.clear();
            sound_id.clear();
            subtitle_id.clear();
            subtitle_lang.clear();
            ccap_id.clear();
            ccap_lang.clear();
            aux_data_id.clear();
            aux_data_type.clear();
            in_picture = false;
            in_sound = false;
            in_subtitle = false;
            in_ccap = false;
            in_aux_data = false;
        } else if trimmed.contains("</Reel>") {
            if in_reel {
                let picture_file = asset_map
                    .get(&picture_id)
                    .map(|p| dcp_dir.join(p).to_string_lossy().into_owned())
                    .unwrap_or_default();
                let sound_file = asset_map
                    .get(&sound_id)
                    .map(|p| dcp_dir.join(p).to_string_lossy().into_owned())
                    .unwrap_or_default();
                let subtitle_file = asset_map
                    .get(&subtitle_id)
                    .map(|p| dcp_dir.join(p).to_string_lossy().into_owned())
                    .unwrap_or_default();
                let ccap_file = asset_map
                    .get(&ccap_id)
                    .map(|p| dcp_dir.join(p).to_string_lossy().into_owned())
                    .unwrap_or_default();
                let aux_data_file = asset_map
                    .get(&aux_data_id)
                    .map(|p| dcp_dir.join(p).to_string_lossy().into_owned())
                    .unwrap_or_default();
                entries.push(TimelineEntry {
                    reel_id: reel_id.clone(),
                    reel_number,
                    duration_frames: duration,
                    entry_point,
                    edit_rate: edit_rate.clone(),
                    picture_asset_id: picture_id.clone(),
                    sound_asset_id: sound_id.clone(),
                    subtitle_asset_id: subtitle_id.clone(),
                    ccap_asset_id: ccap_id.clone(),
                    aux_data_asset_id: aux_data_id.clone(),
                    picture_file,
                    sound_file,
                    subtitle_file,
                    ccap_file,
                    aux_data_file,
                    subtitle_language: subtitle_lang.clone(),
                    ccap_language: ccap_lang.clone(),
                    aux_data_type: aux_data_type.clone(),
                });
            }
            in_reel = false;
        } else if in_reel {
            if trimmed.contains("MainPicture") && trimmed.contains('<') && !trimmed.contains('/') {
                in_picture = true;
                in_sound = false;
                in_subtitle = false;
            } else if trimmed.contains("MainSound")
                && trimmed.contains('<')
                && !trimmed.contains('/')
            {
                in_sound = true;
                in_picture = false;
                in_subtitle = false;
            } else if trimmed.contains("MainSubtitle")
                && trimmed.contains('<')
                && !trimmed.contains('/')
            {
                in_subtitle = true;
                in_picture = false;
                in_sound = false;
            } else if is_closed_caption(trimmed, opens_local_element) {
                in_ccap = true;
                in_picture = false;
                in_sound = false;
                in_subtitle = false;
            } else if opens_local_element(trimmed, "AuxData") {
                in_aux_data = true;
                in_picture = false;
                in_sound = false;
                in_subtitle = false;
            } else if trimmed.contains("</MainPicture>") {
                in_picture = false;
            } else if trimmed.contains("</MainSound>") {
                in_sound = false;
            } else if trimmed.contains("</MainSubtitle>") {
                in_subtitle = false;
            } else if is_closed_caption(trimmed, closes_local_element) {
                in_ccap = false;
            } else if closes_local_element(trimmed, "AuxData") {
                in_aux_data = false;
            }

            if let Some(id) = extract_xml_value(trimmed, "Id") {
                let clean_id = id.replace("urn:uuid:", "");
                if reel_id.is_empty()
                    && !in_picture
                    && !in_sound
                    && !in_subtitle
                    && !in_ccap
                    && !in_aux_data
                {
                    reel_id = clean_id;
                } else if in_picture && picture_id.is_empty() {
                    picture_id = clean_id;
                } else if in_sound && sound_id.is_empty() {
                    sound_id = clean_id;
                } else if in_subtitle && subtitle_id.is_empty() {
                    subtitle_id = clean_id;
                } else if in_ccap && ccap_id.is_empty() {
                    ccap_id = clean_id;
                } else if in_aux_data && aux_data_id.is_empty() {
                    aux_data_id = clean_id;
                }
            }
            if in_subtitle && let Some(lang) = extract_xml_value(trimmed, "Language") {
                subtitle_lang = lang;
            }
            if in_ccap && let Some(lang) = extract_local_xml_value(trimmed, "Language") {
                ccap_lang = lang;
            }
            if in_aux_data && let Some(ul) = extract_local_xml_value(trimmed, "DataType") {
                aux_data_type = ul;
            }
            if let Some(d) = extract_xml_value(trimmed, "Duration")
                && let Ok(v) = d.parse::<u64>()
            {
                duration = v;
            }
            if let Some(ep) = extract_xml_value(trimmed, "EntryPoint")
                && let Ok(v) = ep.parse::<u64>()
                && in_picture
            {
                entry_point = v;
            }
            if let Some(er) = extract_xml_value(trimmed, "EditRate")
                && edit_rate.is_empty()
            {
                edit_rate = er;
            }
        }
    }

    entries
}

/// Parse the ASSETMAP to build a UUID → relative file path map.
fn parse_assetmap(dcp_dir: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let assetmap_path = match find_assetmap(dcp_dir) {
        Some(p) => p,
        None => return map,
    };
    let content = match std::fs::read_to_string(&assetmap_path) {
        Ok(c) => c,
        Err(_) => return map,
    };

    let mut in_asset = false;
    let mut current_id = String::new();
    let mut current_path = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<Asset>") || trimmed.starts_with("<Asset ") {
            in_asset = true;
            current_id.clear();
            current_path.clear();
        } else if trimmed == "</Asset>" {
            if in_asset && !current_id.is_empty() && !current_path.is_empty() {
                map.insert(current_id.clone(), current_path.clone());
            }
            in_asset = false;
        } else if in_asset {
            if let Some(id) = extract_xml_value(trimmed, "Id") {
                current_id = id.replace("urn:uuid:", "");
            }
            if let Some(path) = extract_xml_value(trimmed, "Path") {
                current_path = path;
            }
        }
    }

    map
}

/// One composition in a multi-CPL package. Unlike `--versions` (which shares one
/// essence set across CPLs), each composition here has its OWN picture/audio/
/// subtitle, written as its own CPL, over a single shared PKL/ASSETMAP.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionSpec {
    /// CPL title / ContentTitleText. Required, non-empty, unique in the manifest.
    pub title: String,
    /// Directory of J2K codestreams for this composition's picture.
    pub j2k_dir: PathBuf,
    /// Sound track (WAV) for this composition.
    #[serde(default)]
    pub audio: Option<PathBuf>,
    /// Subtitle track (SRT or supplied SMPTE timed-text XML).
    #[serde(default)]
    pub subtitle: Option<PathBuf>,
    /// Subtitle language code; falls back to the base `--subtitle-language`.
    #[serde(default)]
    pub subtitle_language: Option<String>,
    /// Content kind abbreviation (FTR, TLR, ...); falls back to the base `--kind`.
    #[serde(default)]
    pub kind: Option<String>,
}

/// Parse and validate a compositions manifest (JSON array of [`CompositionSpec`]).
pub fn load_compositions(path: &Path) -> Result<Vec<CompositionSpec>, String> {
    let json = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read compositions manifest {}: {e}", path.display()))?;
    let comps: Vec<CompositionSpec> = serde_json::from_str(&json)
        .map_err(|e| format!("bad compositions manifest {}: {e}", path.display()))?;
    validate_compositions(&comps)?;
    Ok(comps)
}

/// Fail loud on an empty manifest, duplicate/empty titles, missing j2k dir or
/// referenced files, or an unknown content kind.
fn validate_compositions(comps: &[CompositionSpec]) -> Result<(), String> {
    if comps.is_empty() {
        return Err("compositions manifest is empty".into());
    }
    let mut seen = std::collections::HashSet::new();
    for c in comps {
        if c.title.trim().is_empty() {
            return Err("composition title must not be empty".into());
        }
        if !seen.insert(c.title.as_str()) {
            return Err(format!("duplicate composition title: {}", c.title));
        }
        if !c.j2k_dir.is_dir() {
            return Err(format!(
                "j2k_dir not found for composition '{}': {}",
                c.title,
                c.j2k_dir.display()
            ));
        }
        if let Some(p) = c.audio.as_ref().filter(|p| !p.exists()) {
            return Err(format!(
                "audio not found for composition '{}': {}",
                c.title,
                p.display()
            ));
        }
        if let Some(p) = c.subtitle.as_ref().filter(|p| !p.exists()) {
            return Err(format!(
                "subtitle not found for composition '{}': {}",
                c.title,
                p.display()
            ));
        }
        if let Some(k) = c.kind.as_ref()
            && ContentType::from_abbrev(k).is_none()
        {
            return Err(format!(
                "unknown content kind '{k}' for composition '{}'",
                c.title
            ));
        }
    }
    Ok(())
}

/// Build a multi-composition package: one CPL per composition, each with its own
/// picture/audio/subtitle, over one shared PKL/ASSETMAP. Called only when
/// `create --compositions` is set. Reuses the shared wrap primitives (mxf_wrap,
/// versions helpers) rather than duplicating the pipeline; single reel per
/// composition (no reel splitting / 3D / atmos here).
pub fn create_multi_composition(config: &DcpConfig, comps: &[CompositionSpec]) -> i32 {
    use crate::encrypt::KeyType;
    use crate::versions::{cleanup, key_file_path, mint_key, wrap_sound_reel};

    if let Err(e) = validate_compositions(comps) {
        tracing::error!("{e}");
        return -1;
    }
    // prove the signer works before anything is written, so a bad one cannot
    // leave a half-signed package behind
    if let Some(signer) = config.signer.as_ref()
        && let Err(e) = signer.check_usable()
    {
        tracing::error!("unusable signer: {e}");
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

    let mut pkl_entries: Vec<crate::pkl::PklEntry> = Vec::new();
    let mut am_entries: Vec<crate::assetmap::AssetMapEntry> = Vec::new();
    let mut temps: Vec<PathBuf> = Vec::new();
    // (cpl_id, title, bundle keys) for per-CPL key files
    let mut cpl_bundles: Vec<(String, String, Vec<ContentKey>)> = Vec::new();

    for comp in comps {
        // each composition has its own picture, so its own coded raster
        let geometry = match crate::cpl::picture_geometry(
            &comp.j2k_dir,
            config.container_width,
            config.container_height,
        ) {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("composition '{}': {e}", comp.title);
                cleanup(&temps);
                return -1;
            }
        };
        let frames = collect_frames(&comp.j2k_dir);
        if frames.is_empty() {
            tracing::error!(
                "composition '{}' j2k_dir has no codestreams: {}",
                comp.title,
                comp.j2k_dir.display()
            );
            cleanup(&temps);
            return -1;
        }
        let total = frames.len() as u64;
        // single reel per composition
        let range = plan_reel_ranges(total, fps, 0)[0];
        let mut bundle_keys: Vec<ContentKey> = Vec::new();

        // ── picture ──
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
            frames,
            &picture_path,
            crate::mxf_wrap::MxfType::J2kPicture,
            fps,
            picture_key.as_ref().map(crate::reel::mxf_enc),
            None,
            Some(*picture_uuid.as_bytes()),
        )
        .is_none()
        {
            tracing::error!(
                "Failed to wrap picture MXF for composition '{}'",
                comp.title
            );
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
        let picture_key_id = picture_key.as_ref().map(|k| k.info.key_id.clone());
        if let Some(k) = picture_key {
            bundle_keys.push(k.info);
        }

        // ── audio ──
        let mut sound_id = None;
        let mut sound_key_id = None;
        let mut main_sound = None;
        if let Some(audio) = comp.audio.as_ref() {
            let prepared = match crate::versions::prepare_audio(config, audio, &mut temps) {
                Ok(p) => p,
                Err(()) => {
                    cleanup(&temps);
                    return -1;
                }
            };
            let info = match crate::reel::parse_wav(&prepared) {
                Ok(i) => i,
                Err(e) => {
                    tracing::error!("{e}");
                    cleanup(&temps);
                    return -1;
                }
            };
            if !info.sample_rate.is_multiple_of(fps) {
                tracing::error!(
                    "audio {} Hz is not an integer number of samples per {fps} fps frame",
                    info.sample_rate
                );
                cleanup(&temps);
                return -1;
            }
            let sr = match wrap_sound_reel(
                config,
                &prepared,
                &info,
                range,
                fps,
                &mut pkl_entries,
                &mut am_entries,
                &mut temps,
            ) {
                Ok(sr) => sr,
                Err(()) => {
                    cleanup(&temps);
                    return -1;
                }
            };
            sound_id = Some(sr.uuid);
            sound_key_id = sr.key_id;
            if let Some(k) = sr.key_info {
                bundle_keys.push(k);
            }
            if let Ok(ch) = crate::mxf_wrap::wav_channels(&prepared) {
                let configuration = crate::cpl::main_sound_configuration(
                    ch as u32,
                    ch as u32,
                    config.hi_channel,
                    config.vi_channel,
                );
                let sample_rate = crate::mxf_wrap::wav_sample_rate(&prepared).unwrap_or(48000);
                main_sound = Some(crate::cpl::MainSound {
                    configuration,
                    sample_rate,
                });
            }
        }

        // ── subtitle ──
        let sub_lang = comp
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
        let mut subtitle: Option<crate::versions::WrappedTimedText> = None;
        if let Some(sub) = comp.subtitle.as_ref() {
            let is_xml = sub
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("xml"));
            let wrapped = if is_xml {
                crate::versions::wrap_subtitle_xml(
                    "subtitle",
                    sub,
                    config,
                    fps,
                    &mut pkl_entries,
                    &mut am_entries,
                )
            } else {
                let cues = match crate::subtitle::parse_srt_frames(sub, fps) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("subtitle parse failed for '{}': {e}", comp.title);
                        cleanup(&temps);
                        return -1;
                    }
                };
                crate::versions::wrap_subtitle_cues(
                    "subtitle",
                    &cues,
                    &sub_lang,
                    config,
                    &config.subtitle_opts,
                    fps,
                    None,
                    &mut pkl_entries,
                    &mut am_entries,
                    &mut temps,
                )
            };
            match wrapped {
                Ok(track) => {
                    if let Some(key) = track.key_info.as_ref() {
                        bundle_keys.push(key.clone());
                    }
                    subtitle = Some(track);
                }
                Err(()) => {
                    cleanup(&temps);
                    return -1;
                }
            }
        }
        let subtitle_duration = subtitle.as_ref().map(|t| t.duration).unwrap_or(0);

        // ── CPL ──
        let content_kind = comp
            .kind
            .as_deref()
            .and_then(ContentType::from_abbrev)
            .unwrap_or(config.content_type)
            .as_cpl_kind()
            .to_string();
        let reel = crate::cpl::CplReel {
            reel_id: uuid::Uuid::new_v4().to_string(),
            picture_id: picture_uuid.to_string(),
            picture_width: geometry.stored_width,
            picture_height: geometry.stored_height,
            picture_active_width: geometry.active_width,
            picture_active_height: geometry.active_height,
            picture_edit_rate_num: fps,
            picture_edit_rate_den: 1,
            picture_duration: total,
            picture_entry_point: 0,
            picture_key_id,
            sound_id,
            sound_edit_rate_num: fps,
            sound_edit_rate_den: 1,
            sound_duration: total,
            sound_entry_point: 0,
            sound_key_id,
            subtitle_id: subtitle.as_ref().map(|t| t.id.clone()),
            subtitle_edit_rate_num: fps,
            subtitle_edit_rate_den: 1,
            subtitle_duration,
            subtitle_entry_point: 0,
            subtitle_language: (subtitle_duration > 0).then(|| sub_lang.clone()),
            subtitle_hash: subtitle.as_ref().map(|t| t.hash.clone()),
            subtitle_key_id: subtitle.as_ref().and_then(|t| t.key_id.clone()),
            stereoscopic: false,
            aux_data: None,
            markers: crate::markers::default_markers(total),
            ..Default::default()
        };
        let cpl_uuid = uuid::Uuid::new_v4().to_string();
        let cpl_path = config.output_dir.join(format!("CPL_{cpl_uuid}.xml"));
        let cpl_config = crate::cpl::CplConfig {
            title: comp.title.clone(),
            content_kind,
            reels: vec![reel],
            standard: config.standard,
            main_sound,
            ..Default::default()
        };
        if crate::cpl::generate_cpl(&cpl_config, &cpl_uuid, &cpl_path) != 0 {
            tracing::error!("Failed to generate CPL for composition '{}'", comp.title);
            cleanup(&temps);
            return -1;
        }
        // before the PKL entry below hashes it
        if !crate::package_signature::sign_if_configured(
            config.signer.as_ref(),
            &cpl_path,
            &format!("CPL for composition '{}'", comp.title),
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
        cpl_bundles.push((cpl_uuid, comp.title.clone(), bundle_keys));
    }

    // ── one PKL, one ASSETMAP over every asset ──
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_path = config.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    if crate::pkl::generate_pkl(&pkl_entries, &pkl_uuid, config.standard, None, &pkl_path) != 0 {
        tracing::error!("Failed to generate PKL");
        cleanup(&temps);
        return -1;
    }
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
        "Multi-composition DCP created ({} CPLs): {}",
        cpl_bundles.len(),
        config.output_dir.display()
    );
    0
}

// a picture mxf is an asset too, and reading one whole to look for a cpl tag takes gigabytes
fn is_xml_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
}

fn find_assetmap(dir: &Path) -> Option<PathBuf> {
    for name in &["ASSETMAP", "ASSETMAP.xml"] {
        let path = dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

pub(crate) fn extract_xml_value(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start_pos = text.find(&open)?;
    let after_open = &text[start_pos + open.len()..];
    let content_start = after_open.find('>')?;
    let content = &after_open[content_start + 1..];
    let end_pos = content.find(&close)?;
    let value = content[..end_pos].trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

/// `extract_xml_value` matching the element's local name, so a prefixed
/// `<tt:Language>` or `<axd:DataType>` reads the same as a bare one. Closed
/// captions and aux data sit in their own namespace, and so do their children
/// that the namespace declares.
fn extract_local_xml_value(text: &str, tag: &str) -> Option<String> {
    if let Some(value) = extract_xml_value(text, tag) {
        return Some(value);
    }
    let colon = text.find(&format!(":{tag}>"))?;
    let open = text[..colon].rfind('<')?;
    extract_xml_value(text, &text[open + 1..colon + 1 + tag.len()])
}

/// A SMPTE closed caption is `ClosedCaption` in the ST 429-12 namespace, but
/// Interop and older writers name the same track MainClosedCaption.
fn is_closed_caption(trimmed: &str, matches: fn(&str, &str) -> bool) -> bool {
    matches(trimmed, "ClosedCaption") || matches(trimmed, "MainClosedCaption")
}

/// Whether a trimmed CPL line opens `element`, with or without a namespace
/// prefix.
fn opens_local_element(trimmed: &str, element: &str) -> bool {
    let Some(after) = trimmed.strip_prefix('<') else {
        return false;
    };
    let name = after
        .split([' ', '\t', '>', '/'])
        .next()
        .unwrap_or_default();
    local_name(name) == element
}

/// Whether a trimmed CPL line closes `element`, with or without a prefix.
fn closes_local_element(trimmed: &str, element: &str) -> bool {
    let Some(after) = trimmed.strip_prefix("</") else {
        return false;
    };
    local_name(after.trim_end_matches('>')) == element
}

fn local_name(qualified: &str) -> &str {
    qualified.rsplit(':').next().unwrap_or(qualified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_cpls_reads_only_xml_assets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("CPL_abc.xml"),
            r#"<?xml version="1.0"?><CompositionPlaylist><ContentTitleText>Film</ContentTitleText></CompositionPlaylist>"#,
        )
        .unwrap();
        // essence that happens to carry the tag must never be read for it
        std::fs::write(
            dir.path().join("picture.mxf"),
            b"\x06\x0e\x2bCompositionPlaylist",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            r#"<?xml version="1.0"?>
<AssetMap>
  <AssetList>
    <Asset>
      <Id>urn:uuid:1111</Id>
      <ChunkList><Chunk><Path>CPL_abc.xml</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:2222</Id>
      <ChunkList><Chunk><Path>picture.mxf</Path></Chunk></ChunkList>
    </Asset>
  </AssetList>
</AssetMap>"#,
        )
        .unwrap();

        let cpls = list_cpls(dir.path());
        assert_eq!(cpls.len(), 1, "{cpls:?}");
        assert_eq!(cpls[0].id, "1111");
        assert_eq!(cpls[0].content_title, "Film");
    }
}
