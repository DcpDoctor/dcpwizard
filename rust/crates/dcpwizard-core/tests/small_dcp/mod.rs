//! A 24 frame 2K package built through the create path, small enough that a
//! test can build one per case and still read the picture back.

use dcpwizard_core::dcp::DcpConfig;
use std::path::{Path, PathBuf};

pub const FPS: u32 = 24;
pub const W: u32 = 2048;
pub const H: u32 = 1080;
pub const FRAMES: usize = 24;

/// 16 MB covers a single 4K J2K frame, and a read is bounded by it rather than
/// by the file, so a test never holds a whole essence.
const MAX_FRAME_BUF: usize = 16 * 1024 * 1024;

/// Encode one black J2K frame and copy it into `FRAMES` frames. Returns the
/// codestream bytes (the pre-encryption source frame 0).
pub fn make_frames(dir: &Path) -> Vec<u8> {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(W, H, FPS, &seed).expect("encode content frame");
    let bytes = std::fs::read(&seed).unwrap();
    for i in 0..FRAMES {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
    bytes
}

/// A stereo 48 kHz 24-bit WAV, `FRAMES` frames long.
pub fn make_wav(path: &Path) {
    let sample_rate = 48_000u32;
    let channels = 2u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let n_samples = FRAMES as u64 * (sample_rate as u64 / FPS as u64);
    let data_len = n_samples * block_align as u64;
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
    std::fs::write(path, &w).unwrap();
}

/// The package as the create path takes it. `key_out` asks for an encrypted
/// package and names the KEYS.json its content keys are written to.
pub fn base_config(out: &Path, j2k: PathBuf, audio: PathBuf, key_out: Option<&Path>) -> DcpConfig {
    DcpConfig {
        title: "Secret".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(j2k),
        audio_path: Some(audio),
        encrypt: key_out.is_some(),
        key_out: key_out.map(|path| path.to_path_buf()),
        ..Default::default()
    }
}

pub fn find_mxf(dir: &Path, prefix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".mxf"))
        })
}

/// Read frame 0 of a picture MXF as cleartext (no crypto context).
pub fn read_picture_frame0(mxf: &Path) -> Vec<u8> {
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&mxf.to_string_lossy())
        .expect("open picture");
    let mut buf = vec![0u8; MAX_FRAME_BUF];
    let n = reader
        .read_frame(0, &mut buf, None, None)
        .expect("read frame 0");
    buf.truncate(n);
    buf
}
