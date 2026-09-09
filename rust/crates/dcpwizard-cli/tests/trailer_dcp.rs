use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAME_RATE: u32 = 24;
const CONTENT_FRAMES: u32 = 3;
const COUNTDOWN_SECONDS: u32 = 1;
// postkit holds the ratings card for a fixed five seconds
const RATINGS_CARD_SECONDS: u32 = 5;

// the card is a flat band colour, so its channels land at the ends of the 8-bit range
const BAND_CHANNEL_FLOOR: u8 = 200;
const BAND_CHANNEL_CEILING: u8 = 60;

struct Band {
    flag: &'static str,
    // the card's sRGB channels, in R G B order
    lit: [bool; 3],
}

const BANDS: [Band; 2] = [
    Band {
        flag: "green",
        lit: [false, true, false],
    },
    Band {
        flag: "red",
        lit: [true, false, false],
    },
];

fn write_content(path: &Path) {
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=size={WIDTH}x{HEIGHT}:rate={FRAME_RATE}"),
            "-frames:v",
            &CONTENT_FRAMES.to_string(),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("ffmpeg has to run");
    assert!(
        made.status.success(),
        "ffmpeg could not write the trailer content: {}",
        String::from_utf8_lossy(&made.stderr)
    );
}

fn only_file_starting_with(directory: &Path, prefix: &str) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .expect("the package directory has to be readable")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    found.sort();
    assert_eq!(found.len(), 1, "one {prefix}* in {}", directory.display());
    found.remove(0)
}

// the display-RGB centre pixel the preview renders the packaged frame back to
fn preview_centre_pixel(picture_mxf: &Path, frame: u32, ppm: &Path) -> [u8; 3] {
    assert_eq!(
        postkit::preview::extract_frame(picture_mxf, frame, ppm, None),
        0,
        "frame {frame} has to render"
    );
    let data = std::fs::read(ppm).expect("the preview frame has to read");
    let mut offset = 0;
    for _ in 0..3 {
        offset += data[offset..]
            .iter()
            .position(|&byte| byte == b'\n')
            .expect("a P6 header has three lines")
            + 1;
    }
    let pixels = &data[offset..];
    let centre = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 3) as usize;
    [pixels[centre], pixels[centre + 1], pixels[centre + 2]]
}

#[test]
fn the_trailer_command_packages_a_card_and_a_leader_into_a_trailer_dcp() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let content = directory.path().join("content.mp4");
    write_content(&content);

    for band in &BANDS {
        let out = directory.path().join(format!("trailer_{}", band.flag));
        Command::cargo_bin("dcpwizard")
            .unwrap()
            .env("XDG_CONFIG_HOME", config_home.path())
            .args([
                "trailer",
                "--content",
                content.to_str().unwrap(),
                "--output",
                out.to_str().unwrap(),
                "--title",
                "Breadth Trailer",
                "--rating",
                "PG-13",
                "--rating-system",
                "mpaa",
                "--band",
                band.flag,
                "--countdown",
                &COUNTDOWN_SECONDS.to_string(),
                "--fps",
                &FRAME_RATE.to_string(),
            ])
            .assert()
            .success();

        let dcp = out.join("dcp");
        let picture_mxf = only_file_starting_with(&dcp, "picture_");
        let mut reader = asdcplib::jp2k::MxfReader::new();
        reader
            .open_read(&picture_mxf.to_string_lossy())
            .expect("the picture MXF has to open");
        let card_frames = RATINGS_CARD_SECONDS * FRAME_RATE;
        let leader_frames = COUNTDOWN_SECONDS * FRAME_RATE;
        assert_eq!(
            reader
                .picture_descriptor()
                .expect("picture descriptor")
                .container_duration,
            card_frames + leader_frames + CONTENT_FRAMES,
            "the {} band package must carry the card and the leader ahead of the content",
            band.flag
        );

        let ppm = directory.path().join("frame.ppm");
        let card = preview_centre_pixel(&picture_mxf, 0, &ppm);
        for (channel, lit) in band.lit.iter().enumerate() {
            if *lit {
                assert!(
                    card[channel] >= BAND_CHANNEL_FLOOR,
                    "the {} band card reads back as {card:?}",
                    band.flag
                );
            } else {
                assert!(
                    card[channel] <= BAND_CHANNEL_CEILING,
                    "the {} band card reads back as {card:?}",
                    band.flag
                );
            }
        }
        let first_content = preview_centre_pixel(&picture_mxf, card_frames + leader_frames, &ppm);
        assert_ne!(
            card, first_content,
            "the content frames must not be the card"
        );

        let cpl = std::fs::read_to_string(only_file_starting_with(&dcp, "CPL_")).unwrap();
        assert!(
            cpl.contains("<ContentKind>trailer</ContentKind>"),
            "the package must be a trailer composition: {cpl}"
        );
        let verified = dcpwizard_core::verify::verify_dcp(&dcp);
        assert!(verified.valid, "dcpdoctor errors: {:?}", verified.errors);
    }
}
