use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAME_RATE: u32 = 24;
const MEDIA_FRAMES: u32 = 8;
const REEL_FRAMES: u32 = 6;
const REELS: usize = 2;
const DCI_PRECISION_BITS: u8 = 12;
const CODESTREAM_BUFFER_BYTES: usize = 4 * 1024 * 1024;

const XMEML_TITLE: &str = "Xmeml Cut";
const FCPXML_TITLE: &str = "Fcpxml Cut";

fn write_media(directory: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    for reel in ["REEL001", "REEL002"] {
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
                &MEDIA_FRAMES.to_string(),
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(directory.join(format!("{reel}.mov")))
            .output()
            .expect("ffmpeg has to run");
        assert!(
            made.status.success(),
            "ffmpeg could not write {reel}: {}",
            String::from_utf8_lossy(&made.stderr)
        );
    }
}

// clipitem name is the reel, in/out are source frames and start/end record frames
fn write_xmeml(path: &Path) {
    std::fs::write(
        path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<xmeml version="5">
  <sequence>
    <name>{XMEML_TITLE}</name>
    <rate><timebase>{FRAME_RATE}</timebase><ntsc>FALSE</ntsc></rate>
    <media>
      <video>
        <track>
          <clipitem>
            <name>REEL001</name>
            <rate><timebase>{FRAME_RATE}</timebase></rate>
            <start>0</start><end>{REEL_FRAMES}</end>
            <in>0</in><out>{REEL_FRAMES}</out>
            <file id="f1"><name>REEL001.mov</name></file>
          </clipitem>
          <clipitem>
            <name>REEL002</name>
            <start>{REEL_FRAMES}</start><end>{}</end>
            <in>0</in><out>{REEL_FRAMES}</out>
            <file id="f2"><name>REEL002.mov</name></file>
          </clipitem>
        </track>
      </video>
    </media>
  </sequence>
</xmeml>
"#,
            REEL_FRAMES * 2
        ),
    )
    .unwrap();
}

// asset-clip ref resolves to the asset, and its name is the reel the media dir is searched for
fn write_fcpxml(path: &Path) {
    std::fs::write(
        path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<fcpxml version="1.10">
  <resources>
    <format id="r1" name="FFVideoFormat" frameDuration="100/2400s" width="{WIDTH}" height="{HEIGHT}"/>
    <asset id="r2" name="REEL001" start="0s" duration="{MEDIA_FRAMES}/{FRAME_RATE}s" hasVideo="1" format="r1">
      <media-rep kind="original-media" src="file:///media/REEL001.mov"/>
    </asset>
    <asset id="r3" name="REEL002" start="0s" duration="{MEDIA_FRAMES}/{FRAME_RATE}s" hasVideo="1" format="r1">
      <media-rep kind="original-media" src="file:///media/REEL002.mov"/>
    </asset>
  </resources>
  <library>
    <event name="Conform Event">
      <project name="{FCPXML_TITLE}">
        <sequence format="r1" duration="{}/{FRAME_RATE}s" tcStart="0s" tcFormat="NDF">
          <spine>
            <asset-clip ref="r2" offset="0/{FRAME_RATE}s" name="REEL001" start="0/{FRAME_RATE}s" duration="{REEL_FRAMES}/{FRAME_RATE}s" format="r1"/>
            <asset-clip ref="r3" offset="{REEL_FRAMES}/{FRAME_RATE}s" name="REEL002" start="0/{FRAME_RATE}s" duration="{REEL_FRAMES}/{FRAME_RATE}s" format="r1"/>
          </spine>
        </sequence>
      </project>
    </event>
  </library>
</fcpxml>
"#,
            REEL_FRAMES * 2
        ),
    )
    .unwrap();
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

fn assert_reel_picture_decodes(picture_mxf: &Path) {
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&picture_mxf.to_string_lossy())
        .expect("the picture MXF has to open");
    let descriptor = reader.picture_descriptor().expect("picture descriptor");
    assert_eq!(
        descriptor.container_duration, REEL_FRAMES,
        "a reel carries the frames the timeline trimmed it to"
    );
    let mut buffer = vec![0u8; CODESTREAM_BUFFER_BYTES];
    let read = reader
        .read_frame(0, &mut buffer, None, None)
        .expect("the first codestream has to read");
    let frame = postkit::grok_decoder::decode(buffer[..read].to_vec(), 0)
        .expect("the codestream has to decode");
    assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT));
    assert_eq!(frame.precision, DCI_PRECISION_BITS);
}

fn conform_to_dcp(timeline: &Path, media: &Path, out: &Path, config_home: &Path) {
    Command::cargo_bin("dcpwizard")
        .unwrap()
        .env("XDG_CONFIG_HOME", config_home)
        .args([
            "conform",
            "--input",
            timeline.to_str().unwrap(),
            "--media-dir",
            media.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();
}

fn assert_conformed(out: &Path, title: &str, format: &str) {
    let manifest = std::fs::read_to_string(out.join("conform_manifest.json"))
        .expect("the conform manifest is kept as an artifact");
    assert!(
        manifest.contains(&format!("\"format\": \"{format}\"")),
        "the manifest must name the {format} parser: {manifest}"
    );
    let plan = std::fs::read_to_string(out.join("conform_plan.json"))
        .expect("the reel plan is kept as an artifact");
    assert!(
        plan.contains("REEL001") && plan.contains("REEL002"),
        "{plan}"
    );

    let cpl = std::fs::read_to_string(only_file_starting_with(out, "CPL_")).unwrap();
    assert!(
        cpl.contains(&format!("<ContentTitleText>{title}<")),
        "the timeline's own title has to reach the CPL: {cpl}"
    );
    assert_eq!(
        cpl.matches("<Reel>").count(),
        REELS,
        "one reel per timeline event: {cpl}"
    );

    let mut pictures: Vec<PathBuf> = std::fs::read_dir(out)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("picture_"))
        })
        .collect();
    pictures.sort();
    assert_eq!(pictures.len(), REELS, "one picture track file per reel");
    for picture in &pictures {
        assert_reel_picture_decodes(picture);
    }

    let verified = dcpwizard_core::verify::verify_dcp(out);
    assert!(verified.valid, "dcpdoctor errors: {:?}", verified.errors);
}

#[test]
fn an_xmeml_timeline_conforms_to_a_multi_reel_dcp() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let media = directory.path().join("media");
    write_media(&media);
    let timeline = directory.path().join("cut.xml");
    write_xmeml(&timeline);
    let out = directory.path().join("dcp");

    conform_to_dcp(&timeline, &media, &out, config_home.path());
    assert_conformed(&out, XMEML_TITLE, "XmlFcp");
}

#[test]
fn an_fcpxml_timeline_conforms_to_a_multi_reel_dcp() {
    let directory = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let media = directory.path().join("media");
    write_media(&media);
    let timeline = directory.path().join("cut.fcpxml");
    write_fcpxml(&timeline);
    let out = directory.path().join("dcp");

    conform_to_dcp(&timeline, &media, &out, config_home.path());
    assert_conformed(&out, FCPXML_TITLE, "XmlFcpx");
}
