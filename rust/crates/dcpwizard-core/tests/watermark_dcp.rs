//! End-to-end `watermark`: build a small DCP, mark its picture essence through
//! the transcode path at the source's own bandwidth, then decode frame 0 of the
//! output and prove the mark is in the picture, that the rest of the frame came
//! through, and that the package around it still names every asset.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dcpwizard_core::watermark::{WatermarkOptions, watermark_burn};
use postkit::subtitle_formats::VAlign;

/// The reel asset elements that name a track file, so each one owes a Hash.
const HASHED_REEL_ASSETS: [&str; 2] = ["MainPicture", "MainSound"];

mod small_dcp;
use small_dcp::{FPS, H, W, base_config, find_mxf, make_frames, make_wav, read_picture_frame0};

const MARK_TEXT: &str = "SCREENER DIST-001";

/// Text height as a percent of the frame height, well above the default so the
/// band it draws in is unmistakable.
const MARK_FONT_SIZE_PERCENT: f32 = 8.0;

/// Turns that percent back into the ratio postkit's own defaults are in.
const PERCENT_DIVISOR: f32 = 100.0;

/// Code values a sample may move by where the mark is not drawn, 0.8% of the
/// 12-bit range. The mark takes bits the rest of the frame was allocated, so
/// the picture around it is quantised more coarsely and reconstructs a little
/// off: with no mark the same re-encode comes back unchanged, which
/// `re_encoding_the_picture_without_a_mark_leaves_it_alone` holds to.
const REENCODE_TOLERANCE: i32 = 32;

/// How far a marked sample has to move to count as drawn on, a quarter of the
/// 12-bit range, so a codec artefact can never be mistaken for the mark.
const MARK_MINIMUM_RISE: i32 = 1024;

/// Samples the mark has to raise, about the count a line of text at
/// `MARK_FONT_SIZE_PERCENT` covers on a 2K frame.
const MARK_MINIMUM_SAMPLES: usize = 500;

/// The first row the mark can reach: its line box, plus a line height of glyph
/// overhang and drop shadow, above the margin it is anchored at.
fn first_marked_row() -> usize {
    let line_height = MARK_FONT_SIZE_PERCENT / PERCENT_DIVISOR
        * postkit::subtitle_raster::DEFAULT_LINE_HEIGHT_RATIO;
    let from_bottom = postkit::subtitle_raster::DEFAULT_MARGIN_RATIO + line_height * 2.0;
    ((1.0 - from_bottom) * H as f32).floor() as usize
}

/// The three components of frame 0 of a picture MXF, decoded.
fn decode_frame0(mxf: &Path) -> Vec<Vec<i32>> {
    let codestream = read_picture_frame0(mxf);
    let decoded = postkit::grok_decoder::decode(codestream, 0).expect("decode frame 0");
    assert_eq!(
        (decoded.width, decoded.height),
        (W, H),
        "the marked picture keeps the source raster"
    );
    decoded.components
}

fn cpl_xml(dcp_dir: &Path) -> String {
    let cpl = std::fs::read_dir(dcp_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_"))
        })
        .expect("CPL written");
    std::fs::read_to_string(cpl).unwrap()
}

fn strip_urn(id: &str) -> String {
    id.trim().trim_start_matches("urn:uuid:").to_lowercase()
}

fn child_text(node: roxmltree::Node, tag: &str) -> Option<String> {
    node.children()
        .find(|c| c.has_tag_name(tag))
        .and_then(|c| c.text())
        .map(str::to_string)
}

fn pkl_path(dcp_dir: &Path) -> PathBuf {
    std::fs::read_dir(dcp_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("PKL_"))
        })
        .expect("PKL written")
}

/// Asset id to Hash for every reel asset that names a track file, each Hash
/// asserted present and non empty as it is read.
fn cpl_reel_asset_hashes(cpl_text: &str) -> BTreeMap<String, String> {
    let doc = roxmltree::Document::parse(cpl_text).expect("parse CPL");
    let mut hashes = BTreeMap::new();
    for asset in doc
        .descendants()
        .filter(|n| HASHED_REEL_ASSETS.contains(&n.tag_name().name()))
    {
        let element = asset.tag_name().name();
        let id = child_text(asset, "Id")
            .map(|id| strip_urn(&id))
            .unwrap_or_else(|| panic!("CPL {element} without an Id"));
        let hash = child_text(asset, "Hash")
            .unwrap_or_else(|| panic!("CPL {element} {id} carries no Hash"));
        assert!(
            !hash.trim().is_empty(),
            "CPL {element} {id} has an empty Hash"
        );
        hashes.insert(id, hash.trim().to_string());
    }
    assert_eq!(
        hashes.len(),
        HASHED_REEL_ASSETS.len(),
        "expected a picture and a sound asset in the marked CPL"
    );
    hashes
}

fn pkl_asset_hashes(pkl_text: &str) -> BTreeMap<String, String> {
    let doc = roxmltree::Document::parse(pkl_text).expect("parse PKL");
    doc.descendants()
        .filter(|n| n.has_tag_name("Asset"))
        .filter_map(|asset| {
            let id = child_text(asset, "Id")?;
            let hash = child_text(asset, "Hash")?;
            Some((strip_urn(&id), hash.trim().to_string()))
        })
        .collect()
}

fn sound_asset_id(dcp_dir: &Path) -> String {
    let cpls = dcpwizard_core::multi_cpl::list_cpls(dcp_dir);
    let cpl = cpls.first().expect("a CPL to read the timeline from");
    let timeline = dcpwizard_core::multi_cpl::get_timeline(&dcp_dir.join(&cpl.file_path));
    let reel = timeline.first().expect("a reel");
    assert!(!reel.sound_asset_id.is_empty(), "the source has sound");
    reel.sound_asset_id.clone()
}

#[test]
fn the_watermark_command_marks_the_picture_and_ships_the_rest_of_the_dcp() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let j2k = root.join("frames");
    make_frames(&j2k);
    let wav = root.join("audio.wav");
    make_wav(&wav);
    let source_dcp = root.join("source");
    assert_eq!(
        dcpwizard_core::dcp::create_dcp(&base_config(&source_dcp, j2k, wav, None)),
        0,
        "build the DCP to be marked"
    );
    let source_picture = find_mxf(&source_dcp, "picture").expect("source picture");
    let source_frame = decode_frame0(&source_picture);

    let options = WatermarkOptions {
        text: MARK_TEXT.into(),
        font_size_percent: MARK_FONT_SIZE_PERCENT,
        position: VAlign::Bottom,
        ..Default::default()
    };
    let mark = watermark_burn(&options, None, f64::from(FPS)).expect("a system font to draw with");

    // no bandwidth named: the mark is the only thing meant to change, so the
    // re-encode targets the source picture's own average
    let marked_dcp = root.join("marked");
    assert_eq!(
        dcpwizard_core::j2k_transcode::transcode_dcp(
            &dcpwizard_core::j2k_transcode::DcpTranscodeConfig {
                input_dir: source_dcp.clone(),
                output_dir: marked_dcp.clone(),
                watermark: Some(mark),
                ..Default::default()
            }
        ),
        0,
        "marking the DCP must succeed"
    );

    // the package around the picture: a fresh CPL, PKL and ASSETMAP, with the
    // source's own sound track shipped under its own asset id
    assert!(
        std::fs::read_dir(&marked_dcp)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("PKL_")),
        "the marked DCP needs a PKL"
    );
    assert!(
        marked_dcp.join("ASSETMAP.xml").exists() || marked_dcp.join("ASSETMAP").exists(),
        "the marked DCP needs an ASSETMAP"
    );
    let marked_cpl = cpl_xml(&marked_dcp);
    let carried_sound = sound_asset_id(&source_dcp);
    assert!(
        marked_cpl.contains(&carried_sound),
        "the source sound asset {carried_sound} has to be carried over"
    );
    assert!(
        find_mxf(&marked_dcp, "sound").is_some(),
        "the sound essence has to ship with the marked picture"
    );

    // every reel asset carries the same hash the PKL ships it under, so a server
    // hash-checking against the CPL has something to check
    let pkl_text = std::fs::read_to_string(pkl_path(&marked_dcp)).unwrap();
    let pkl_hashes = pkl_asset_hashes(&pkl_text);
    for (id, cpl_hash) in cpl_reel_asset_hashes(&marked_cpl) {
        let pkl_hash = pkl_hashes
            .get(&id)
            .unwrap_or_else(|| panic!("the PKL ships no asset {id}"));
        assert_eq!(&cpl_hash, pkl_hash, "CPL and PKL disagree on asset {id}");
    }

    let cpl_doc = roxmltree::Document::parse(&marked_cpl).expect("parse marked CPL");
    let cpl_title = cpl_doc
        .descendants()
        .find(|n| n.has_tag_name("ContentTitleText"))
        .and_then(|n| n.text())
        .expect("the marked CPL names its content title")
        .to_string();
    let pkl_doc = roxmltree::Document::parse(&pkl_text).expect("parse marked PKL");
    let pkl_annotation = pkl_doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name("AnnotationText"))
        .and_then(|n| n.text())
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        pkl_annotation, cpl_title,
        "the PKL AnnotationText has to match the CPL's content title"
    );

    let result = dcpwizard_core::verify::verify_dcp(&marked_dcp);
    assert!(
        result.valid,
        "the marked DCP must validate: {:?}",
        result.errors
    );

    // the picture itself: the mark in its band, the picture above it untouched
    let marked_frame = decode_frame0(&find_mxf(&marked_dcp, "picture").expect("marked picture"));
    assert_eq!(marked_frame.len(), source_frame.len());

    let width = W as usize;
    for (component, (before, after)) in source_frame.iter().zip(&marked_frame).enumerate() {
        let mut raised = 0usize;
        for row in 0..H as usize {
            for column in 0..width {
                let at = row * width + column;
                let moved = after[at] - before[at];
                if row < first_marked_row() {
                    assert!(
                        moved.abs() <= REENCODE_TOLERANCE,
                        "component {component} row {row} column {column} moved by {moved}, more \
                         than the {REENCODE_TOLERANCE} code values a re-encode of an unmarked \
                         row can explain"
                    );
                }
                if moved >= MARK_MINIMUM_RISE {
                    raised += 1;
                    assert!(
                        row >= first_marked_row(),
                        "component {component} row {row} was drawn on, above the mark's band \
                         starting at row {}",
                        first_marked_row()
                    );
                }
            }
        }
        assert!(
            raised >= MARK_MINIMUM_SAMPLES,
            "the mark raised {raised} samples of component {component}, fewer than the \
             {MARK_MINIMUM_SAMPLES} a line of text covers"
        );
    }
}

/// The tolerance the marked frame is read back against is about the bits the
/// mark takes, not about the transcode itself: the same re-encode with no mark
/// leaves the picture where it was.
#[test]
fn re_encoding_the_picture_without_a_mark_leaves_it_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let j2k = root.join("frames");
    make_frames(&j2k);
    let wav = root.join("audio.wav");
    make_wav(&wav);
    let source_dcp = root.join("source");
    assert_eq!(
        dcpwizard_core::dcp::create_dcp(&base_config(&source_dcp, j2k, wav, None)),
        0,
        "build the DCP to re-encode"
    );
    let source_frame = decode_frame0(&find_mxf(&source_dcp, "picture").expect("source picture"));

    let plain = root.join("plain");
    assert_eq!(
        dcpwizard_core::j2k_transcode::transcode_dcp(
            &dcpwizard_core::j2k_transcode::DcpTranscodeConfig {
                input_dir: source_dcp,
                output_dir: plain.clone(),
                ..Default::default()
            }
        ),
        0,
        "re-encoding at the source bandwidth must succeed"
    );
    let plain_frame = decode_frame0(&find_mxf(&plain, "picture").expect("re-encoded picture"));

    for (component, (before, after)) in source_frame.iter().zip(&plain_frame).enumerate() {
        let worst = before
            .iter()
            .zip(after)
            .map(|(before, after)| (after - before).abs())
            .max()
            .unwrap_or(0);
        assert!(
            worst <= REENCODE_TOLERANCE,
            "component {component} moved by {worst} code values with nothing drawn on it"
        );
    }
}
