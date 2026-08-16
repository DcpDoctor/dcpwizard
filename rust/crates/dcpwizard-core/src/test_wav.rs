//! Synthetic WAVs for the unit tests that slice, shift or pad audio.

use std::path::Path;

/// Write a 48 kHz 16-bit WAV of `frames` samples per channel whose value ramps
/// with the sample index, so a test can tell which part of the source survived.
pub(crate) fn write_ramp_wav(path: &Path, channels: u16, samples: u64) {
    const BITS: u16 = 16;
    const SAMPLE_RATE: u32 = 48_000;
    let block_align = (BITS / 8) * channels;
    let mut data = Vec::new();
    for sample in 0..samples {
        for _ in 0..channels {
            data.extend_from_slice(&((sample & 0xffff) as u16).to_le_bytes());
        }
    }
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * block_align as u32).to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&BITS.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(path, &wav).unwrap();
}
