//! An encrypted DCP must not ship a track the wrap layer cannot encrypt.
//! Picture and sound are encrypted at wrap time; timed text and Atmos are not,
//! so a package carrying one of those with --encrypt is refused instead of
//! delivering that track in the clear.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 4;
const SRT: &str = "1\n00:00:00,100 --> 00:00:00,150\nHello\n\n";

fn make_frames(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(WIDTH, HEIGHT, FPS, &seed).expect("encode frame");
    for i in 0..FRAMES {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
    dir.to_path_buf()
}

fn make_wav(path: &Path) -> PathBuf {
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
    path.to_path_buf()
}

fn base_config(root: &Path, out: &Path) -> DcpConfig {
    DcpConfig {
        title: "Track Test".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(make_frames(&root.join("frames"))),
        audio_path: Some(make_wav(&root.join("audio.wav"))),
        ..Default::default()
    }
}

fn mxf_with_prefix(dir: &Path, prefix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".mxf"))
        })
}

fn picture_is_encrypted(mxf: &Path) -> bool {
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&mxf.to_string_lossy())
        .expect("open picture");
    reader.writer_info().expect("writer info").encrypted_essence
}

fn sound_is_encrypted(mxf: &Path) -> bool {
    let mut reader = asdcplib::pcm::MxfReader::new();
    reader
        .open_read(&mxf.to_string_lossy())
        .expect("open sound");
    reader.writer_info().expect("writer info").encrypted_essence
}

#[test]
fn an_encrypted_dcp_refuses_a_subtitle_it_cannot_encrypt() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    let srt = dir.path().join("sub.srt");
    std::fs::write(&srt, SRT).unwrap();

    let config = DcpConfig {
        encrypt: true,
        key_out: Some(dir.path().join("KEYS.json")),
        subtitle_path: Some(srt),
        ..base_config(dir.path(), &out)
    };
    assert_ne!(
        create_dcp(&config),
        0,
        "a cleartext subtitle must be refused"
    );
    assert!(
        mxf_with_prefix(&out, "subtitle").is_none(),
        "no subtitle MXF may be written"
    );
}

#[test]
fn an_encrypted_dcp_refuses_atmos_it_cannot_encrypt() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    let atmos = dir.path().join("atmos.bin");
    std::fs::write(&atmos, [0u8; 64]).unwrap();

    let config = DcpConfig {
        encrypt: true,
        key_out: Some(dir.path().join("KEYS.json")),
        atmos_path: Some(atmos),
        ..base_config(dir.path(), &out)
    };
    assert_ne!(create_dcp(&config), 0, "cleartext Atmos must be refused");
    assert!(
        mxf_with_prefix(&out, "atmos").is_none(),
        "no Atmos MXF may be written"
    );
}

#[test]
fn an_encrypted_dcp_without_those_tracks_still_encrypts_picture_and_sound() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    let config = DcpConfig {
        encrypt: true,
        key_out: Some(dir.path().join("KEYS.json")),
        ..base_config(dir.path(), &out)
    };
    assert_eq!(create_dcp(&config), 0, "the guard must not fire here");
    assert!(picture_is_encrypted(
        &mxf_with_prefix(&out, "picture").expect("picture MXF")
    ));
    assert!(sound_is_encrypted(
        &mxf_with_prefix(&out, "sound").expect("sound MXF")
    ));
}

#[test]
fn an_unencrypted_dcp_still_packages_a_subtitle() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    let srt = dir.path().join("sub.srt");
    std::fs::write(&srt, SRT).unwrap();

    let config = DcpConfig {
        subtitle_path: Some(srt),
        ..base_config(dir.path(), &out)
    };
    assert_eq!(create_dcp(&config), 0, "an unencrypted DCP is unchanged");
    assert!(
        mxf_with_prefix(&out, "subtitle").is_some(),
        "the subtitle track is still packaged"
    );
}
