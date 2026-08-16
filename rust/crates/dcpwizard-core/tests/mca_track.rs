//! End-to-end MCA labelling: a 6-channel WAV wraps to a PCM MXF that carries
//! ST 429-12 / 377-4 channel-label subdescriptors, read back via asdcplib.

use dcpwizard_core::mxf_wrap::{MxfType, MxfWrapConfig, build_mca_config, wrap_mxf_result};
use std::io::Write;

/// Minimal RIFF/WAVE with `channels` 24-bit 48 kHz channels and one frame of
/// silence per channel, enough for asdcplib to wrap.
fn write_wav(path: &std::path::Path, channels: u16) {
    let sample_rate: u32 = 48_000;
    let bits: u16 = 24;
    let block_align = (bits / 8) * channels;
    let byte_rate = sample_rate * block_align as u32;
    // 2000 sample frames of silence
    let data_len = block_align as u32 * 2000;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
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
    w.extend_from_slice(&data_len.to_le_bytes());
    w.extend_from_slice(&vec![0u8; data_len as usize]);
    std::fs::File::create(path).unwrap().write_all(&w).unwrap();
}

#[test]
fn six_channel_wav_wraps_with_mca_labels() {
    // the auto-derived config for 6 channels is 5.1
    assert_eq!(
        build_mca_config(6, 6, None, None).as_deref(),
        Some("51(L,R,C,LFE,Ls,Rs)")
    );

    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("sound.wav");
    write_wav(&wav, 6);
    let mxf = dir.path().join("sound.mxf");

    // no explicit mca_config: the wrap auto-derives 5.1 from the 6 channels
    let track = wrap_mxf_result(&MxfWrapConfig {
        input_path: wav,
        output_mxf: mxf.clone(),
        mxf_type: MxfType::PcmAudio,
        frame_rate: 24,
        ..Default::default()
    })
    .expect("6ch PCM wrap");
    assert!(mxf.exists(), "sound MXF written: {track:?}");

    // read the MCA subdescriptors back out of the MXF header
    let mut reader = asdcplib::pcm::MxfReader::new();
    reader
        .open_read(mxf.to_str().unwrap())
        .expect("open sound MXF");
    let mca = reader.mca_labels().expect("read mca labels");
    assert!(
        mca.has_mca_channel_assignment,
        "WaveAudioDescriptor must carry the MCA ChannelAssignment UL"
    );
    assert_eq!(
        mca.soundfield_groups, 1,
        "one 5.1 soundfield group, got {mca:?}"
    );
    assert_eq!(
        mca.channel_labels, 6,
        "six AudioChannelLabelSubDescriptors, got {mca:?}"
    );
}
