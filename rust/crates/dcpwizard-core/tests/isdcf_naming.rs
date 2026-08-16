//! End-to-end ISDCF naming: a created DCP's CPL carries the built content title,
//! the ratings and the content version label.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::isdcf_name::{
    IsdcfDate, IsdcfNameInput, Rating, SoundtrackChannel, isdcf_name,
};
use dcpwizard_core::isdcf_title::{IsdcfNamingOptions, isdcf_title, soundtrack_summary};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 8;
const SAMPLE_RATE: u32 = 48_000;
const MPAA_AGENCY: &str = "http://www.mpaa.org/2003-ratings";
const DATE: IsdcfDate = IsdcfDate {
    year: 2026,
    month: 8,
    day: 16,
};

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A short testsrc clip encoded to J2K frames through the real create pipeline's
/// input shape: a codestream directory.
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

/// A stereo 24-bit 48 kHz clip as long as the picture, written by ffmpeg so the
/// sound comes from the same tool the create path demuxes with.
fn make_stereo_wav(path: &Path) -> bool {
    let seconds = FRAMES as f64 / FPS as f64;
    std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=1000:duration={seconds}:sample_rate={SAMPLE_RATE}"),
            "-ac",
            "2",
            "-c:a",
            "pcm_s24le",
        ])
        .arg(path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        && path.exists()
}

fn read_cpl(dir: &Path) -> String {
    let path = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_"))
        })
        .expect("CPL written");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_named_dcp_carries_its_isdcf_title_ratings_and_content_version() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("stereo.wav");
    if !make_stereo_wav(&audio) {
        eprintln!("skipping: ffmpeg could not synthesize the test sound");
        return;
    }
    let out = dir.path().join("dcp");

    let mut config = DcpConfig {
        title: "My Film".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        container_width: WIDTH,
        container_height: HEIGHT,
        output_dir: out.clone(),
        j2k_dir: Some(make_frames(&dir.path().join("frames"))),
        audio_path: Some(audio.clone()),
        audio_language: Some("en".into()),
        ratings: vec![Rating {
            agency: MPAA_AGENCY.into(),
            label: "PG-13".into(),
        }],
        content_versions: vec!["Final Cut".into()],
        ..Default::default()
    };

    let options = IsdcfNamingOptions {
        date: Some(DATE),
        ..Default::default()
    };
    let sound = soundtrack_summary(2, None, None);
    let name = isdcf_title(&config, &options, &sound, false);
    config.title = name.clone();

    // the same name built by hand from the same facts, so the mapping is pinned
    // by something other than the mapping
    let expected = isdcf_name(&IsdcfNameInput {
        title: "My Film".into(),
        content_type: dcpwizard_core::ContentType::Test,
        version_number: 1,
        content_versions: vec!["Final Cut".into()],
        frame_rate: FPS,
        container_size: (WIDTH, HEIGHT),
        audio_language: Some("en".into()),
        ratings: vec![Rating {
            agency: MPAA_AGENCY.into(),
            label: "PG-13".into(),
        }],
        soundtrack_channels: vec![SoundtrackChannel::Left, SoundtrackChannel::Right],
        resolution: dcpwizard_core::Resolution::TwoK,
        date: Some(DATE),
        standard: dcpwizard_core::Standard::Smpte,
        ..Default::default()
    });
    assert_eq!(name, expected);
    assert!(name.starts_with("MyFilm_TST"), "{name}");
    assert!(name.contains("_20_"), "stereo is named 20: {name}");
    assert!(
        name.contains("_EN-XX_"),
        "English audio with no text track: {name}"
    );

    assert_eq!(create_dcp(&config), 0);

    let cpl = read_cpl(&out);
    assert!(
        cpl.contains(&format!("<ContentTitleText>{name}</ContentTitleText>")),
        "{cpl}"
    );
    assert!(cpl.contains("<LabelText>Final Cut</LabelText>"), "{cpl}");
    assert!(
        cpl.contains(&format!("<Agency>{MPAA_AGENCY}</Agency>")),
        "{cpl}"
    );
    assert!(cpl.contains("<Label>PG-13</Label>"), "{cpl}");

    // the sound MXF declares the audio language on every MCA label
    let sound_mxf = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("sound_"))
        })
        .expect("sound MXF written");
    let mut reader = asdcplib::pcm::MxfReader::new();
    reader.open_read(sound_mxf.to_str().unwrap()).unwrap();
    let languages: Vec<Option<String>> = reader
        .mca_label_subdescriptors()
        .unwrap()
        .into_iter()
        .map(|label| label.spoken_language)
        .collect();
    assert!(
        !languages.is_empty() && languages.iter().all(|lang| lang.as_deref() == Some("en")),
        "every MCA label carries the audio language, got {languages:?}"
    );
}
