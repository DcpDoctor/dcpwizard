//! A DCP's picture must be an AS-DCP JPEG 2000 track file carrying a DCI cinema
//! codestream whose samples are X'Y'Z'. Build a flat pure-red Rec.709 DCP through
//! the real encode and wrap path, then assert that from the produced picture MXF
//! alone: essence type, codestream header, decoded centre pixel, and the sRGB
//! preview the wizard shows.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use postkit::colour::{ColourSpace, DcdmTransform};
use postkit::j2k::J2kProfile;
use std::path::{Path, PathBuf};

const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FPS: u32 = 24;
const FRAMES: usize = 4;

/// Pure Rec.709 red at full 16-bit scale, the source the DCDM transform is fed.
const SOURCE_RED: [u16; 3] = [65535, 0, 0];
/// DCI carries 12-bit samples.
const DCI_BIT_DEPTH: u8 = 12;
const DCI_MAX_CODE: u16 = 4095;
const DCI_COMPONENTS: usize = 3;
/// The 9/7 wavelet is lossy, so a decoded code lands near the computed one.
const CODE_TOLERANCE: i32 = 40;
/// Every X'Y'Z' component of red clears this. Two of three fall to zero if the
/// samples ever reach the codestream as RGB instead.
const XYZ_COMPONENT_FLOOR: i32 = 500;

/// An 8-bit sRGB preview of red: the red channel near full, the others near off.
const PREVIEW_RED_FLOOR: u8 = 200;
const PREVIEW_OTHER_CEILING: u8 = 80;

fn red_frame_dir(dir: &Path) -> PathBuf {
    let j2k = dir.join("j2k");
    std::fs::create_dir_all(&j2k).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_solid_frame(WIDTH, HEIGHT, FPS, SOURCE_RED, &seed)
        .expect("encode red frame");
    for i in 0..FRAMES {
        std::fs::copy(&seed, j2k.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
    j2k
}

/// Package the red frames into a real DCP and return the picture MXF.
fn red_picture_mxf(dir: &Path) -> PathBuf {
    let out = dir.join("dcp");
    let config = DcpConfig {
        title: "RedReadback".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(red_frame_dir(dir)),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "create_dcp must succeed");

    std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("picture") && n.ends_with(".mxf"))
        })
        .expect("picture MXF")
}

fn decode_centre_pixel(mxf: &Path) -> [i32; DCI_COMPONENTS] {
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&mxf.to_string_lossy())
        .expect("open picture mxf");
    let mut buf = vec![0u8; 16 * 1024 * 1024];
    let n = reader
        .read_frame(0, &mut buf, None, None)
        .expect("read frame 0");
    buf.truncate(n);

    let frame = postkit::grok_decoder::decode(buf, 0).expect("decode frame 0");
    assert_eq!(frame.precision, DCI_BIT_DEPTH);
    assert_eq!(frame.components.len(), DCI_COMPONENTS);
    let centre = (frame.height / 2 * frame.width + frame.width / 2) as usize;
    [
        frame.components[0][centre],
        frame.components[1][centre],
        frame.components[2][centre],
    ]
}

#[test]
fn the_produced_picture_is_a_dci_cinema_xyz_track_file() {
    let dir = tempfile::tempdir().unwrap();
    let mxf = red_picture_mxf(dir.path());

    assert_eq!(
        asdcplib::essence_type(&mxf.to_string_lossy()).expect("essence type"),
        asdcplib::EssenceType::Jpeg2000,
        "a DCP picture is an AS-DCP track file, not an AS-02 one"
    );

    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader
        .open_read(&mxf.to_string_lossy())
        .expect("open picture mxf");
    let codestream = reader
        .picture_descriptor()
        .expect("picture descriptor")
        .codestream;

    let profile = J2kProfile::from(codestream.rsize);
    assert!(
        profile.is_dci_cinema(),
        "RSIZ {:#06x} reads as {profile:?}, so the wrapped codestream is not a \
         DCI cinema profile and its samples are not X'Y'Z'",
        codestream.rsize
    );
    assert_eq!((codestream.xsize, codestream.ysize), (WIDTH, HEIGHT));
    assert_eq!(codestream.components.len(), DCI_COMPONENTS);
    for (i, component) in codestream.components.iter().enumerate() {
        assert_eq!(
            component.bit_depth(),
            DCI_BIT_DEPTH,
            "component {i} must be 12 bit"
        );
        assert_eq!(
            (component.x_rsize, component.y_rsize),
            (1, 1),
            "component {i} must be unsubsampled"
        );
    }

    let decoded = decode_centre_pixel(&mxf);
    let expected = DcdmTransform::to_xyz(ColourSpace::Rec709)
        .expect("rec709 dcdm transform")
        .pixel(SOURCE_RED, DCI_MAX_CODE);

    for (i, (&got, &want)) in decoded.iter().zip(expected.iter()).enumerate() {
        assert!(
            got > XYZ_COMPONENT_FLOOR,
            "component {i} decoded {got} of {DCI_MAX_CODE}. X'Y'Z' red is about \
             {expected:?}, so every component is large; an RGB encode of the same \
             red would give about (4095, 0, 0)"
        );
        assert!(
            (got - want as i32).abs() <= CODE_TOLERANCE,
            "component {i} decoded {got}, expected {want} within {CODE_TOLERANCE}. \
             Whole pixel {decoded:?} against {expected:?}; an RGB encode of the same \
             red would give about (4095, 0, 0)"
        );
    }
}

#[test]
fn the_preview_turns_the_picture_back_into_red() {
    let dir = tempfile::tempdir().unwrap();
    let mxf = red_picture_mxf(dir.path());
    let ppm = dir.path().join("frame.ppm");

    assert_eq!(
        postkit::preview::extract_frame(&mxf, 0, &ppm, None),
        0,
        "extract_frame must render the DCP picture"
    );

    let data = std::fs::read(&ppm).expect("read ppm");
    // P6 header: magic, dimensions, maxval, each followed by one whitespace byte
    let body = data
        .iter()
        .position(|&b| b == b'\n')
        .and_then(|first| {
            let rest = &data[first + 1..];
            rest.iter().position(|&b| b == b'\n').map(|d| first + 1 + d)
        })
        .and_then(|second| {
            let rest = &data[second + 1..];
            rest.iter()
                .position(|&b| b == b'\n')
                .map(|d| second + 1 + d + 1)
        })
        .expect("ppm header");

    let pixel_count = (WIDTH * HEIGHT) as usize;
    assert_eq!(data.len() - body, pixel_count * 3, "ppm pixel data length");
    let centre = body + ((HEIGHT / 2 * WIDTH + WIDTH / 2) as usize) * 3;
    let rgb = [data[centre], data[centre + 1], data[centre + 2]];
    assert!(
        rgb[0] >= PREVIEW_RED_FLOOR
            && rgb[1] <= PREVIEW_OTHER_CEILING
            && rgb[2] <= PREVIEW_OTHER_CEILING,
        "preview pixel {rgb:?} is not red, so the X'Y'Z' inverse did not run"
    );
}
