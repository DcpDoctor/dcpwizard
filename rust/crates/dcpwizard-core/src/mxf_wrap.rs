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
    /// AES-128 content encryption for the wrapped essence. Not serialized.
    #[serde(skip)]
    pub encryption: Option<postkit::mxf_wrap::MxfEncryption>,
    /// ST 429-12 MCA channel labels for a PCM wrap (e.g. `"51(L,R,C,LFE,Ls,Rs)"`
    /// plus the spoken language). None auto-derives the labels from the probed
    /// channel count and leaves the language to asdcplib's en-US default.
    #[serde(skip)]
    pub mca_config: Option<postkit::mxf_wrap::McaConfig>,
    /// The id the MXF carries as its AssetUUID. A caller that already named the
    /// output file or wrote the CPL/PKL/ASSETMAP entry must pass its id here, or
    /// the MXF ends up carrying a different id than the package claims. None
    /// mints a fresh id.
    #[serde(default)]
    pub asset_uuid: Option<[u8; 16]>,
}

/// Collect sorted files from a directory, or treat a single file as one-element list.
pub(crate) fn collect_inputs(path: &std::path::Path) -> Result<Vec<PathBuf>, String> {
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

/// The sound layouts a DCP track is packaged with. Nothing in DCP carries a
/// track of any other width.
pub const PACKAGED_CHANNEL_COUNTS: [u32; 4] = [2, 6, 8, 16];

/// A canonical DCP 5.1 track: L, R, C, LFE, Ls, Rs.
pub const CANONICAL_51_CHANNELS: u32 = 6;
/// What a 5.1 source is packaged with when nothing asks for a count.
pub const DEFAULT_PACKAGED_51_CHANNELS: u32 = 16;

/// Refuse a packaged channel count that is not a DCP sound layout.
pub fn check_packaged_channel_count(count: u32) -> Result<(), String> {
    if PACKAGED_CHANNEL_COUNTS.contains(&count) {
        return Ok(());
    }
    Err(format!(
        "{count} is not a DCP sound layout: package {}, {}, {} or {} channels",
        PACKAGED_CHANNEL_COUNTS[0],
        PACKAGED_CHANNEL_COUNTS[1],
        PACKAGED_CHANNEL_COUNTS[2],
        PACKAGED_CHANNEL_COUNTS[3]
    ))
}

/// Refuse a source carrying more channels than the packaged track has room for.
/// Filling adds silent channels, so it can never narrow a track.
pub fn check_source_fits_packaged_channels(
    source_channels: u32,
    packaged_channels: u32,
) -> Result<(), String> {
    if source_channels <= packaged_channels {
        return Ok(());
    }
    Err(format!(
        "the sound carries {source_channels} channels, more than the {packaged_channels} \
         asked for: fold them onto {packaged_channels} lanes with --audio-map, or package \
         {source_channels} channels or more"
    ))
}

const WAV_HEADER_BYTES: usize = 44;
const WAV_IO_BUFFER_BYTES: usize = 1 << 20;

/// Lay a WAV out the way the packaged sound track carries it: a six-channel
/// source is reordered to canonical DCP 5.1, then silent channels fill the track
/// up to `packaged_channels`. None keeps the wrap's own rule, where 5.1 is
/// widened to 16 and every other source is left as it is. Returns false when the
/// source was left untouched and no file was written.
pub fn prepare_packaged_channels(
    input: &Path,
    output: &Path,
    input_order: AudioInputOrder,
    packaged_channels: Option<u32>,
) -> Result<bool, String> {
    use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};

    let cannot_read = |e: std::io::Error| format!("cannot read {}: {e}", input.display());
    let mut source = BufReader::with_capacity(
        WAV_IO_BUFFER_BYTES,
        std::fs::File::open(input).map_err(cannot_read)?,
    );
    let source_len = source.get_ref().metadata().map_err(cannot_read)?.len();
    let mut riff = [0u8; 12];
    if source_len < riff.len() as u64 {
        return Err(format!("{} is not a RIFF/WAVE file", input.display()));
    }
    source.read_exact(&mut riff).map_err(cannot_read)?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(format!("{} is not a RIFF/WAVE file", input.display()));
    }

    let mut pos = riff.len() as u64;
    let mut fmt: Option<Vec<u8>> = None;
    let mut payload: Option<(u64, u64)> = None;
    while pos + 8 <= source_len {
        source.seek(SeekFrom::Start(pos)).map_err(cannot_read)?;
        let mut chunk = [0u8; 8];
        source.read_exact(&mut chunk).map_err(cannot_read)?;
        let size = u64::from(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
        let body = pos + 8;
        if body + size > source_len {
            return Err(format!("{} has a truncated WAV chunk", input.display()));
        }
        match &chunk[0..4] {
            b"fmt " if size >= 16 => {
                let mut bytes = vec![0u8; size as usize];
                source.read_exact(&mut bytes).map_err(cannot_read)?;
                fmt = Some(bytes);
            }
            b"data" => payload = Some((body, size)),
            _ => {}
        }
        pos = body + size + (size & 1);
    }
    let Some(fmt) = fmt else {
        return Err(format!("no fmt chunk found in {}", input.display()));
    };
    let Some((payload_offset, payload_len)) = payload else {
        return Err(format!("no data chunk found in {}", input.display()));
    };
    let format = u16::from_le_bytes(fmt[0..2].try_into().unwrap());
    let channels = u32::from(u16::from_le_bytes(fmt[2..4].try_into().unwrap()));
    let target_channels = match packaged_channels {
        Some(count) => {
            check_packaged_channel_count(count)?;
            check_source_fits_packaged_channels(channels, count)?;
            count
        }
        None if channels == CANONICAL_51_CHANNELS => DEFAULT_PACKAGED_51_CHANNELS,
        None => return Ok(false),
    };
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
    let source_frame_bytes = sample_bytes * channels as usize;
    if payload_len % source_frame_bytes as u64 != 0 {
        return Err(format!("{} has incomplete audio frames", input.display()));
    }
    let frame_count = payload_len / source_frame_bytes as u64;

    let order: Vec<usize> = if channels == CANONICAL_51_CHANNELS {
        match input_order {
            AudioInputOrder::Canonical51 => vec![0, 1, 2, 3, 4, 5],
            AudioInputOrder::LrcLsRsLfe => vec![0, 1, 2, 5, 3, 4],
        }
    } else {
        (0..channels as usize).collect()
    };
    let output_frame_bytes = sample_bytes * target_channels as usize;
    let data_size = frame_count * output_frame_bytes as u64;
    let riff_size = WAV_HEADER_BYTES as u64 - 8 + data_size;
    if riff_size > u64::from(u32::MAX) {
        return Err(format!(
            "{} widened to {target_channels} channels exceeds the 4 GiB a RIFF/WAVE file can hold",
            input.display()
        ));
    }

    let cannot_write = |e: std::io::Error| format!("cannot write {}: {e}", output.display());
    let mut sink = BufWriter::with_capacity(
        WAV_IO_BUFFER_BYTES,
        std::fs::File::create(output).map_err(cannot_write)?,
    );
    let mut header = Vec::with_capacity(WAV_HEADER_BYTES);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(riff_size as u32).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&(target_channels as u16).to_le_bytes());
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&(sample_rate * output_frame_bytes as u32).to_le_bytes());
    header.extend_from_slice(&(output_frame_bytes as u16).to_le_bytes());
    header.extend_from_slice(&bits.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(data_size as u32).to_le_bytes());
    sink.write_all(&header).map_err(cannot_write)?;

    source
        .seek(SeekFrom::Start(payload_offset))
        .map_err(cannot_read)?;
    let mut source_frame = vec![0u8; source_frame_bytes];
    // bytes past the source channels stay zero, the silent fill
    let mut output_frame = vec![0u8; output_frame_bytes];
    for _ in 0..frame_count {
        source.read_exact(&mut source_frame).map_err(cannot_read)?;
        for (slot, channel) in order.iter().enumerate() {
            let from = channel * sample_bytes;
            let to = slot * sample_bytes;
            output_frame[to..to + sample_bytes]
                .copy_from_slice(&source_frame[from..from + sample_bytes]);
        }
        sink.write_all(&output_frame).map_err(cannot_write)?;
    }
    sink.flush().map_err(cannot_write)?;
    Ok(true)
}

/// Build a ST 429-12 MCA config for a sound wrap. The layout comes from the
/// channels the content fills, the label count from the channels the track is
/// packaged with, so the silent fill channels get '-' placeholders instead of
/// claiming a soundfield they carry nothing of. HI and VI-N are labelled as
/// standalone channels at the given indices. Returns None when the layout has no
/// asdcplib DCP label.
pub fn build_mca_config(
    content_channels: u32,
    packaged_channels: u32,
    hi_channel: Option<u32>,
    vi_channel: Option<u32>,
) -> Option<String> {
    use postkit::mca::{McaLabel, McaTagSymbol};

    let extra = hi_channel.is_some() as u32 + vi_channel.is_some() as u32;
    let main_count = content_channels.saturating_sub(extra);
    // main layout by channel count; 8 is 7.1 here (postkit's detect_soundfield
    // treats 8 as 5.1+HI+VI, but accessibility tracks are opt-in via the flags).
    // 16 is 5.1 because no content is 16 channels wide: a caller that knows only
    // the packaged count is looking at the 5.1 track this wrap has always filled.
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
    for _ in (sf.channels.len() as u32)..packaged_channels {
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
    mca_config: Option<postkit::mxf_wrap::McaConfig>,
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
                // nothing here knows what the content was, so the layout is read
                // from the packaged count
                mca_config =
                    build_mca_config(channels as u32, channels as u32, None, None).map(|labels| {
                        postkit::mxf_wrap::McaConfig {
                            labels,
                            spoken_language: None,
                        }
                    });
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
        timed_text_duration_frames: None,
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
/// the first input file; resources follow in order. `duration_frames` is the
/// essence duration to write when the caller knows it, which is how a reel's
/// subtitle spans the reel; None derives it from the last cue.
pub fn wrap_timed_text_resources(
    dcst: &std::path::Path,
    resources: &[(PathBuf, [u8; 16])],
    output_mxf: &std::path::Path,
    frame_rate: u32,
    asset_uuid: Option<[u8; 16]>,
    duration_frames: Option<u32>,
    encryption: Option<postkit::mxf_wrap::MxfEncryption>,
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
        encryption,
        mca_config: None,
        resource_ids,
        hdr: None,
        asset_uuid,
        timed_text_duration_frames: duration_frames,
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

// the addendum names one descriptor item, the ST 2084 transfer, and no primaries
pub fn dci_hdr_metadata() -> asdcplib::jp2k::HdrMetadata {
    asdcplib::jp2k::HdrMetadata {
        transfer_characteristic: Some(asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084),
        ..Default::default()
    }
}

pub fn wrap_j2k_hdr_files(
    input_files: Vec<PathBuf>,
    output_mxf: &std::path::Path,
    frame_rate: u32,
    encryption: Option<postkit::mxf_wrap::MxfEncryption>,
    asset_uuid: Option<[u8; 16]>,
) -> Option<postkit::mxf_wrap::MxfTrackFile> {
    if input_files.is_empty() {
        tracing::error!("no essence files to wrap into {}", output_mxf.display());
        return None;
    }
    let fps = if frame_rate == 0 { 24 } else { frame_rate };
    let options = postkit::mxf_wrap::IncrementalWrapOptions {
        output: output_mxf.to_path_buf(),
        standard: postkit::mxf_wrap::MxfStandard::AsDcp,
        fps_num: fps,
        fps_den: 1,
        encryption,
        hdr: Some(dci_hdr_metadata()),
        asset_uuid,
    };
    let mut wrap = match postkit::mxf_wrap::IncrementalJ2kWrap::new(options) {
        Ok(wrap) => wrap,
        Err(e) => {
            tracing::error!("JP2K HDR wrap failed: {e}");
            return None;
        }
    };
    // a feature's codestreams run to tens of GB
    for f in &input_files {
        let frame = match std::fs::read(f) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::error!("failed to read {}: {e}", f.display());
                return None;
            }
        };
        if let Err(e) = wrap.write_frame(&frame) {
            tracing::error!("{e}: {}", f.display());
            return None;
        }
    }
    match wrap.finish() {
        Ok(track) => {
            tracing::info!(
                "Wrapped DCI HDR picture MXF (ST 2084): {}",
                output_mxf.display()
            );
            Some(track)
        }
        Err(e) => {
            tracing::error!("JP2K HDR wrap failed: {e}");
            None
        }
    }
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
        assert_eq!(build_mca_config(2, 2, None, None).as_deref(), Some("L,R"));
        assert_eq!(
            build_mca_config(6, 6, None, None).as_deref(),
            Some("51(L,R,C,LFE,Ls,Rs)")
        );
        assert_eq!(
            build_mca_config(8, 8, None, None).as_deref(),
            Some("71(L,R,C,LFE,Ls,Rs,Lrs,Rrs)")
        );
        // 5.1 plus HI/VI accessibility channels at indices 6 and 7
        assert_eq!(
            build_mca_config(8, 8, Some(6), Some(7)).as_deref(),
            Some("51(L,R,C,LFE,Ls,Rs),HI,VIN")
        );
    }

    /// Stereo filled to a wide track keeps the stereo labels: the fill channels
    /// carry silence, not a 5.1 mix.
    #[test]
    fn filled_channels_are_placeholders_not_soundfield_labels() {
        assert_eq!(
            build_mca_config(2, 16, None, None).as_deref(),
            Some("L,R,-,-,-,-,-,-,-,-,-,-,-,-,-,-")
        );
        assert_eq!(
            build_mca_config(6, 8, None, None).as_deref(),
            Some("51(L,R,C,LFE,Ls,Rs),-,-")
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

        assert!(
            prepare_packaged_channels(&input, &output, AudioInputOrder::Canonical51, None).unwrap()
        );
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

        assert!(
            prepare_packaged_channels(&input, &output, AudioInputOrder::Canonical51, None).unwrap()
        );
        assert_eq!(wav_channels(&output).unwrap(), 16);
        assert_eq!(
            build_mca_config(6, 16, None, None).as_deref(),
            Some("51(L,R,C,LFE,Ls,Rs),-,-,-,-,-,-,-,-,-,-")
        );
    }

    /// A WAV with `channels` interleaved 8-bit samples, one sample per channel
    /// per frame, counting up so a reorder or a fill is visible byte by byte.
    fn write_counting_wav(path: &std::path::Path, channels: u16, frames: usize) {
        let block_align = channels;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&0u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&(48_000 * block_align as u32).to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&8u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        let payload: Vec<u8> = (1..=(channels as usize * frames) as u8).collect();
        wav.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        wav.extend_from_slice(&payload);
        let riff_size = wav.len() as u32 - 8;
        wav[4..8].copy_from_slice(&riff_size.to_le_bytes());
        std::fs::write(path, wav).unwrap();
    }

    /// Stereo filled to a wider track keeps its samples where they were and pays
    /// only silence for the rest.
    #[test]
    fn fills_stereo_to_the_requested_channel_count() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("stereo.wav");
        write_counting_wav(&input, 2, 2);

        for (target, expected_frame) in [
            (6u32, vec![1u8, 2, 0, 0, 0, 0]),
            (16, vec![1u8, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        ] {
            let output = dir.path().join(format!("filled_{target}.wav"));
            assert!(
                prepare_packaged_channels(
                    &input,
                    &output,
                    AudioInputOrder::Canonical51,
                    Some(target)
                )
                .unwrap()
            );
            assert_eq!(wav_channels(&output).unwrap(), target as u16);
            let written = std::fs::read(&output).unwrap();
            let width = target as usize;
            assert_eq!(&written[44..44 + width], expected_frame.as_slice());
            // the second frame's source samples land bit-exact too
            assert_eq!(&written[44 + width..44 + width + 2], &[3, 4]);
        }
    }

    #[test]
    fn fills_51_to_eight_channels() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("five_one.wav");
        let output = dir.path().join("eight.wav");
        write_counting_wav(&input, 6, 2);

        assert!(
            prepare_packaged_channels(&input, &output, AudioInputOrder::Canonical51, Some(8))
                .unwrap()
        );
        assert_eq!(wav_channels(&output).unwrap(), 8);
        let written = std::fs::read(&output).unwrap();
        assert_eq!(&written[44..52], &[1, 2, 3, 4, 5, 6, 0, 0]);
    }

    /// Asking for the count the source already has leaves the track that wide.
    #[test]
    fn an_explicit_six_keeps_a_51_source_at_six_channels() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("five_one.wav");
        let output = dir.path().join("six.wav");
        write_counting_wav(&input, 6, 2);

        assert!(
            prepare_packaged_channels(&input, &output, AudioInputOrder::Canonical51, Some(6))
                .unwrap()
        );
        assert_eq!(wav_channels(&output).unwrap(), 6);
        assert_eq!(
            &std::fs::read(&output).unwrap()[44..50],
            &[1, 2, 3, 4, 5, 6]
        );
    }

    /// Filling only adds silence, so a source wider than the count asked for is
    /// refused rather than narrowed.
    #[test]
    fn refuses_a_source_wider_than_the_packaged_count() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("seven_one.wav");
        let output = dir.path().join("six.wav");
        write_counting_wav(&input, 8, 2);

        let error =
            prepare_packaged_channels(&input, &output, AudioInputOrder::Canonical51, Some(6))
                .unwrap_err();
        assert!(error.contains('8') && error.contains('6'), "{error}");
        assert!(error.contains("--audio-map"), "{error}");
        assert!(!output.exists());
    }

    #[test]
    fn refuses_a_packaged_count_that_is_not_a_dcp_layout() {
        let error = check_packaged_channel_count(4).unwrap_err();
        assert!(error.contains('4'), "{error}");
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

        prepare_packaged_channels(&input, &output, AudioInputOrder::LrcLsRsLfe, None).unwrap();
        let wav = std::fs::read(output).unwrap();
        assert_eq!(&wav[44..50], &[1, 2, 3, 6, 4, 5]);
        assert!(wav[50..60].iter().all(|sample| *sample == 0));
    }

    // Wrap a real DCI J2K frame with --hdr-dci signaling, then read the picture
    // essence descriptor back and assert the ST 2084 UL is the only colour item.
    #[test]
    fn hdr_dci_wrap_writes_the_st2084_ul_and_no_primaries() {
        use asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084;

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

        let codestream = reader
            .picture_descriptor()
            .expect("picture descriptor")
            .codestream;
        assert!(
            postkit::j2k::J2kProfile::from(codestream.rsize).is_dci_cinema(),
            "RSIZ {:#06x} is not a DCI cinema profile",
            codestream.rsize
        );
        assert_eq!((codestream.xsize, codestream.ysize), (2048, 1080));

        let tc = reader.transfer_characteristic().expect("read transfer");
        assert_eq!(
            tc,
            Some(TRANSFER_CHARACTERISTIC_ST2084),
            "picture descriptor must carry the ST 2084 TransferCharacteristic UL"
        );
        let hdr = reader.hdr_metadata().expect("read hdr metadata");
        assert_eq!(
            hdr.color_primaries, None,
            "the addendum names no ColorPrimaries item, so the descriptor must carry none"
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
