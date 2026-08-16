use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Version File DCP configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VfConfig {
    pub ov_dir: PathBuf,
    pub vf_dir: PathBuf,
    pub title: String,
    pub replacement_reels: Vec<ReplacementReel>,
    /// Language code for wrapped subtitle tracks (default "en").
    pub subtitle_language: String,
    pub signer: Option<crate::package_signature::PackageSigner>,
}

/// A reel in the VF that replaces one or more OV essence tracks. A track is
/// raw essence (J2K frames for picture, WAV for sound, SRT/SMPTE XML for
/// subtitle) that gets wrapped, or an already-wrapped `.mxf`. Reels with no
/// replacement are referenced from the OV.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReplacementReel {
    pub reel_number: u32,
    pub picture: Option<PathBuf>,
    pub sound: Option<PathBuf>,
    /// Subtitle to add or replace on this reel (SRT converted, SMPTE XML wrapped
    /// unchanged). Both --add-subtitle and --replace-subtitle land here.
    pub subtitle: Option<PathBuf>,
    /// Closed caption to add or replace on this reel. Same input formats as
    /// `subtitle`; emitted under the MainClosedCaption CPL role.
    #[serde(default)]
    pub ccap: Option<PathBuf>,
}

/// A new MXF that physically ships in the VF (registered in PKL + ASSETMAP).
struct NewAsset {
    id: String,
    filename: String,
    hash: String,
    size: u64,
    duration: u64,
}

/// Create a Version File DCP that references the Original Version.
///
/// Unchanged reels reference the OV's real asset ids; the VF ships only the new
/// MXFs plus its own CPL/PKL/ASSETMAP. Replaced tracks are wrapped (or copied if
/// already MXF) and registered under their real asset id in all three files.
pub fn create_vf(config: &VfConfig) -> i32 {
    if !config.ov_dir.exists() {
        tracing::error!("OV directory not found: {}", config.ov_dir.display());
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

    // Read the OV CPL to get reel structure and identity.
    let ov_cpls = crate::multi_cpl::list_cpls(&config.ov_dir);
    let Some(ov_cpl) = ov_cpls.first() else {
        tracing::error!("No CPL found in OV directory");
        return -1;
    };
    let ov_cpl_path = config.ov_dir.join(&ov_cpl.file_path);
    let ov_timeline = crate::multi_cpl::get_timeline(&ov_cpl_path);
    if ov_timeline.is_empty() {
        tracing::error!("OV CPL has no reels");
        return -1;
    }

    // Fail loud if a replacement targets a reel the OV doesn't have.
    for rep in &config.replacement_reels {
        if !ov_timeline.iter().any(|e| e.reel_number == rep.reel_number) {
            tracing::error!(
                "replacement targets reel {} which the OV does not have",
                rep.reel_number
            );
            return -1;
        }
    }

    // A VF that replaces nothing is just a copy: refuse it.
    if !config.replacement_reels.iter().any(|r| {
        r.picture.is_some() || r.sound.is_some() || r.subtitle.is_some() || r.ccap.is_some()
    }) {
        tracing::error!("no replacement essence supplied; nothing to replace");
        return -1;
    }

    let sub_lang = if config.subtitle_language.is_empty() {
        "en"
    } else {
        &config.subtitle_language
    };

    if let Err(e) = std::fs::create_dir_all(&config.vf_dir) {
        tracing::error!("Failed to create VF directory: {e}");
        return -1;
    }

    let ov_cpl_content = std::fs::read_to_string(&ov_cpl_path).unwrap_or_default();
    let standard = if ov_cpl_content.contains("digicine.com") {
        crate::Standard::Interop
    } else {
        crate::Standard::Smpte
    };
    // vf inherits the ov's picture dimensions (reel coherence keeps them uniform).
    let (pic_w, pic_h) = parse_screen_aspect(&ov_cpl_content);
    // keep the OV's masking: without this a VF over a letterboxed OV would
    // declare the whole raster active
    let (active_w, active_h) =
        crate::cpl::active_area_from_cpl(&ov_cpl_content).unwrap_or((pic_w, pic_h));

    let mut cpl_reels: Vec<crate::cpl::CplReel> = Vec::new();
    let mut new_assets: Vec<NewAsset> = Vec::new();

    for entry in &ov_timeline {
        let rep = config
            .replacement_reels
            .iter()
            .find(|r| r.reel_number == entry.reel_number);
        let (edit_num, edit_den) = parse_edit_rate(&entry.edit_rate);

        // Picture: always present in a DCP reel.
        let (picture_id, picture_duration) = match rep.and_then(|r| r.picture.as_ref()) {
            Some(input) => {
                let Some(a) = prepare_asset(
                    input,
                    "picture",
                    crate::mxf_wrap::MxfType::J2kPicture,
                    edit_num,
                    entry.duration_frames,
                    &config.vf_dir,
                ) else {
                    return -1;
                };
                let out = (a.id.clone(), a.duration);
                new_assets.push(a);
                out
            }
            None => (entry.picture_asset_id.clone(), entry.duration_frames),
        };

        // Sound: from a replacement, or referenced from the OV, or absent.
        let sound = match rep.and_then(|r| r.sound.as_ref()) {
            Some(input) => {
                let Some(a) = prepare_asset(
                    input,
                    "sound",
                    crate::mxf_wrap::MxfType::PcmAudio,
                    edit_num,
                    entry.duration_frames,
                    &config.vf_dir,
                ) else {
                    return -1;
                };
                let out = Some((a.id.clone(), a.duration));
                new_assets.push(a);
                out
            }
            None if !entry.sound_asset_id.is_empty() => {
                Some((entry.sound_asset_id.clone(), entry.duration_frames))
            }
            None => None,
        };

        // Subtitle: a new track added or replaced on this reel. Unchanged reels
        // keep no subtitle here (the VF ships only what it replaces).
        let subtitle = match rep.and_then(|r| r.subtitle.as_ref()) {
            Some(input) => {
                let Some(a) = prepare_timed_text(
                    input,
                    "subtitle",
                    sub_lang,
                    edit_num,
                    entry.duration_frames,
                    &config.vf_dir,
                ) else {
                    return -1;
                };
                let out = Some((a.id.clone(), a.duration));
                new_assets.push(a);
                out
            }
            None => None,
        };

        // Closed caption: same handling, emitted under the MainClosedCaption role.
        let ccap = match rep.and_then(|r| r.ccap.as_ref()) {
            Some(input) => {
                let Some(a) = prepare_timed_text(
                    input,
                    "ccap",
                    sub_lang,
                    edit_num,
                    entry.duration_frames,
                    &config.vf_dir,
                ) else {
                    return -1;
                };
                let out = Some((a.id.clone(), a.duration));
                new_assets.push(a);
                out
            }
            None => None,
        };

        cpl_reels.push(crate::cpl::CplReel {
            reel_id: uuid::Uuid::new_v4().to_string(),
            picture_id,
            picture_width: pic_w,
            picture_height: pic_h,
            picture_active_width: active_w,
            picture_active_height: active_h,
            picture_edit_rate_num: edit_num,
            picture_edit_rate_den: edit_den,
            picture_duration,
            picture_entry_point: 0,
            picture_key_id: None,
            sound_id: sound.as_ref().map(|s| s.0.clone()),
            sound_edit_rate_num: edit_num,
            sound_edit_rate_den: edit_den,
            sound_duration: sound.as_ref().map(|s| s.1).unwrap_or(0),
            sound_entry_point: 0,
            sound_key_id: None,
            subtitle_id: subtitle.as_ref().map(|s| s.0.clone()),
            subtitle_edit_rate_num: if subtitle.is_some() { edit_num } else { 0 },
            subtitle_edit_rate_den: if subtitle.is_some() { edit_den } else { 0 },
            subtitle_duration: subtitle.as_ref().map(|s| s.1).unwrap_or(0),
            subtitle_entry_point: 0,
            subtitle_language: subtitle.as_ref().map(|_| sub_lang.to_string()),
            ccap_id: ccap.as_ref().map(|c| c.0.clone()),
            ccap_edit_rate_num: if ccap.is_some() { edit_num } else { 0 },
            ccap_edit_rate_den: if ccap.is_some() { edit_den } else { 0 },
            ccap_duration: ccap.as_ref().map(|c| c.1).unwrap_or(0),
            ccap_entry_point: 0,
            ccap_language: ccap.as_ref().map(|_| sub_lang.to_string()),
            stereoscopic: false,
            aux_data: None,
            markers: Vec::new(),
            ..Default::default()
        });
    }
    crate::cpl::apply_default_markers(&mut cpl_reels);

    let title = if config.title.is_empty() {
        format!("{}_VF", ov_cpl.content_title)
    } else {
        config.title.clone()
    };
    let content_kind = if ov_cpl.content_kind.is_empty() {
        "feature".to_string()
    } else {
        ov_cpl.content_kind.clone()
    };

    // ── Write CPL via the shared postkit writer, then mark it supplemental ──
    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let cpl_path = config.vf_dir.join(format!("CPL_{cpl_uuid}.xml"));
    let cpl_config = crate::cpl::CplConfig {
        title,
        content_kind,
        rating: String::new(),
        reels: cpl_reels,
        standard,
        main_sound: None,
        sign_language: None,
        ..Default::default()
    };
    if crate::cpl::generate_cpl(&cpl_config, &cpl_uuid, &cpl_path) != 0 {
        tracing::error!("Failed to generate VF CPL");
        return -1;
    }

    // dcpdoctor detects a supplemental DCP by an <OriginalPackagingList> /
    // <OriginalFileName> / <OPL> marker in the CPL. Reference the OV package so
    // validating the VF alone yields the supplemental warning, not a hard
    // cross-ref error.
    let ov_ref =
        find_ov_pkl_id(&config.vf_dir, &config.ov_dir).unwrap_or_else(|| ov_cpl.id.clone());
    let marker = format!("  <OriginalPackagingList>urn:uuid:{ov_ref}</OriginalPackagingList>\n");
    match std::fs::read_to_string(&cpl_path) {
        Ok(xml) => {
            let marked = xml.replace(
                "</CompositionPlaylist>",
                &format!("{marker}</CompositionPlaylist>"),
            );
            if let Err(e) = std::fs::write(&cpl_path, marked) {
                tracing::error!("Failed to mark VF CPL supplemental: {e}");
                return -1;
            }
        }
        Err(e) => {
            tracing::error!("Failed to read VF CPL back: {e}");
            return -1;
        }
    }
    // after the supplemental marker rewrite, which would otherwise invalidate
    // the signature, and before the PKL hashes the file
    if !crate::package_signature::sign_if_configured(config.signer.as_ref(), &cpl_path, "VF CPL") {
        return -1;
    }

    // ── PKL: the CPL plus every new MXF ────────────────────────────────────
    let pkl_uuid = uuid::Uuid::new_v4().to_string();
    let cpl_hash = crate::hash::hash_file(&cpl_path).unwrap_or_default();
    let cpl_size = std::fs::metadata(&cpl_path).map(|m| m.len()).unwrap_or(0);
    let mut pkl_entries = vec![crate::pkl::PklEntry {
        id: cpl_uuid.clone(),
        asset_type: "text/xml".into(),
        file: cpl_path.clone(),
        hash: cpl_hash,
        size: cpl_size,
    }];
    for a in &new_assets {
        pkl_entries.push(crate::pkl::PklEntry {
            id: a.id.clone(),
            asset_type: "application/mxf".into(),
            file: config.vf_dir.join(&a.filename),
            hash: a.hash.clone(),
            size: a.size,
        });
    }
    let pkl_path = config.vf_dir.join(format!("PKL_{pkl_uuid}.xml"));
    if crate::pkl::generate_pkl(&pkl_entries, &pkl_uuid, standard, None, &pkl_path) != 0 {
        tracing::error!("Failed to generate VF PKL");
        return -1;
    }
    if !crate::package_signature::sign_if_configured(config.signer.as_ref(), &pkl_path, "VF PKL") {
        return -1;
    }

    // ── ASSETMAP: PKL, CPL, and every new MXF ──────────────────────────────
    let mut am_entries = vec![
        crate::assetmap::AssetMapEntry {
            id: pkl_uuid,
            path: file_name(&pkl_path),
            packing_list: true,
        },
        crate::assetmap::AssetMapEntry {
            id: cpl_uuid,
            path: file_name(&cpl_path),
            packing_list: false,
        },
    ];
    for a in &new_assets {
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: a.id.clone(),
            path: a.filename.clone(),
            packing_list: false,
        });
    }
    if crate::assetmap::generate_assetmap(&am_entries, &config.vf_dir, standard, None) != 0 {
        tracing::error!("Failed to generate VF ASSETMAP");
        return -1;
    }

    tracing::info!(
        "Created VF DCP at {} ({} new asset(s))",
        config.vf_dir.display(),
        new_assets.len()
    );
    0
}

/// Read the AssetUUID an already-wrapped MXF carries, so a copied track file is
/// listed in the CPL/PKL/ASSETMAP under the id it actually holds.
fn embedded_asset_uuid(path: &Path, mxf_type: crate::mxf_wrap::MxfType) -> Option<uuid::Uuid> {
    let file = path.to_string_lossy().to_string();
    let info = match mxf_type {
        crate::mxf_wrap::MxfType::J2kPicture => {
            let mut reader = asdcplib::jp2k::MxfReader::new();
            reader.open_read(&file).ok()?;
            reader.writer_info().ok()?
        }
        crate::mxf_wrap::MxfType::PcmAudio => {
            let mut reader = asdcplib::pcm::MxfReader::new();
            reader.open_read(&file).ok()?;
            reader.writer_info().ok()?
        }
        crate::mxf_wrap::MxfType::TimedText => {
            let mut reader = asdcplib::timed_text::MxfReader::new();
            reader.open_read(&file).ok()?;
            reader.writer_info().ok()?
        }
        crate::mxf_wrap::MxfType::Atmos => {
            let mut reader = asdcplib::atmos::MxfReader::new();
            reader.open_read(&file).ok()?;
            reader.writer_info().ok()?
        }
    };
    Some(uuid::Uuid::from_bytes(info.asset_uuid))
}

/// Wrap raw essence (or copy an already-wrapped MXF) into the VF directory and
/// return its real asset id, hash, and size. `None` on any failure (logged).
fn prepare_asset(
    input: &Path,
    prefix: &str,
    mxf_type: crate::mxf_wrap::MxfType,
    fps: u32,
    fallback_duration: u64,
    vf_dir: &Path,
) -> Option<NewAsset> {
    if !input.exists() {
        tracing::error!("replacement essence not found: {}", input.display());
        return None;
    }

    let is_mxf = input.is_file()
        && input
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("mxf"));

    let (id, filename, duration) = if is_mxf {
        // Already wrapped: copy verbatim under the id the MXF itself carries,
        // so the CPL/PKL/ASSETMAP entries match what the file holds.
        let Some(id) = embedded_asset_uuid(input, mxf_type) else {
            tracing::error!("cannot read the asset id of MXF {}", input.display());
            return None;
        };
        let filename = format!("{prefix}_{id}.mxf");
        let id = id.to_string();
        if let Err(e) = std::fs::copy(input, vf_dir.join(&filename)) {
            tracing::error!("Failed to copy MXF {}: {e}", input.display());
            return None;
        }
        (id, filename, fallback_duration)
    } else {
        // Raw essence: mint the id, name the file with it and wrap the MXF under it.
        let id = uuid::Uuid::new_v4();
        let filename = format!("{prefix}_{id}.mxf");
        let wrap_config = crate::mxf_wrap::MxfWrapConfig {
            input_path: input.to_path_buf(),
            output_mxf: vf_dir.join(&filename),
            mxf_type,
            frame_rate: fps,
            encryption: None,
            mca_config: None,
            asset_uuid: Some(*id.as_bytes()),
        };
        let tf = crate::mxf_wrap::wrap_mxf_result(&wrap_config)?;
        let duration = if tf.duration > 0 {
            tf.duration
        } else {
            fallback_duration
        };
        (tf.uuid, filename, duration)
    };

    let path = vf_dir.join(&filename);
    let hash = match crate::hash::hash_file(&path) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to hash {}: {e}", path.display());
            return None;
        }
    };
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Some(NewAsset {
        id,
        filename,
        hash,
        size,
        duration,
    })
}

/// Wrap a timed-text track (subtitle or closed caption) into the VF: SRT is
/// converted to ST 428-7 DCST first, supplied SMPTE XML is wrapped unchanged.
/// `prefix` names the loose/wrapped files ("subtitle" or "ccap"). Returns the
/// wrapped track's real asset id, hash, size, and duration. `None` on failure.
fn prepare_timed_text(
    input: &Path,
    prefix: &str,
    lang: &str,
    fps: u32,
    fallback_duration: u64,
    vf_dir: &Path,
) -> Option<NewAsset> {
    if !input.exists() {
        tracing::error!("{prefix} not found: {}", input.display());
        return None;
    }
    let is_xml = input
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("xml"));

    // SRT gets converted to a temp DCST; supplied XML is wrapped as-is.
    let mut temp_dcst: Option<PathBuf> = None;
    let dcst_path = if is_xml {
        input.to_path_buf()
    } else {
        let tmp = vf_dir.join(format!("{prefix}_{}.xml", uuid::Uuid::new_v4()));
        if let Err(e) = crate::subtitle::srt_to_shifted_dcst(input, 0, lang, fps, &tmp) {
            tracing::error!("{prefix} conversion failed: {e}");
            return None;
        }
        temp_dcst = Some(tmp.clone());
        tmp
    };

    let id = uuid::Uuid::new_v4();
    let filename = format!("{prefix}_{id}.mxf");
    let wrap_config = crate::mxf_wrap::MxfWrapConfig {
        input_path: dcst_path,
        output_mxf: vf_dir.join(&filename),
        mxf_type: crate::mxf_wrap::MxfType::TimedText,
        frame_rate: fps,
        encryption: None,
        mca_config: None,
        asset_uuid: Some(*id.as_bytes()),
    };
    let track = crate::mxf_wrap::wrap_mxf_result(&wrap_config);
    if let Some(tmp) = temp_dcst {
        let _ = std::fs::remove_file(tmp);
    }
    let track = track?;
    let path = vf_dir.join(&filename);
    let hash = match crate::hash::hash_file(&path) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to hash {}: {e}", path.display());
            return None;
        }
    };
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let duration = if track.duration > 0 {
        track.duration
    } else {
        fallback_duration
    };
    Some(NewAsset {
        id: track.uuid,
        filename,
        hash,
        size,
        duration,
    })
}

/// Parse a CPL EditRate string like "24 1" into (num, den), defaulting to 24/1.
fn parse_edit_rate(s: &str) -> (u32, u32) {
    let mut it = s.split_whitespace();
    let num = it.next().and_then(|v| v.parse().ok()).unwrap_or(24);
    let den = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    (num, den)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Picture dimensions from the OV CPL's ScreenAspectRatio. SMPTE carries a
/// "w h" pair; Interop carries a decimal, from which we recover w at 1080p.
fn parse_screen_aspect(cpl: &str) -> (u32, u32) {
    let inner = cpl
        .split_once("<ScreenAspectRatio>")
        .and_then(|(_, r)| r.split_once("</ScreenAspectRatio>"))
        .map(|(v, _)| v.trim());
    match inner {
        Some(v) if v.contains(char::is_whitespace) => {
            let mut it = v.split_whitespace();
            let w = it.next().and_then(|x| x.parse().ok()).unwrap_or(2048);
            let h = it.next().and_then(|x| x.parse().ok()).unwrap_or(1080);
            (w, h)
        }
        Some(v) => match v.parse::<f64>() {
            Ok(r) if r > 0.0 => ((r * 1080.0).round() as u32, 1080),
            _ => (2048, 1080),
        },
        None => (2048, 1080),
    }
}

/// Find the OV's PKL id (the id to record as the original package). Skips any
/// PKL already written into `vf_dir`.
fn find_ov_pkl_id(vf_dir: &Path, ov_dir: &Path) -> Option<String> {
    for entry in std::fs::read_dir(ov_dir).ok()?.flatten() {
        let path = entry.path();
        if path.parent() == Some(vf_dir) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.starts_with("PKL") && name.ends_with(".xml") {
            let content = std::fs::read_to_string(&path).ok()?;
            if let Some(pos) = content.find("urn:uuid:") {
                let rest = &content[pos + "urn:uuid:".len()..];
                let id: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
                    .collect();
                if !id.is_empty() {
                    return Some(id);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIC_ID: &str = "11111111-1111-1111-1111-111111111111";
    const SND_ID: &str = "22222222-2222-2222-2222-222222222222";
    const CPL_ID: &str = "33333333-3333-3333-3333-333333333333";
    const PKL_ID: &str = "44444444-4444-4444-4444-444444444444";

    /// Write a minimal SMPTE OV DCP (ASSETMAP + CPL + PKL) with one reel.
    fn write_ov(dir: &Path) {
        let cpl = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:{CPL_ID}</Id>
  <ContentTitleText>OV Movie</ContentTitleText>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <ContentKind>feature</ContentKind>
  <ReelList>
    <Reel>
      <Id>urn:uuid:55555555-5555-5555-5555-555555555555</Id>
      <AssetList>
        <MainPicture>
          <Id>urn:uuid:{PIC_ID}</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>48</IntrinsicDuration>
          <Duration>48</Duration>
        </MainPicture>
        <MainSound>
          <Id>urn:uuid:{SND_ID}</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>48</IntrinsicDuration>
          <Duration>48</Duration>
        </MainSound>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>
"#
        );
        std::fs::write(dir.join(format!("CPL_{CPL_ID}.xml")), cpl).unwrap();
        std::fs::write(
            dir.join(format!("PKL_{PKL_ID}.xml")),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">
  <Id>urn:uuid:{PKL_ID}</Id>
</PackingList>
"#
            ),
        )
        .unwrap();
        let am = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:66666666-6666-6666-6666-666666666666</Id>
  <AssetList>
    <Asset>
      <Id>urn:uuid:{PKL_ID}</Id>
      <PackingList>true</PackingList>
      <ChunkList><Chunk><Path>PKL_{PKL_ID}.xml</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:{CPL_ID}</Id>
      <ChunkList><Chunk><Path>CPL_{CPL_ID}.xml</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:{PIC_ID}</Id>
      <ChunkList><Chunk><Path>picture.mxf</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:{SND_ID}</Id>
      <ChunkList><Chunk><Path>sound.mxf</Path></Chunk></ChunkList>
    </Asset>
  </AssetList>
</AssetMap>
"#
        );
        std::fs::write(dir.join("ASSETMAP.xml"), am).unwrap();
    }

    /// Wrap a second of stereo silence into a real sound MXF carrying `asset_id`.
    fn write_sound_mxf(dir: &Path, asset_id: uuid::Uuid) -> PathBuf {
        let sample_rate = 48_000u32;
        let channels = 2u16;
        let bits = 24u16;
        let block_align = (bits / 8) * channels;
        let data_len = sample_rate as u64 * block_align as u64;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
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
        w.extend_from_slice(&(data_len as u32).to_le_bytes());
        w.resize(w.len() + data_len as usize, 0);
        let wav = dir.join("replacement.wav");
        std::fs::write(&wav, &w).unwrap();

        let mxf = dir.join("new_sound.mxf");
        crate::mxf_wrap::wrap_mxf_files(
            vec![wav],
            &mxf,
            crate::mxf_wrap::MxfType::PcmAudio,
            24,
            None,
            None,
            Some(*asset_id.as_bytes()),
        )
        .expect("wrap replacement sound MXF");
        mxf
    }

    /// Replacing a reel's sound must put the new MXF's real (registered) id in
    /// the CPL AND declare it in PKL + ASSETMAP, while unchanged reels stay refs.
    #[test]
    fn replaced_reel_registers_new_asset_everywhere() {
        let tmp = tempfile::tempdir().unwrap();
        let ov = tmp.path().join("ov");
        let vf = tmp.path().join("vf");
        std::fs::create_dir_all(&ov).unwrap();
        write_ov(&ov);

        // A pre-wrapped replacement ships under the id it already carries, so the
        // fixture has to be a real MXF with a known AssetUUID.
        let embedded_id = uuid::Uuid::new_v4();
        let new_snd = write_sound_mxf(tmp.path(), embedded_id);

        let config = VfConfig {
            ov_dir: ov.clone(),
            vf_dir: vf.clone(),
            title: String::new(),
            subtitle_language: String::new(),
            signer: None,
            replacement_reels: vec![ReplacementReel {
                reel_number: 1,
                picture: None,
                sound: Some(new_snd),
                subtitle: None,
                ccap: None,
            }],
        };
        assert_eq!(create_vf(&config), 0);

        // The new sound MXF ships in the VF; its id is embedded in its filename.
        let mxf = std::fs::read_dir(&vf)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.starts_with("sound_") && n.ends_with(".mxf"))
            .expect("new sound MXF present in VF");
        let new_id = mxf
            .trim_start_matches("sound_")
            .trim_end_matches(".mxf")
            .to_string();
        assert_eq!(
            new_id,
            embedded_id.to_string(),
            "the shipped file must be named with the id the MXF itself carries"
        );
        assert!(vf.join(&mxf).exists());

        let cpl_name = std::fs::read_dir(&vf)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.starts_with("CPL_"))
            .unwrap();
        let cpl = std::fs::read_to_string(vf.join(&cpl_name)).unwrap();
        let am = std::fs::read_to_string(vf.join("ASSETMAP.xml")).unwrap();
        let pkl_name = std::fs::read_dir(&vf)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.starts_with("PKL_"))
            .unwrap();
        let pkl = std::fs::read_to_string(vf.join(&pkl_name)).unwrap();

        // Replaced sound: real id in CPL, declared in PKL and ASSETMAP.
        assert!(
            cpl.contains(&format!("urn:uuid:{new_id}")),
            "CPL must reference the new sound's real id"
        );
        assert!(pkl.contains(&new_id), "PKL must declare the new sound");
        assert!(am.contains(&new_id), "ASSETMAP must declare the new sound");

        // Unchanged picture: referenced from the OV, not shipped/declared here.
        assert!(cpl.contains(PIC_ID), "CPL must still reference OV picture");
        assert!(
            !am.contains(PIC_ID),
            "ASSETMAP must not list the unchanged OV picture"
        );
        // Old OV sound id must be gone from the CPL (it was replaced).
        assert!(
            !cpl.contains(SND_ID),
            "replaced OV sound id must not remain in CPL"
        );
        // Supplemental marker so dcpdoctor detects a VF.
        assert!(
            cpl.contains("OriginalPackagingList"),
            "VF CPL must carry a supplemental marker"
        );
    }

    #[test]
    fn fails_when_nothing_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let ov = tmp.path().join("ov");
        std::fs::create_dir_all(&ov).unwrap();
        write_ov(&ov);
        let config = VfConfig {
            ov_dir: ov,
            vf_dir: tmp.path().join("vf"),
            title: String::new(),
            subtitle_language: String::new(),
            signer: None,
            replacement_reels: vec![],
        };
        assert_eq!(create_vf(&config), -1);
    }

    #[test]
    fn fails_on_unknown_reel() {
        let tmp = tempfile::tempdir().unwrap();
        let ov = tmp.path().join("ov");
        std::fs::create_dir_all(&ov).unwrap();
        write_ov(&ov);
        let snd = tmp.path().join("s.mxf");
        std::fs::write(&snd, b"x").unwrap();
        let config = VfConfig {
            ov_dir: ov,
            vf_dir: tmp.path().join("vf"),
            title: String::new(),
            subtitle_language: String::new(),
            signer: None,
            replacement_reels: vec![ReplacementReel {
                reel_number: 9,
                picture: None,
                sound: Some(snd),
                subtitle: None,
                ccap: None,
            }],
        };
        assert_eq!(create_vf(&config), -1);
    }
}
