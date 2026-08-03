//! Head/tail padding: black picture frames and digital silence.
//!
//! Adds leading and/or trailing black frames (and matching silence) to a DCP so
//! the program can carry a run-up or run-out. One black frame is encoded with the
//! grok pipeline and its codestream is repeated for every padded frame (a
//! frame-wrapped MXF may reuse the same codestream), so padding costs one encode
//! regardless of length. Audio is padded sample-accurately at frame edges,
//! reusing the WAV arithmetic in [`crate::reel`].

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Parse a pad duration into a whole number of frames at `fps`.
///
/// The value must carry an explicit unit: `f` for frames (e.g. `48f`) or `s` for
/// seconds (e.g. `2s`, `1.5s`). A bare number is rejected as ambiguous. Seconds
/// must land on a whole frame at `fps`; a fractional-frame duration is rejected
/// rather than silently rounded.
pub fn parse_pad_frames(spec: &str, fps: u32) -> Result<u64, String> {
    let fps = fps.max(1);
    let spec = spec.trim();
    let ambiguous =
        || format!("pad duration '{spec}' needs a unit: frames (e.g. 48f) or seconds (e.g. 2s)");
    let Some(unit) = spec.chars().last() else {
        return Err(ambiguous());
    };
    let number = &spec[..spec.len() - unit.len_utf8()];
    match unit {
        'f' | 'F' => {
            let frames: u64 = number
                .parse()
                .map_err(|_| format!("invalid frame count in pad duration '{spec}'"))?;
            Ok(frames)
        }
        's' | 'S' => {
            let seconds: f64 = number
                .parse()
                .map_err(|_| format!("invalid seconds in pad duration '{spec}'"))?;
            if !seconds.is_finite() || seconds < 0.0 {
                return Err(format!(
                    "pad duration '{spec}' must be a non-negative number"
                ));
            }
            let exact = seconds * fps as f64;
            let rounded = exact.round();
            if (exact - rounded).abs() > 1e-6 {
                return Err(format!(
                    "pad duration '{spec}' is {exact} frames at {fps} fps; use a whole-frame duration"
                ));
            }
            Ok(rounded as u64)
        }
        _ => Err(ambiguous()),
    }
}

/// Read a J2K codestream's stored pixel dimensions from its SIZ marker.
pub fn read_j2k_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let data = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let header = postkit::j2k::parse_j2k_header(&data)
        .ok_or_else(|| format!("cannot parse J2K header in {}", path.display()))?;
    if header.width == 0 || header.height == 0 {
        return Err(format!("{} reports a zero dimension", path.display()));
    }
    Ok((header.width, header.height))
}

/// Parse a pad/background colour into 16-bit per-channel RGB.
///
/// Accepts `#RRGGBB` or `RRGGBB` (8-bit hex sRGB). Each 8-bit value is expanded to
/// 16 bits by `v * 257` so `ff` -> `ffff`, matching how an 8-bit source decodes to
/// the rgb48 buffer the encoder feeds grok.
pub fn parse_pad_color(spec: &str) -> Result<[u16; 3], String> {
    let hex = spec.trim().strip_prefix('#').unwrap_or(spec.trim());
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "pad colour '{spec}' must be 6 hex digits (RRGGBB or #RRGGBB)"
        ));
    }
    let mut rgb = [0u16; 3];
    for (i, slot) in rgb.iter_mut().enumerate() {
        let v = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("invalid hex in pad colour '{spec}'"))?;
        *slot = v as u16 * 257;
    }
    Ok(rgb)
}

/// Encode one solid `rgb` frame at `width`x`height` to a J2K codestream at `out`,
/// using the same grok pipeline the encoder uses for content frames. The colour is
/// run through postkit's Rec.709 RGB -> DCI X'Y'Z' DCDM transform
/// (`colour::rgb_to_xyz_inplace`) before compression, so the stored codestream
/// carries the correct X'Y'Z' code values. Black (0,0,0) maps to all-zero samples.
pub fn generate_solid_frame(
    width: u32,
    height: u32,
    fps: u32,
    rgb: [u16; 3],
    out: &Path,
) -> Result<(), String> {
    use postkit::grok_encoder::{self, CompressParams, RawFrame};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let pixels = (width as usize) * (height as usize);
    // one pixel as rgb48be (6 bytes, each channel big-endian 16-bit)
    let mut px = [
        (rgb[0] >> 8) as u8,
        rgb[0] as u8,
        (rgb[1] >> 8) as u8,
        rgb[1] as u8,
        (rgb[2] >> 8) as u8,
        rgb[2] as u8,
    ];
    // apply the DCDM colour transform (Rec.709 -> DCI X'Y'Z') to the solid pixel,
    // then replicate the transformed sample across the whole frame
    postkit::colour::rgb_to_xyz_inplace(&mut px);
    let mut data = Vec::with_capacity(pixels * 6);
    for _ in 0..pixels {
        data.extend_from_slice(&px);
    }

    let params = CompressParams {
        frame_rate: fps.max(1) as u16,
        ..CompressParams::default()
    };

    let work = out
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".dcpwizard_solid_{}", uuid::Uuid::new_v4()));
    let cancel = Arc::new(AtomicBool::new(false));

    grok_encoder::initialize(0);
    let mut produced = false;
    let result = grok_encoder::encode_pipeline(
        &work,
        &params,
        1,
        &cancel,
        || {
            if produced {
                return None;
            }
            produced = true;
            Some(RawFrame::Packed {
                data: std::mem::take(&mut data),
                width,
                height,
                precision: 16,
                index: 0,
            })
        },
        |_p| {},
    );

    if !result.success {
        let _ = std::fs::remove_dir_all(&work);
        return Err(format!("solid frame encode failed: {}", result.error));
    }
    let frame = work.join("frame_00000000.j2c");
    let outcome = std::fs::rename(&frame, out)
        .or_else(|_| std::fs::copy(&frame, out).map(|_| ()))
        .map_err(|e| format!("cannot place solid frame at {}: {e}", out.display()));
    let _ = std::fs::remove_dir_all(&work);
    outcome
}

/// Encode one black frame (convenience for `generate_solid_frame` with rgb 0,0,0).
pub fn generate_black_frame(width: u32, height: u32, fps: u32, out: &Path) -> Result<(), String> {
    generate_solid_frame(width, height, fps, [0, 0, 0], out)
}

/// Write `src` padded with `head_samples` of silence before and `tail_samples`
/// after the PCM payload, sample-accurate at the given block alignment. Reuses
/// [`crate::reel::parse_wav`] so the byte arithmetic matches reel slicing.
pub fn pad_wav_with_silence(
    src: &Path,
    head_samples: u64,
    tail_samples: u64,
    out: &Path,
) -> Result<(), String> {
    let info = crate::reel::parse_wav(src)?;
    let ba = info.block_align as u64;
    let head_bytes = head_samples * ba;
    let tail_bytes = tail_samples * ba;
    let want_bytes = head_bytes + info.data_size + tail_bytes;

    let mut header = info.header.clone();
    let riff_size = (info.header.len() as u64 - 8) + want_bytes;
    header[4..8].copy_from_slice(&(riff_size as u32).to_le_bytes());
    let p = info.data_size_field_pos;
    header[p..p + 4].copy_from_slice(&(want_bytes as u32).to_le_bytes());

    let mut w = std::fs::File::create(out).map_err(|e| format!("cannot create {out:?}: {e}"))?;
    w.write_all(&header).map_err(|e| e.to_string())?;

    let zeros = vec![0u8; 1 << 16];
    let write_zeros = |mut n: u64, w: &mut std::fs::File| -> Result<(), String> {
        while n > 0 {
            let take = n.min(zeros.len() as u64) as usize;
            w.write_all(&zeros[..take]).map_err(|e| e.to_string())?;
            n -= take as u64;
        }
        Ok(())
    };

    write_zeros(head_bytes, &mut w)?;

    let mut r = std::fs::File::open(src).map_err(|e| e.to_string())?;
    r.seek(SeekFrom::Start(info.data_offset))
        .map_err(|e| e.to_string())?;
    let mut remaining = info.data_size;
    let mut buf = vec![0u8; 1 << 16];
    while remaining > 0 {
        let take = remaining.min(buf.len() as u64) as usize;
        r.read_exact(&mut buf[..take]).map_err(|e| e.to_string())?;
        w.write_all(&buf[..take]).map_err(|e| e.to_string())?;
        remaining -= take as u64;
    }

    write_zeros(tail_bytes, &mut w)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_unit_parses_directly() {
        assert_eq!(parse_pad_frames("48f", 24).unwrap(), 48);
        assert_eq!(parse_pad_frames("0f", 24).unwrap(), 0);
    }

    #[test]
    fn seconds_unit_converts_at_fps() {
        assert_eq!(parse_pad_frames("2s", 24).unwrap(), 48);
        assert_eq!(parse_pad_frames("1.5s", 24).unwrap(), 36);
    }

    #[test]
    fn bare_number_is_rejected_as_ambiguous() {
        assert!(parse_pad_frames("48", 24).is_err());
        assert!(parse_pad_frames("2.0", 24).is_err());
    }

    #[test]
    fn pad_color_parses_hex() {
        assert_eq!(parse_pad_color("#000000").unwrap(), [0, 0, 0]);
        assert_eq!(parse_pad_color("ffffff").unwrap(), [0xffff, 0xffff, 0xffff]);
        assert_eq!(parse_pad_color("#ff0000").unwrap(), [0xffff, 0, 0]);
        // 0x80 -> 0x80*257 = 0x8080
        assert_eq!(parse_pad_color("808080").unwrap(), [0x8080, 0x8080, 0x8080]);
    }

    #[test]
    fn pad_color_rejects_bad_input() {
        assert!(parse_pad_color("fff").is_err());
        assert!(parse_pad_color("#gggggg").is_err());
        assert!(parse_pad_color("red").is_err());
    }

    #[test]
    fn fractional_frame_seconds_is_rejected() {
        // 0.5s at 25fps is 12.5 frames: not representable, reject loudly
        assert!(parse_pad_frames("0.5s", 25).is_err());
    }

    #[test]
    fn pad_wav_prepends_and_appends_silence() {
        // 2-channel 16-bit 48k WAV of 100 samples with a nonzero ramp
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.wav");
        let channels = 2u16;
        let bits = 16u16;
        let block_align = (bits / 8) * channels;
        let n = 100u64;
        let mut data = Vec::new();
        for s in 0..n {
            for _ in 0..channels {
                data.extend_from_slice(&(((s + 1) & 0xffff) as u16).to_le_bytes());
            }
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&(48_000 * block_align as u32).to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        std::fs::write(&src, &wav).unwrap();

        let out = dir.path().join("padded.wav");
        // 10 samples of head silence, 20 of tail
        pad_wav_with_silence(&src, 10, 20, &out).unwrap();

        let info = crate::reel::parse_wav(&out).unwrap();
        let ba = info.block_align as u64;
        assert_eq!(info.data_size, (10 + n + 20) * ba);

        let bytes = std::fs::read(&out).unwrap();
        let start = info.data_offset as usize;
        // head is silent
        assert!(
            bytes[start..start + (10 * ba) as usize]
                .iter()
                .all(|b| *b == 0)
        );
        // first original sample sits right after the head silence
        let orig = start + (10 * ba) as usize;
        assert_eq!(u16::from_le_bytes([bytes[orig], bytes[orig + 1]]), 1);
        // tail is silent
        let tail = start + ((10 + n) * ba) as usize;
        assert!(
            bytes[tail..tail + (20 * ba) as usize]
                .iter()
                .all(|b| *b == 0)
        );
    }
}
