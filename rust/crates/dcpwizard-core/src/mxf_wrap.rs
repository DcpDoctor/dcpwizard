//! AS-DCP MXF wrapping.
//!
//! Delegates to [`postkit::mxf_wrap`] (asdcplib FFI). This layer keeps
//! dcpwizard's exit-code API and maps [`MxfType`]/[`MxfWrapConfig`] onto
//! postkit's `EssenceType`/`MxfWrapOptions`. Unlike the previous local
//! implementation, postkit reads the real J2K codestream dimensions (rather
//! than hardcoding 2048x1080) and derives timed-text duration from the
//! subtitle timing.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Declared 5.1 channel order for input WAV files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AudioInputOrder {
    /// L, R, C, LFE, Ls, Rs. This is the DCP order.
    #[default]
    Canonical51,
    /// L, R, C, Ls, Rs, LFE. This order is common in source files.
    LrcLsRsLfe,
}

/// MXF essence type. There is no separate DTS:X essence: no public DTS:X
/// DataEssenceCoding UL exists. Since ST 429-18/-19 (2019) DTS:X is delivered as
/// a standard IAB track (ST 2098-2, "DTS:X for IAB"), which is exactly the `Atmos`
/// (IAB / ST 429-18) essence below. Deliver DTS:X auditoriums via `create --atmos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MxfType {
    #[default]
    J2kPicture,
    PcmAudio,
    TimedText,
    Atmos,
}

/// MXF wrapping configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MxfWrapConfig {
    pub input_path: PathBuf,
    pub output_mxf: PathBuf,
    pub mxf_type: MxfType,
    pub frame_rate: u32,
    /// AES-128 content encryption for J2K picture / PCM sound. Not serialized.
    #[serde(skip)]
    pub encryption: Option<postkit::mxf_wrap::MxfEncryption>,
    /// ST 429-12 MCA channel-label config for a PCM wrap (e.g.
    /// `"51(L,R,C,LFE,Ls,Rs)"`). None auto-derives from the probed channel count.
    #[serde(skip)]
    pub mca_config: Option<String>,
    /// The id the MXF carries as its AssetUUID. A caller that already named the
    /// output file or wrote the CPL/PKL/ASSETMAP entry must pass its id here, or
    /// the MXF ends up carrying a different id than the package claims. None
    /// mints a fresh id.
    #[serde(default)]
    pub asset_uuid: Option<[u8; 16]>,
}

/// Collect sorted files from a directory, or treat a single file as one-element list.
fn collect_inputs(path: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if path.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|e| format!("cannot read dir {}: {e}", path.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        if files.is_empty() {
            return Err(format!("no files in {}", path.display()));
        }
        return Ok(files);
    }
    Err(format!("input path not found: {}", path.display()))
}

/// DCP-legal PCM sample rates (SMPTE 428-2 / DCI): 48 kHz and 96 kHz. postkit
/// wraps the real channel count / bit depth / sample rate it reads from the WAV,
/// but happily wraps any rate; a DCP with 44.1 kHz sound is illegal, so reject
/// non-DCP rates here instead of shipping a mislabeled MXF.
const DCP_SAMPLE_RATES: [u32; 2] = [48_000, 96_000];

/// Read the `fmt ` chunk body (channels at +2, sample rate at +4) from a WAV.
/// Reads a bounded prefix since the fmt chunk sits near the file start.
fn wav_fmt(path: &std::path::Path) -> Result<(u16, u32), String> {
    use std::io::Read;
    let mut f =
        std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut buf = vec![0u8; 65536];
    let n = f
        .read(&mut buf)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let d = &buf[..n];
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return Err(format!("{} is not a RIFF/WAVE file", path.display()));
    }
    let mut pos = 12usize;
    while pos + 8 <= d.len() {
        let size = u32::from_le_bytes([d[pos + 4], d[pos + 5], d[pos + 6], d[pos + 7]]) as usize;
        let body = pos + 8;
        if &d[pos..pos + 4] == b"fmt " && body + 8 <= d.len() {
            let channels = u16::from_le_bytes([d[body + 2], d[body + 3]]);
            let sample_rate =
                u32::from_le_bytes([d[body + 4], d[body + 5], d[body + 6], d[body + 7]]);
            return Ok((channels, sample_rate));
        }
        pos = body + size + (size & 1);
    }
    Err(format!("no fmt chunk found in {}", path.display()))
}

/// Probe a WAV's channel count for MCA labelling.
pub fn wav_channels(path: &std::path::Path) -> Result<u16, String> {
    wav_fmt(path).map(|(ch, _)| ch)
}

/// Probe a WAV's sample rate for the CompositionMetadataAsset MainSoundSampleRate.
pub fn wav_sample_rate(path: &std::path::Path) -> Result<u32, String> {
    wav_fmt(path).map(|(_, sr)| sr)
}

/// Expand a six-channel WAV to DCP's 16-channel PCM layout. The first six
/// channels are canonical DCP 5.1 and channels 7 through 16 are silent.
/// Returns false when the source is not 5.1 and was left untouched.
pub fn prepare_51_audio(
    input: &Path,
    output: &Path,
    input_order: AudioInputOrder,
) -> Result<bool, String> {
    let data = std::fs::read(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{} is not a RIFF/WAVE file", input.display()));
    }

    let mut pos = 12usize;
    let mut fmt = None;
    let mut payload = None;
    while pos + 8 <= data.len() {
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if body + size > data.len() {
            return Err(format!("{} has a truncated WAV chunk", input.display()));
        }
        match &data[pos..pos + 4] {
            b"fmt " if size >= 16 => fmt = Some(&data[body..body + size]),
            b"data" => payload = Some(&data[body..body + size]),
            _ => {}
        }
        pos = body + size + (size & 1);
    }
    let Some(fmt) = fmt else {
        return Err(format!("no fmt chunk found in {}", input.display()));
    };
    let Some(payload) = payload else {
        return Err(format!("no data chunk found in {}", input.display()));
    };
    let format = u16::from_le_bytes(fmt[0..2].try_into().unwrap());
    let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap());
    if channels != 6 {
        return Ok(false);
    }
    // ffmpeg writes >2ch pcm as WAVE_FORMAT_EXTENSIBLE (0xFFFE); the real
    // format code is the first two bytes of the SubFormat guid
    let is_pcm = format == 1
        || (format == 0xFFFE
            && fmt.len() >= 26
            && u16::from_le_bytes(fmt[24..26].try_into().unwrap()) == 1);
    if !is_pcm {
        return Err(format!("{} must use PCM WAV samples", input.display()));
    }
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
    let bits = u16::from_le_bytes(fmt[14..16].try_into().unwrap());
    if bits == 0 || !bits.is_multiple_of(8) {
        return Err(format!("{} has unsupported PCM bit depth", input.display()));
    }
    let sample_bytes = (bits / 8) as usize;
    let source_frame_bytes = sample_bytes * 6;
    if payload.len() % source_frame_bytes != 0 {
        return Err(format!("{} has incomplete audio frames", input.display()));
    }

    let order = match input_order {
        AudioInputOrder::Canonical51 => [0, 1, 2, 3, 4, 5],
        AudioInputOrder::LrcLsRsLfe => [0, 1, 2, 5, 3, 4],
    };
    let output_frame_bytes = sample_bytes * 16;
    let mut wav = Vec::with_capacity(44 + payload.len() / source_frame_bytes * output_frame_bytes);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&0u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * output_frame_bytes as u32).to_le_bytes());
    wav.extend_from_slice(&(output_frame_bytes as u16).to_le_bytes());
    wav.extend_from_slice(&bits.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&0u32.to_le_bytes());
    for frame in payload.chunks_exact(source_frame_bytes) {
        for channel in order {
            let start = channel * sample_bytes;
            wav.extend_from_slice(&frame[start..start + sample_bytes]);
        }
        wav.extend(std::iter::repeat_n(0, sample_bytes * 10));
    }
    let data_size = (wav.len() - 44) as u32;
    let riff_size = wav.len() as u32 - 8;
    wav[4..8].copy_from_slice(&riff_size.to_le_bytes());
    wav[40..44].copy_from_slice(&data_size.to_le_bytes());
    std::fs::write(output, wav).map_err(|e| format!("cannot write {}: {e}", output.display()))?;
    Ok(true)
}

/// Build a ST 429-12 MCA config for a sound wrap from the channel count plus
/// optional accessibility (HI/VI) channel indices. The main layout is 2.0/5.1/
/// 7.1 by channel count; HI and VI-N are labelled as standalone channels at the
/// given indices. Returns None when the layout has no asdcplib DCP label.
pub fn build_mca_config(
    channel_count: u32,
    hi_channel: Option<u32>,
    vi_channel: Option<u32>,
) -> Option<String> {
    use postkit::mca::{McaLabel, McaTagSymbol};

    let extra = hi_channel.is_some() as u32 + vi_channel.is_some() as u32;
    let main_count = channel_count.saturating_sub(extra);
    // main layout by channel count; 8 is 7.1 here (postkit's detect_soundfield
    // treats 8 as 5.1+HI+VI, but accessibility tracks are opt-in via the flags).
    let mut sf = match main_count {
        2 => postkit::mca::soundfield_stereo(),
        6 | 16 => postkit::mca::soundfield_51(),
        8 => postkit::mca::soundfield_71(),
        n => postkit::mca::detect_soundfield(n),
    };
    let mut push = |symbol: McaTagSymbol, index: u32| {
        sf.channels.push(McaLabel {
            symbol,
            tag_name: symbol.tag_name().to_string(),
            tag_symbol: symbol.symbol_string().to_string(),
            channel_index: index,
            spoken_language: String::new(),
        });
    };
    if let Some(idx) = hi_channel {
        push(McaTagSymbol::Hi, idx);
    }
    if let Some(idx) = vi_channel {
        push(McaTagSymbol::Vi, idx);
    }
    let mut config = postkit::mca::soundfield_to_mca_config(&sf)?;
    // asdcplib requires a label per physical channel; the silent fill channels
    // after the labeled ones get '-' placeholders
    for _ in (sf.channels.len() as u32)..channel_count {
        config.push_str(",-");
    }
    Some(config)
}

/// Wrap essence into an MXF and return the track file (real embedded asset id,
/// hash, size, duration). `None` on input-collection or wrap failure.
pub fn wrap_mxf_result(config: &MxfWrapConfig) -> Option<postkit::mxf_wrap::MxfTrackFile> {
    let input_files = match collect_inputs(&config.input_path) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("{e}");
            return None;
        }
    };
    wrap_mxf_files(
        input_files,
        &config.output_mxf,
        config.mxf_type,
        config.frame_rate,
        config.encryption.clone(),
        config.mca_config.clone(),
        config.asset_uuid,
    )
}

/// Wrap an explicit, ordered list of essence files (already collected/sorted).
/// Used by reel splitting to wrap a per-reel J2K frame subrange without touching
/// postkit (whose wrapper always consumes every file it is given).
pub fn wrap_mxf_files(
    input_files: Vec<PathBuf>,
    output_mxf: &std::path::Path,
    mxf_type: MxfType,
    frame_rate: u32,
    encryption: Option<postkit::mxf_wrap::MxfEncryption>,
    mca_config: Option<String>,
    asset_uuid: Option<[u8; 16]>,
) -> Option<postkit::mxf_wrap::MxfTrackFile> {
    if input_files.is_empty() {
        tracing::error!("no essence files to wrap into {}", output_mxf.display());
        return None;
    }

    // PCM: reject non-DCP sample rates and derive MCA labels from the channel
    // count when the caller gave no explicit config.
    let mut mca_config = mca_config;
    if mxf_type == MxfType::PcmAudio {
        for f in &input_files {
            let (channels, sr) = match wav_fmt(f) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("{e}");
                    return None;
                }
            };
            if !DCP_SAMPLE_RATES.contains(&sr) {
                tracing::error!(
                    "audio {} is {sr} Hz; DCP requires 48000 or 96000 Hz",
                    f.display()
                );
                return None;
            }
            if mca_config.is_none() {
                mca_config = build_mca_config(channels as u32, None, None);
            }
        }
    }

    let essence_type = match mxf_type {
        MxfType::J2kPicture => postkit::mxf_wrap::EssenceType::J2k,
        MxfType::PcmAudio => postkit::mxf_wrap::EssenceType::Pcm,
        MxfType::TimedText => postkit::mxf_wrap::EssenceType::TimedText,
        MxfType::Atmos => postkit::mxf_wrap::EssenceType::Atmos,
    };

    let fps = if frame_rate == 0 { 24 } else { frame_rate };

    let opts = postkit::mxf_wrap::MxfWrapOptions {
        input_files,
        output: output_mxf.to_path_buf(),
        essence_type,
        standard: postkit::mxf_wrap::MxfStandard::AsDcp,
        fps_num: fps,
        fps_den: 1,
        partition_size: 0,
        encryption,
        mca_config,
        resource_ids: vec![],
        hdr: None,
        asset_uuid,
    };

    let result = postkit::mxf_wrap::mxf_wrap(&opts);
    if result.success {
        tracing::info!("Wrapped {:?} to MXF: {}", mxf_type, output_mxf.display());
        Some(result)
    } else {
        tracing::error!("MXF wrap failed: {}", result.error);
        None
    }
}

/// Wrap a DCST XML plus its ancillary resources (embedded font, bitmap PNGs)
/// into a timed-text MXF. Each `(file, id)` resource is embedded under `id`, so
/// a `urn:uuid` reference in the DCST matches the stored resource. The XML is
/// the first input file; resources follow in order.
pub fn wrap_timed_text_resources(
    dcst: &std::path::Path,
    resources: &[(PathBuf, [u8; 16])],
    output_mxf: &std::path::Path,
    frame_rate: u32,
    asset_uuid: Option<[u8; 16]>,
) -> Option<postkit::mxf_wrap::MxfTrackFile> {
    let mut input_files = vec![dcst.to_path_buf()];
    let mut resource_ids = Vec::new();
    for (path, id) in resources {
        input_files.push(path.clone());
        resource_ids.push(*id);
    }
    let fps = if frame_rate == 0 { 24 } else { frame_rate };
    let opts = postkit::mxf_wrap::MxfWrapOptions {
        input_files,
        output: output_mxf.to_path_buf(),
        essence_type: postkit::mxf_wrap::EssenceType::TimedText,
        standard: postkit::mxf_wrap::MxfStandard::AsDcp,
        fps_num: fps,
        fps_den: 1,
        partition_size: 0,
        encryption: None,
        mca_config: None,
        resource_ids,
        hdr: None,
        asset_uuid,
    };
    let result = postkit::mxf_wrap::mxf_wrap(&opts);
    if result.success {
        Some(result)
    } else {
        tracing::error!("timed-text wrap failed: {}", result.error);
        None
    }
}

/// Wrap essence into an MXF container using postkit's asdcplib wrapper.
pub fn wrap_mxf(config: &MxfWrapConfig) -> i32 {
    if wrap_mxf_result(config).is_some() {
        0
    } else {
        -1
    }
}

/// Wrap a DCI HDR Addendum picture MXF from an ordered J2K frame list, setting
/// the Generic Picture Essence Descriptor's TransferCharacteristic = ST 2084 (PQ)
/// and ColorPrimaries = P3-D65 through asdcplib's `open_write_hdr`. postkit's
/// `mxf_wrap` has no HDR setter, so this replicates its AS-DCP JP2K wrap and adds
/// the HDR ULs directly. Returns the track file or None on failure (logged).
pub fn wrap_j2k_hdr_files(
    input_files: Vec<PathBuf>,
    output_mxf: &std::path::Path,
    frame_rate: u32,
    encryption: Option<postkit::mxf_wrap::MxfEncryption>,
    asset_uuid: Option<[u8; 16]>,
) -> Option<postkit::mxf_wrap::MxfTrackFile> {
    use asdcplib::jp2k::{COLOR_PRIMARIES_P3D65, HdrMetadata, TRANSFER_CHARACTERISTIC_ST2084};

    if input_files.is_empty() {
        tracing::error!("no essence files to wrap into {}", output_mxf.display());
        return None;
    }

    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(input_files.len());
    for f in &input_files {
        match std::fs::read(f) {
            Ok(data) => frames.push(data),
            Err(e) => {
                tracing::error!("failed to read {}: {e}", f.display());
                return None;
            }
        }
    }

    // validate every frame as a DCI JPEG 2000 codestream (matches postkit's wrap)
    let mut header = None;
    for (f, frame) in input_files.iter().zip(&frames) {
        let Some(h) = postkit::j2k::parse_j2k_header(frame) else {
            tracing::error!("invalid JPEG 2000 codestream: {}", f.display());
            return None;
        };
        if let Err(error) = postkit::j2k::validate_dci_header(&h) {
            tracing::error!("invalid DCI JPEG 2000 codestream: {error}: {}", f.display());
            return None;
        }
        if header.is_none() {
            header = Some(h);
        }
    }
    let header = header.unwrap();

    let asset_uuid = asset_uuid.unwrap_or_else(|| *uuid::Uuid::new_v4().as_bytes());
    let mut info = asdcplib::WriterInfo {
        asset_uuid,
        context_id: *uuid::Uuid::new_v4().as_bytes(),
        label_set: asdcplib::LabelSet::Smpte,
        ..Default::default()
    };

    let mut enc_ctx = None;
    let mut hmac_ctx = None;
    if let Some(e) = &encryption {
        info.encrypted_essence = true;
        info.uses_hmac = true;
        info.cryptographic_key_id = e.key_id;
        let mut ec = asdcplib::crypto::AesEncContext::new();
        if let Err(err) = ec.init_key(&e.content_key) {
            tracing::error!("AES key init failed: {err}");
            return None;
        }
        let mut hc = asdcplib::crypto::HmacContext::new();
        if let Err(err) = hc.init_key(&e.content_key, info.label_set) {
            tracing::error!("HMAC key init failed: {err}");
            return None;
        }
        enc_ctx = Some(ec);
        hmac_ctx = Some(hc);
    }

    let fps = if frame_rate == 0 { 24 } else { frame_rate };
    let desc = asdcplib::jp2k::PictureDescriptor {
        edit_rate: asdcplib::Rational::new(fps as i32, 1),
        sample_rate: asdcplib::Rational::new(fps as i32, 1),
        stored_width: header.width,
        stored_height: header.height,
        aspect_ratio: asdcplib::Rational::new(header.width as i32, header.height as i32),
        container_duration: frames.len() as u32,
        component_count: header.num_components,
    };
    let hdr = HdrMetadata {
        transfer_characteristic: Some(TRANSFER_CHARACTERISTIC_ST2084),
        color_primaries: Some(COLOR_PRIMARIES_P3D65),
        ..Default::default()
    };

    let mut writer = asdcplib::jp2k::MxfWriter::new();
    let output_str = output_mxf.to_string_lossy().to_string();
    if let Err(e) = writer.open_write_hdr(&output_str, &info, &desc, &hdr, 16384) {
        tracing::error!("JP2K HDR open_write failed: {e}");
        return None;
    }
    for frame in &frames {
        if let Err(e) = writer.write_frame(frame, enc_ctx.as_mut(), hmac_ctx.as_mut()) {
            tracing::error!("JP2K HDR write_frame failed: {e}");
            return None;
        }
    }
    if let Err(e) = writer.finalize() {
        tracing::error!("JP2K HDR finalize failed: {e}");
        return None;
    }

    let data = std::fs::read(output_mxf).ok()?;
    let hash = {
        use sha1::Digest;
        sha1::Sha1::digest(&data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    tracing::info!(
        "Wrapped DCI HDR picture MXF (ST 2084 / P3-D65): {}",
        output_mxf.display()
    );
    Some(postkit::mxf_wrap::MxfTrackFile {
        uuid: uuid::Uuid::from_bytes(asset_uuid).hyphenated().to_string(),
        hash,
        size: data.len() as u64,
        duration: frames.len() as u64,
        path: output_mxf.to_path_buf(),
        success: true,
        error: String::new(),
    })
}

/// Wrap a stereoscopic (ST 429-10) picture MXF from equal-length left/right eye
/// frame lists. `fps` is the composition edit rate; the essence carries two
/// frames per edit unit (left then right). Returns the track file (real embedded
/// asset id, hash, size, per-eye frame count as duration) or None on failure.
pub fn wrap_stereoscopic_files(
    left_files: Vec<PathBuf>,
    right_files: Vec<PathBuf>,
    output_mxf: &std::path::Path,
    fps: u32,
    encryption: Option<postkit::mxf_wrap::MxfEncryption>,
    asset_uuid: Option<[u8; 16]>,
) -> Option<postkit::mxf_wrap::MxfTrackFile> {
    if left_files.is_empty() || right_files.is_empty() {
        tracing::error!("stereoscopic wrap needs both eyes");
        return None;
    }
    if left_files.len() != right_files.len() {
        tracing::error!(
            "eye frame count mismatch: left={}, right={}",
            left_files.len(),
            right_files.len()
        );
        return None;
    }
    let fps = if fps == 0 { 24 } else { fps };
    let opts = postkit::mxf_wrap::StereoscopicWrapOptions {
        left_files,
        right_files,
        output: output_mxf.to_path_buf(),
        fps_num: fps,
        fps_den: 1,
        encryption,
        asset_uuid,
    };
    let result = postkit::mxf_wrap::wrap_stereoscopic(&opts);
    if result.success {
        tracing::info!("Wrapped stereoscopic MXF: {}", output_mxf.display());
        Some(result)
    } else {
        tracing::error!("stereoscopic MXF wrap failed: {}", result.error);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // minimal RIFF/WAVE header with the given sample rate and no audio payload
    fn write_wav(path: &std::path::Path, sample_rate: u32) {
        let channels: u16 = 2;
        let bits: u16 = 24;
        let block_align = (bits / 8) * channels;
        let byte_rate = sample_rate * block_align as u32;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&36u32.to_le_bytes());
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
        w.extend_from_slice(&0u32.to_le_bytes());
        std::fs::File::create(path).unwrap().write_all(&w).unwrap();
    }

    #[test]
    fn reads_fmt_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        write_wav(&p, 44_100);
        let (channels, sr) = wav_fmt(&p).unwrap();
        assert_eq!(sr, 44_100);
        assert_eq!(channels, 2);
    }

    #[test]
    fn mca_config_by_channel_count() {
        assert_eq!(build_mca_config(2, None, None).as_deref(), Some("L,R"));
        assert_eq!(
            build_mca_config(6, None, None).as_deref(),
            Some("51(L,R,C,LFE,Ls,Rs)")
        );
        assert_eq!(
            build_mca_config(8, None, None).as_deref(),
            Some("71(L,R,C,LFE,Ls,Rs,Lrs,Rrs)")
        );
        // 5.1 plus HI/VI accessibility channels at indices 6 and 7
        assert_eq!(
            build_mca_config(8, Some(6), Some(7)).as_deref(),
            Some("51(L,R,C,LFE,Ls,Rs),HI,VIN")
        );
    }

    #[test]
    fn accepts_wave_format_extensible_pcm() {
        // ffmpeg emits 6ch pcm as WAVE_FORMAT_EXTENSIBLE, not plain format 1
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.wav");
        let output = dir.path().join("dcp.wav");
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&78u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&40u32.to_le_bytes());
        wav.extend_from_slice(&0xFFFEu16.to_le_bytes()); // extensible
        wav.extend_from_slice(&6u16.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&864_000u32.to_le_bytes());
        wav.extend_from_slice(&18u16.to_le_bytes());
        wav.extend_from_slice(&24u16.to_le_bytes());
        wav.extend_from_slice(&22u16.to_le_bytes()); // cbSize
        wav.extend_from_slice(&24u16.to_le_bytes()); // valid bits
        wav.extend_from_slice(&0x3Fu32.to_le_bytes()); // channel mask
        // KSDATAFORMAT_SUBTYPE_PCM guid; the leading u16 is the format code
        wav.extend_from_slice(&[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38,
            0x9b, 0x71,
        ]);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&18u32.to_le_bytes());
        for sample in 1u8..=18 {
            wav.push(sample);
        }
        std::fs::write(&input, wav).unwrap();

        assert!(prepare_51_audio(&input, &output, AudioInputOrder::Canonical51).unwrap());
        assert_eq!(wav_channels(&output).unwrap(), 16);
    }

    #[test]
    fn pads_51_to_16_channels_with_canonical_mca_labels() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.wav");
        let output = dir.path().join("dcp.wav");
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&54u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&6u16.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&864_000u32.to_le_bytes());
        wav.extend_from_slice(&18u16.to_le_bytes());
        wav.extend_from_slice(&24u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&18u32.to_le_bytes());
        for sample in 1u8..=18 {
            wav.push(sample);
        }
        std::fs::write(&input, wav).unwrap();

        assert!(prepare_51_audio(&input, &output, AudioInputOrder::Canonical51).unwrap());
        assert_eq!(wav_channels(&output).unwrap(), 16);
        assert_eq!(
            build_mca_config(16, None, None).as_deref(),
            Some("51(L,R,C,LFE,Ls,Rs),-,-,-,-,-,-,-,-,-,-")
        );
    }

    #[test]
    fn reorders_alternate_51_input() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.wav");
        let output = dir.path().join("dcp.wav");
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&54u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&6u16.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&288_000u32.to_le_bytes());
        wav.extend_from_slice(&6u16.to_le_bytes());
        wav.extend_from_slice(&8u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&6u32.to_le_bytes());
        wav.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        std::fs::write(&input, wav).unwrap();

        prepare_51_audio(&input, &output, AudioInputOrder::LrcLsRsLfe).unwrap();
        let wav = std::fs::read(output).unwrap();
        assert_eq!(&wav[44..50], &[1, 2, 3, 6, 4, 5]);
        assert!(wav[50..60].iter().all(|sample| *sample == 0));
    }

    // Wrap a real DCI J2K frame with --hdr-dci signaling, then read the picture
    // essence descriptor back and assert the ST 2084 / P3-D65 ULs are present.
    #[test]
    fn hdr_dci_wrap_writes_st2084_and_p3d65_uls() {
        use asdcplib::jp2k::{COLOR_PRIMARIES_P3D65, TRANSFER_CHARACTERISTIC_ST2084};

        let dir = tempfile::tempdir().unwrap();
        let seed = dir.path().join("seed.j2c");
        crate::pad::generate_black_frame(2048, 1080, 24, &seed).expect("encode DCI frame");
        let frames: Vec<PathBuf> = (0..3)
            .map(|i| {
                let f = dir.path().join(format!("frame_{i:05}.j2c"));
                std::fs::copy(&seed, &f).unwrap();
                f
            })
            .collect();

        let mxf = dir.path().join("hdr_picture.mxf");
        let track = wrap_j2k_hdr_files(frames, &mxf, 24, None, None).expect("hdr wrap");
        assert_eq!(track.duration, 3);
        assert!(mxf.exists());

        let mut reader = asdcplib::jp2k::MxfReader::new();
        reader
            .open_read(&mxf.to_string_lossy())
            .expect("open hdr mxf");
        let tc = reader.transfer_characteristic().expect("read transfer");
        assert_eq!(
            tc,
            Some(TRANSFER_CHARACTERISTIC_ST2084),
            "picture descriptor must carry the ST 2084 TransferCharacteristic UL"
        );
        let hdr = reader.hdr_metadata().expect("read hdr metadata");
        assert_eq!(
            hdr.color_primaries,
            Some(COLOR_PRIMARIES_P3D65),
            "picture descriptor must carry the P3-D65 ColorPrimaries UL"
        );
    }

    #[test]
    fn rejects_non_dcp_sample_rate() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("bad.wav");
        write_wav(&wav, 44_100);
        let config = MxfWrapConfig {
            input_path: wav,
            output_mxf: dir.path().join("out.mxf"),
            mxf_type: MxfType::PcmAudio,
            frame_rate: 24,
            encryption: None,
            mca_config: None,
            asset_uuid: None,
        };
        // 44.1 kHz is illegal in a DCP: wrap must fail loud, not mislabel it
        assert!(wrap_mxf_result(&config).is_none());
        assert!(!config.output_mxf.exists());
    }
}
