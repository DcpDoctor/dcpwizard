//! End-to-end sign-language video (ISDCF Doc 13): a synthetic sign video is
//! VP9-packed onto channel 15 of the sound track, the DCP validates, the CPL
//! carries the SignLanguageVideo ExtensionMetadata, and the SLVS MCA label reads
//! back via asdcplib.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::pad::generate_black_frame;
use dcpwizard_core::probe::ffmpeg_has_encoder;
use dcpwizard_core::sign_language::{BLOCK_BYTES, DCP_SOUND_CHANNELS, build_slvs_sound};
use std::path::Path;

const FPS: u32 = 24;
const W: u32 = 2048;
const H: u32 = 1080;

/// Synthesize a 2-second test video (any size; the encoder rescales to 480x640).
fn make_sign_video(path: &Path) {
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=320x240:rate=24",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("run ffmpeg");
    assert!(
        ok.status.success() && path.exists(),
        "ffmpeg wrote no sign language clip\n  stdout: {}\n  stderr: {}",
        String::from_utf8_lossy(&ok.stdout).trim(),
        String::from_utf8_lossy(&ok.stderr).trim(),
    );
}

/// A short 6-channel 24-bit 48 kHz ramp WAV; build_slvs_sound widens it to 16ch
/// and pads it to the SLVS length.
fn write_6ch_wav(path: &Path, samples: u64) {
    let channels = 6u16;
    let block_align = 3 * channels;
    let sample_rate = 48_000u32;
    let mut data = Vec::new();
    for s in 0..samples {
        for _ in 0..channels {
            let v = ((s % 100) + 1) as i32;
            data.extend_from_slice(&v.to_le_bytes()[..3]);
        }
    }
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&24u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    std::fs::write(path, &w).unwrap();
}

fn make_content_frames(dir: &Path, count: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    generate_black_frame(W, H, FPS, &seed).expect("encode content frame");
    for i in 0..count {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

fn read_cpl(dir: &Path) -> String {
    let cpl = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_") && n.ends_with(".xml"))
        })
        .expect("CPL written");
    std::fs::read_to_string(cpl).unwrap()
}

fn find_sound_mxf(dir: &Path) -> std::path::PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("sound_") && n.ends_with(".mxf"))
        })
        .expect("sound MXF written")
}

/// De-interleave channel 15 (index 14) from a 16-channel 24-bit WAV data chunk.
fn channel_15_bytes(wav: &Path, take: usize) -> Vec<u8> {
    let data = std::fs::read(wav).unwrap();
    // find the data chunk
    let mut pos = 12;
    let mut body = 0;
    let mut size = 0;
    while pos + 8 <= data.len() {
        let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if &data[pos..pos + 4] == b"data" {
            body = pos + 8;
            size = sz;
            break;
        }
        pos += 8 + sz + (sz & 1);
    }
    let frame = DCP_SOUND_CHANNELS * 3;
    let ch_off = 14 * 3;
    let mut out = Vec::new();
    let payload = &data[body..body + size];
    let mut f = 0;
    while out.len() < take && (f * frame + ch_off + 3) <= payload.len() {
        out.extend_from_slice(&payload[f * frame + ch_off..f * frame + ch_off + 3]);
        f += 1;
    }
    out
}

#[test]
fn sign_language_dcp_labels_channel_15_and_validates() {
    assert!(
        ffmpeg_has_encoder("libvpx-vp9"),
        "this ffmpeg has no libvpx-vp9 encoder"
    );
    let dir = tempfile::tempdir().unwrap();
    let sign = dir.path().join("sign.mp4");
    make_sign_video(&sign);
    let main_wav = dir.path().join("main.wav");
    write_6ch_wav(&main_wav, 100);

    // build the combined 16-channel SLVS sound track
    let combined = dir.path().join("slvs_sound.wav");
    let main_channels =
        build_slvs_sound(&sign, Some(&main_wav), 0, FPS, &combined).expect("build slvs sound");
    assert_eq!(main_channels, 6);

    // channel 15 carries the packing header: MARKER, Lv, Lb=288000, Le, MARKER
    let ch15 = channel_15_bytes(&combined, 20);
    assert_eq!(&ch15[0..4], &0xFFFF_FFFFu32.to_be_bytes(), "leading MARKER");
    assert_eq!(
        &ch15[8..12],
        &(BLOCK_BYTES as u32).to_be_bytes(),
        "block length Lb"
    );
    assert_eq!(
        &ch15[16..20],
        &0xFFFF_FFFFu32.to_be_bytes(),
        "trailing MARKER"
    );

    // picture must span the whole sound track
    let samples = std::fs::metadata(&combined).unwrap().len() as usize - 44;
    let frames = samples / (DCP_SOUND_CHANNELS * 3) / 2000;
    assert!(
        frames >= 48,
        "expected >= one 2s block of picture, got {frames}"
    );
    let content = dir.path().join("j2k");
    make_content_frames(&content, frames);

    let out = dir.path().join("dcp");
    let config = DcpConfig {
        title: "SLVS Test".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        audio_path: Some(combined),
        sign_language_lang: Some("sgn-ase".into()),
        sign_language_main_channels: Some(6),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "create must succeed");

    // CPL carries the ISDCF SignLanguageVideo ExtensionMetadata + language tag
    let cpl = read_cpl(&out);
    assert!(
        cpl.contains("scope=\"http://isdcf.com/2017/10/SignLanguageVideo\""),
        "CPL missing SLVS scope:\n{cpl}"
    );
    assert!(cpl.contains("<meta:Name>Language Tag</meta:Name>"), "{cpl}");
    assert!(cpl.contains("<meta:Value>sgn-ase</meta:Value>"), "{cpl}");

    // SLVS MCA label reads back: 6 main channels + 1 SLVS = 7 channel labels
    let mxf = find_sound_mxf(&out);
    let mut reader = asdcplib::pcm::MxfReader::new();
    reader
        .open_read(mxf.to_str().unwrap())
        .expect("open sound MXF");
    let mca = reader.mca_labels().expect("read mca labels");
    assert!(
        mca.has_mca_channel_assignment,
        "MCA ChannelAssignment UL: {mca:?}"
    );
    assert_eq!(mca.soundfield_groups, 1, "one 5.1 group: {mca:?}");
    assert_eq!(mca.channel_labels, 7, "6 main + 1 SLVS: {mca:?}");

    // the package validates
    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(
        result.errors.is_empty(),
        "verify errors: {:?}",
        result.errors
    );
}
