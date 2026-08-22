//! ISDCF Doc 13 sign-language video encoding.
//!
//! Packs a VP9 video into a PCM program carried on channel 15 of the 16-channel
//! MainSound track, per ISDCF-Doc13 "Sign Language Video Encoding for Digital
//! Cinema" v1.0. The channel is labelled SLVS via the MCA framework and the CPL
//! gets an ExtensionMetadata scope. Packing math verified against the ISDCF
//! reference encoder (ISDCF/Sign-Language-Video-Encoding, encode-vp9-wav) and
//! the channel/UL/CPL metadata against libdcp (cth103/libdcp).

use std::path::Path;

/// SLVS MCA Label Dictionary ID UL (asdcplib MDD_AudioChannelSLVS, libdcp
/// sign_language[]). Kept for documentation; the label is emitted through the
/// asdcplib MCA config-string parser which already carries this UL for "SLVS".
pub const SLVS_MCA_UL: [u8; 16] = [
    0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x0d, 0x0d, 0x0f, 0x03, 0x02, 0x01, 0x01, 0x00, 0x00,
];

/// Sign-language program sits on channel 15 (0-based index 14) of the 16-channel
/// MainSound track (libdcp Channel::SIGN_LANGUAGE = 14).
pub const SLVS_CHANNEL_INDEX: usize = 14;

/// DCP MainSound track is always 16 channels.
pub const DCP_SOUND_CHANNELS: usize = 16;

const MARKER: u32 = 0xFFFF_FFFF;
const SAMPLE_RATE: u32 = 48_000;
const BYTES_PER_SAMPLE: usize = 3; // 24-bit
const CHUNK_SECONDS: u32 = 2;
/// VP9 encode rate, fixed at 24 fps per ISDCF Doc 13 regardless of the DCP rate.
const VP9_FPS: u32 = 24;
/// Bytes in one 2-second PCM block: 48000 * 3 * 2. The whole per-chunk block
/// (20-byte header + VP9 header + VP9 segment + null padding) is this size.
pub const BLOCK_BYTES: usize = (SAMPLE_RATE as usize) * BYTES_PER_SAMPLE * (CHUNK_SECONDS as usize);
/// VP9 bitrate from the ISDCF reference encoder (48000*24/2 = 576 kbps). Well
/// under the ISDCF Doc 13 1 Mbps ceiling and guarantees a 2-second segment plus
/// its header fits inside one BLOCK_BYTES PCM block.
const VP9_BITRATE: u32 = SAMPLE_RATE * 24 / 2;

/// Pack one VP9 chunk into a BLOCK_BYTES PCM block: 5 big-endian u32
/// (MARKER, Lv=segment len, Lb=block len, Le=vp9 header len, MARKER), then the
/// shared VP9/webm header, then the segment, zero-padded to BLOCK_BYTES.
/// Fails loud if the header+segment cannot fit (per the reference's size check).
pub fn pack_block(vp9_header: &[u8], vp9_segment: &[u8]) -> Result<Vec<u8>, String> {
    let payload = 20 + vp9_header.len() + vp9_segment.len();
    if payload > BLOCK_BYTES {
        return Err(format!(
            "VP9 chunk too large for a {BLOCK_BYTES}-byte block: header {} + segment {} + 20 = {payload}",
            vp9_header.len(),
            vp9_segment.len()
        ));
    }
    let mut block = Vec::with_capacity(BLOCK_BYTES);
    for v in [
        MARKER,
        vp9_segment.len() as u32,
        BLOCK_BYTES as u32,
        vp9_header.len() as u32,
        MARKER,
    ] {
        block.extend_from_slice(&v.to_be_bytes());
    }
    block.extend_from_slice(vp9_header);
    block.extend_from_slice(vp9_segment);
    block.resize(BLOCK_BYTES, 0);
    Ok(block)
}

/// Encode a source video to VP9 chunks via ffmpeg's webm_chunk muxer, replicating
/// the ISDCF reference encoder. Returns (shared vp9/webm header, per-chunk
/// segments in order). Fails loud when ffmpeg lacks the libvpx-vp9 encoder or the
/// source cannot be conformed.
fn encode_vp9_chunks(input: &Path) -> Result<(Vec<u8>, Vec<Vec<u8>>), String> {
    if !crate::probe::ffmpeg_has_encoder("libvpx-vp9") {
        return Err(
            "ffmpeg has no libvpx-vp9 encoder; cannot conform the sign-language video".into(),
        );
    }
    let tmp = tempfile::tempdir().map_err(|e| format!("cannot create temp dir: {e}"))?;
    let build = tmp.path();
    let keyint = (CHUNK_SECONDS * VP9_FPS).to_string();
    let hdr = build.join("chunk.hdr");
    let pattern = build.join("chunk_%05d.chk");
    let bitrate = VP9_BITRATE.to_string();
    let status = std::process::Command::new("ffmpeg")
        .args(["-loglevel", "error", "-hide_banner", "-i"])
        .arg(input)
        .args([
            "-map",
            "0:v:0",
            "-pix_fmt",
            "yuv420p",
            "-vf",
            "scale=w=480:h=640:force_original_aspect_ratio=decrease",
            "-r",
            "24",
            "-c:v",
            "libvpx-vp9",
            "-keyint_min",
            &keyint,
            "-g",
            &keyint,
            "-speed",
            "6",
            "-tile-columns",
            "4",
            "-frame-parallel",
            "1",
            "-static-thresh",
            "0",
            "-max-intra-rate",
            "300",
            "-deadline",
            "realtime",
            "-lag-in-frames",
            "0",
            "-error-resilient",
            "1",
            "-b:v",
            &bitrate,
            "-minrate",
            &bitrate,
            "-maxrate",
            &bitrate,
            "-an",
            "-sn",
            "-f",
            "webm_chunk",
            "-header",
        ])
        .arg(&hdr)
        .args(["-chunk_start_index", "1"])
        .arg(&pattern)
        .status()
        .map_err(|e| format!("failed to run ffmpeg: {e}"))?;
    if !status.success() {
        return Err(format!(
            "ffmpeg could not conform {} to VP9",
            input.display()
        ));
    }
    let header = std::fs::read(&hdr).map_err(|e| format!("cannot read VP9 header: {e}"))?;
    let mut chunk_files: Vec<_> = std::fs::read_dir(build)
        .map_err(|e| format!("cannot read VP9 chunk dir: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("chk"))
        .collect();
    chunk_files.sort();
    if chunk_files.is_empty() {
        return Err("ffmpeg produced no VP9 chunks".into());
    }
    let mut segments = Vec::with_capacity(chunk_files.len());
    for f in chunk_files {
        segments.push(std::fs::read(&f).map_err(|e| format!("cannot read VP9 chunk: {e}"))?);
    }
    Ok((header, segments))
}

/// Build the mono 24-bit PCM byte stream (channel 15 payload) from a source video:
/// encode to VP9 chunks, then pack each chunk into a BLOCK_BYTES PCM block.
pub fn build_slvs_pcm(input_video: &Path) -> Result<Vec<u8>, String> {
    let (header, segments) = encode_vp9_chunks(input_video)?;
    let mut pcm = Vec::with_capacity(segments.len() * BLOCK_BYTES);
    for seg in &segments {
        pcm.extend_from_slice(&pack_block(&header, seg)?);
    }
    Ok(pcm)
}

/// asdcplib MCA config string placing the SLVS channel (with its spoken language)
/// on channel 15 of a 16-channel track. `main` is the config for the leading
/// audio channels (e.g. "51(L,R,C,LFE,Ls,Rs)" for 5.1 or "L,R" for stereo),
/// covering `main_channels` physical channels; the gap up to channel 15 and the
/// trailing channel 16 are '-' placeholders. Empty `main` (no leading layout)
/// means the SLVS channel sits after 14 silent-fill channels. The asdcplib parser
/// accepts `SLVS:<lang>` and emits the AudioChannelSLVS label with that RFC 5646
/// tag. The trailing channel-16 '-' is required: the parser only applies the
/// channel language when another channel follows the SLVS token.
pub fn slvs_mca_config(main: &str, main_channels: usize, language: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !main.is_empty() {
        parts.push(main.to_string());
    }
    // fill channels between the main layout and channel 15 (index 14)
    for _ in main_channels..SLVS_CHANNEL_INDEX {
        parts.push("-".to_string());
    }
    parts.push(format!("SLVS:{language}"));
    // trailing silent fill up to 16 channels
    for _ in (SLVS_CHANNEL_INDEX + 1)..DCP_SOUND_CHANNELS {
        parts.push("-".to_string());
    }
    parts.join(",")
}

/// Build the combined 16-channel MainSound WAV for a sign-language DCP: encode
/// `slvs_video` to VP9 and pack it onto channel 15, carry `main_audio` (24-bit
/// 48 kHz WAV) on the leading channels when present, and pad to at least
/// `min_frames` picture frames. Writes `output` and returns the number of leading
/// main-audio channels (0 when no main audio), for the SLVS MCA config.
pub fn build_slvs_sound(
    slvs_video: &Path,
    main_audio: Option<&Path>,
    min_frames: u64,
    fps: u32,
    output: &Path,
) -> Result<u32, String> {
    let slvs_pcm = build_slvs_pcm(slvs_video)?;
    let (main_channels, main16) = match main_audio {
        Some(p) => {
            let widened = widen_wav_to_16ch(p)?;
            (main_audio_channels(p)?, widened)
        }
        None => (0, Vec::new()),
    };
    let main_ref = main_audio.map(|_| main16.as_slice());
    write_16ch_slvs_wav(&slvs_pcm, main_ref, min_frames, fps, output)?;
    Ok(main_channels)
}

/// Leading channel count of a 24-bit 48 kHz WAV, for the SLVS soundfield layout.
fn main_audio_channels(path: &Path) -> Result<u32, String> {
    let data = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if data.len() < 24 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{} is not a RIFF/WAVE file", path.display()));
    }
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if &data[pos..pos + 4] == b"fmt " && size >= 16 && body + 4 <= data.len() {
            return Ok(u16::from_le_bytes(data[body + 2..body + 4].try_into().unwrap()) as u32);
        }
        pos = body + size + (size & 1);
    }
    Err(format!("{} has no fmt chunk", path.display()))
}

/// Interleave a 16-channel 24-bit 48 kHz WAV: the SLVS mono PCM stream goes on
/// channel 15, the leading channels carry `main_pcm` (already 16-channel-wide,
/// 24-bit interleaved) when present, everything else is silence. The output runs
/// at least `min_frames` picture frames long (at `fps`) so the sound covers the
/// picture; the SLVS stream is padded with silent blocks or, if longer, defines
/// the length. Writes a canonical PCM WAV.
pub fn write_16ch_slvs_wav(
    slvs_pcm: &[u8],
    main_pcm_16ch: Option<&[u8]>,
    min_frames: u64,
    fps: u32,
    output: &Path,
) -> Result<(), String> {
    let frame_bytes_16 = DCP_SOUND_CHANNELS * BYTES_PER_SAMPLE;
    let samples_per_frame = (SAMPLE_RATE / fps.max(1)) as usize;
    // total sample count is the max of: slvs length, main length, min picture len
    let slvs_samples = slvs_pcm.len() / BYTES_PER_SAMPLE;
    let main_samples = main_pcm_16ch.map(|m| m.len() / frame_bytes_16).unwrap_or(0);
    let min_samples = min_frames as usize * samples_per_frame;
    let total_samples = slvs_samples.max(main_samples).max(min_samples);

    let data_len = total_samples * frame_bytes_16;
    let mut out = vec![0u8; data_len];
    if let Some(main) = main_pcm_16ch {
        let n = main.len().min(data_len);
        out[..n].copy_from_slice(&main[..n]);
    }
    // splice the mono SLVS samples into channel 15 (index 14)
    let ch_off = SLVS_CHANNEL_INDEX * BYTES_PER_SAMPLE;
    for (s, sample) in slvs_pcm
        .as_chunks::<BYTES_PER_SAMPLE>()
        .0
        .iter()
        .enumerate()
    {
        let base = s * frame_bytes_16 + ch_off;
        out[base..base + BYTES_PER_SAMPLE].copy_from_slice(sample);
    }

    write_wav_24_48k(output, DCP_SOUND_CHANNELS as u16, &out)
}

/// Read the interleaved 24-bit PCM payload from a canonical 48 kHz WAV and widen
/// it to a 16-channel interleaved buffer (silent fill channels). Errors on any
/// non-48 kHz / non-24-bit / >16-channel input.
pub fn widen_wav_to_16ch(path: &Path) -> Result<Vec<u8>, String> {
    let data = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{} is not a RIFF/WAVE file", path.display()));
    }
    let mut pos = 12usize;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut rate = 0u32;
    let mut payload: &[u8] = &[];
    while pos + 8 <= data.len() {
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if body + size > data.len() {
            break;
        }
        match &data[pos..pos + 4] {
            b"fmt " if size >= 16 => {
                channels = u16::from_le_bytes(data[body + 2..body + 4].try_into().unwrap());
                rate = u32::from_le_bytes(data[body + 4..body + 8].try_into().unwrap());
                bits = u16::from_le_bytes(data[body + 14..body + 16].try_into().unwrap());
            }
            b"data" => payload = &data[body..body + size],
            _ => {}
        }
        pos = body + size + (size & 1);
    }
    if rate != SAMPLE_RATE || bits != 24 {
        return Err(format!(
            "sign-language sound expects 48 kHz 24-bit main audio, got {rate} Hz {bits}-bit"
        ));
    }
    if channels == 0 || channels as usize > DCP_SOUND_CHANNELS {
        return Err(format!("unsupported main-audio channel count {channels}"));
    }
    let src_frame = channels as usize * BYTES_PER_SAMPLE;
    let dst_frame = DCP_SOUND_CHANNELS * BYTES_PER_SAMPLE;
    let frames = payload.len() / src_frame;
    let mut out = vec![0u8; frames * dst_frame];
    for f in 0..frames {
        let src = &payload[f * src_frame..f * src_frame + src_frame];
        out[f * dst_frame..f * dst_frame + src_frame].copy_from_slice(src);
    }
    Ok(out)
}

/// Write a canonical PCM WAV (little-endian, format 1) with the given channel
/// count at 24-bit / 48 kHz from an interleaved sample buffer.
fn write_wav_24_48k(path: &Path, channels: u16, data: &[u8]) -> Result<(), String> {
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let byte_rate = SAMPLE_RATE * block_align as u32;
    let mut w = Vec::with_capacity(44 + data.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    w.extend_from_slice(&byte_rate.to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(data);
    std::fs::write(path, &w).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slvs_ul_matches_libdcp_and_asdcplib() {
        // 06 0e 2b 34 04 01 01 0d 0d 0f 03 02 01 01 00 00
        assert_eq!(
            SLVS_MCA_UL,
            [
                0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x0d, 0x0d, 0x0f, 0x03, 0x02, 0x01, 0x01,
                0x00, 0x00
            ]
        );
    }

    #[test]
    fn block_is_288000_bytes_with_correct_header() {
        assert_eq!(BLOCK_BYTES, 288_000);
        let header = vec![0xAAu8; 100];
        let seg = vec![0xBBu8; 5000];
        let block = pack_block(&header, &seg).unwrap();
        assert_eq!(block.len(), BLOCK_BYTES);
        // 5 big-endian u32 header
        assert_eq!(&block[0..4], &0xFFFF_FFFFu32.to_be_bytes());
        assert_eq!(&block[4..8], &(seg.len() as u32).to_be_bytes()); // Lv
        assert_eq!(&block[8..12], &(BLOCK_BYTES as u32).to_be_bytes()); // Lb
        assert_eq!(&block[12..16], &(header.len() as u32).to_be_bytes()); // Le
        assert_eq!(&block[16..20], &0xFFFF_FFFFu32.to_be_bytes());
        // header then segment then zero padding
        assert_eq!(&block[20..120], header.as_slice());
        assert_eq!(&block[120..5120], seg.as_slice());
        assert!(block[5120..].iter().all(|b| *b == 0));
    }

    #[test]
    fn pack_block_rejects_oversized_chunk() {
        let seg = vec![0u8; BLOCK_BYTES];
        assert!(pack_block(&[], &seg).is_err());
    }

    #[test]
    fn mca_config_places_slvs_on_channel_15() {
        let cfg = slvs_mca_config("51(L,R,C,LFE,Ls,Rs)", 6, "sgn-ase");
        // 5.1 group + 8 fill (7..14) + SLVS(15) + 1 fill (16)
        assert_eq!(cfg, "51(L,R,C,LFE,Ls,Rs),-,-,-,-,-,-,-,-,SLVS:sgn-ase,-");
        // 9 silent-fill channels (8 gap + trailing 16); count '-' channel tokens
        // rather than raw '-' chars, since the language tag "sgn-ase" has a hyphen
        assert_eq!(cfg.split(',').filter(|t| *t == "-").count(), 9);
        // SLVS sits at channel 15 (index 14 among the comma-separated channels,
        // with the soundfield group counting as its 6 members)
        assert!(cfg.contains(",SLVS:sgn-ase,-"));
    }

    #[test]
    fn write_16ch_puts_slvs_on_channel_15() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("s.wav");
        // two mono samples in the slvs stream: 0x010203, 0x040506
        let slvs = vec![0x03, 0x02, 0x01, 0x06, 0x05, 0x04];
        write_16ch_slvs_wav(&slvs, None, 0, 24, &out).unwrap();
        let widened = widen_wav_to_16ch(&out).unwrap();
        let frame = DCP_SOUND_CHANNELS * BYTES_PER_SAMPLE;
        let ch = SLVS_CHANNEL_INDEX * BYTES_PER_SAMPLE;
        assert_eq!(&widened[ch..ch + 3], &[0x03, 0x02, 0x01]);
        assert_eq!(&widened[frame + ch..frame + ch + 3], &[0x06, 0x05, 0x04]);
    }
}
