//! DCP combiner end-to-end: build small SMPTE DCPs with the real create path,
//! combine them, and validate the merged volume with the dcpdoctor engine
//! (0 errors). Covers dedupe, id-clash rejection, separate PKLs, sort +
//! annotation, an interop volume validating clean, and the interop loose
//! subtitle relocation mechanism (dom#2019/2026/2027/2420).

use dcpwizard_core::combine::{CombineConfig, combine};
use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const W: u32 = 2048;
const H: u32 = 1080;

fn make_content_frames(dir: &Path, count: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(W, H, FPS, &seed).expect("encode content frame");
    for i in 0..count {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

/// Build a small valid DCP at `out`.
fn make_dcp(
    root: &Path,
    name: &str,
    title: &str,
    frames: usize,
    standard: dcpwizard_core::Standard,
) -> PathBuf {
    let content = root.join(format!("{name}_j2k"));
    make_content_frames(&content, frames);
    let out = root.join(name);
    let config = DcpConfig {
        title: title.into(),
        standard,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.clone(),
        j2k_dir: Some(content),
        ..Default::default()
    };
    assert_eq!(create_dcp(&config), 0, "create {name} must succeed");
    out
}

fn list_cpls(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_") && n.ends_with(".xml"))
        })
        .collect()
}

fn read_assetmap(dir: &Path) -> String {
    for n in ["ASSETMAP.xml", "ASSETMAP"] {
        let p = dir.join(n);
        if p.exists() {
            return std::fs::read_to_string(p).unwrap();
        }
    }
    panic!("no ASSETMAP in {}", dir.display());
}

/// (id, hash) pairs from every PKL in the volume.
fn pkl_hashes(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("PKL") && name.ends_with(".xml")) {
            continue;
        }
        let content = std::fs::read_to_string(entry.path()).unwrap();
        for block in content.split("<Asset>").skip(1) {
            let block = block.split("</Asset>").next().unwrap_or("");
            let id = tag(block, "Id").map(|v| v.replace("urn:uuid:", ""));
            let hash = tag(block, "Hash");
            if let (Some(id), Some(hash)) = (id, hash) {
                out.push((id, hash));
            }
        }
    }
    out
}

/// id -> ASSETMAP path.
fn assetmap_paths(dir: &Path) -> std::collections::HashMap<String, String> {
    let xml = read_assetmap(dir);
    let mut map = std::collections::HashMap::new();
    for block in xml.split("<Asset>").skip(1) {
        let block = block.split("</Asset>").next().unwrap_or("");
        let id = tag(block, "Id").map(|v| v.replace("urn:uuid:", ""));
        let path = tag(block, "Path");
        if let (Some(id), Some(path)) = (id, path) {
            map.insert(id, path);
        }
    }
    map
}

fn tag(text: &str, t: &str) -> Option<String> {
    let open = format!("<{t}");
    let start = text.find(&open)?;
    let after = &text[start + open.len()..];
    let gt = after.find('>')?;
    let content = &after[gt + 1..];
    let end = content.find(&format!("</{t}>"))?;
    let v = content[..end].trim().to_string();
    (!v.is_empty()).then_some(v)
}

#[test]
fn combine_two_dcps_validates_and_keeps_both_cpls() {
    let root = tempfile::tempdir().unwrap();
    let a = make_dcp(
        root.path(),
        "a",
        "Alpha Movie",
        3,
        dcpwizard_core::Standard::Smpte,
    );
    let b = make_dcp(
        root.path(),
        "b",
        "Bravo Movie",
        4,
        dcpwizard_core::Standard::Smpte,
    );

    let out = root.path().join("combined");
    let cfg = CombineConfig {
        inputs: vec![a, b],
        output_dir: out.clone(),
        separate_pkls: false,
        sort: false,
        annotation: None,
        signer: None,
    };
    assert_eq!(combine(&cfg), 0, "combine must succeed");

    // both CPLs present
    let cpls = list_cpls(&out);
    assert_eq!(cpls.len(), 2, "both CPLs must be copied");

    // dcpdoctor: zero errors
    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(
        result.errors.is_empty(),
        "dcpdoctor errors: {:?}",
        result.errors
    );

    // every merged-PKL hash matches the actual file it maps to
    let paths = assetmap_paths(&out);
    for (id, hash) in pkl_hashes(&out) {
        let rel = paths
            .get(&id)
            .unwrap_or_else(|| panic!("id {id} not in ASSETMAP"));
        let actual = dcpwizard_core::hash::hash_file(&out.join(rel)).unwrap();
        assert_eq!(actual, hash, "hash mismatch for {id} ({rel})");
    }

    // single merged PKL by default
    let pkl_count = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("PKL") && n.ends_with(".xml")
        })
        .count();
    assert_eq!(pkl_count, 1, "default is one merged PKL");
}

#[test]
fn combine_dedupes_identical_assets() {
    let root = tempfile::tempdir().unwrap();
    let a = make_dcp(root.path(), "a", "Solo", 3, dcpwizard_core::Standard::Smpte);

    // combining a DCP with itself: every id repeats with an identical hash
    let out = root.path().join("combined");
    let cfg = CombineConfig {
        inputs: vec![a.clone(), a.clone()],
        output_dir: out.clone(),
        separate_pkls: false,
        sort: false,
        annotation: None,
        signer: None,
    };
    assert_eq!(combine(&cfg), 0);

    // each asset file exists exactly once (one CPL, one picture, one sound)
    let mxf = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "mxf"))
        .count();
    let src_mxf = std::fs::read_dir(&a)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "mxf"))
        .count();
    assert_eq!(mxf, src_mxf, "deduped, so no duplicate MXFs");
    assert_eq!(list_cpls(&out).len(), 1, "deduped to one CPL");

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(
        result.errors.is_empty(),
        "dcpdoctor errors: {:?}",
        result.errors
    );
}

#[test]
fn combine_rejects_id_clash() {
    let root = tempfile::tempdir().unwrap();
    let a = make_dcp(
        root.path(),
        "a",
        "Clash A",
        3,
        dcpwizard_core::Standard::Smpte,
    );
    // b is a copy of a, so it shares every asset id
    let b = root.path().join("b");
    copy_dir(&a, &b);

    // tamper one shared asset in b and rewrite b's PKL hash so the same id now
    // maps to different content
    let pic = std::fs::read_dir(&b)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("picture_"))
        })
        .expect("picture mxf");
    let old_hash = dcpwizard_core::hash::hash_file(&pic).unwrap();
    let mut bytes = std::fs::read(&pic).unwrap();
    bytes.extend_from_slice(b"tampered");
    std::fs::write(&pic, &bytes).unwrap();
    let new_hash = dcpwizard_core::hash::hash_file(&pic).unwrap();
    for entry in std::fs::read_dir(&b).unwrap().filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("PKL") && name.ends_with(".xml") {
            let content = std::fs::read_to_string(entry.path()).unwrap();
            std::fs::write(entry.path(), content.replace(&old_hash, &new_hash)).unwrap();
        }
    }

    let out = root.path().join("combined");
    let cfg = CombineConfig {
        inputs: vec![a, b],
        output_dir: out,
        separate_pkls: false,
        sort: false,
        annotation: None,
        signer: None,
    };
    assert_eq!(
        combine(&cfg),
        -1,
        "same id + different hash must be rejected"
    );
}

#[test]
fn combine_separate_pkls_keeps_each_input_pkl() {
    let root = tempfile::tempdir().unwrap();
    let a = make_dcp(
        root.path(),
        "a",
        "Alpha",
        3,
        dcpwizard_core::Standard::Smpte,
    );
    let b = make_dcp(
        root.path(),
        "b",
        "Bravo",
        3,
        dcpwizard_core::Standard::Smpte,
    );

    let out = root.path().join("combined");
    let cfg = CombineConfig {
        inputs: vec![a, b],
        output_dir: out.clone(),
        separate_pkls: true,
        sort: false,
        annotation: None,
        signer: None,
    };
    assert_eq!(combine(&cfg), 0);

    let pkl_count = std::fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("PKL") && n.ends_with(".xml")
        })
        .count();
    assert_eq!(pkl_count, 2, "one PKL per input");

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(
        result.errors.is_empty(),
        "dcpdoctor errors: {:?}",
        result.errors
    );
}

#[test]
fn combine_sort_orders_cpls_and_sets_annotation() {
    let root = tempfile::tempdir().unwrap();
    // input order is Zeta then Alpha; --sort must list Alpha first
    let z = make_dcp(root.path(), "z", "Zeta", 3, dcpwizard_core::Standard::Smpte);
    let al = make_dcp(
        root.path(),
        "al",
        "Alpha",
        3,
        dcpwizard_core::Standard::Smpte,
    );

    let out = root.path().join("combined");
    let cfg = CombineConfig {
        inputs: vec![z, al],
        output_dir: out.clone(),
        separate_pkls: false,
        sort: true,
        annotation: Some("Feature Reel".into()),
        signer: None,
    };
    assert_eq!(combine(&cfg), 0);

    let am = read_assetmap(&out);
    assert!(
        am.contains("<AnnotationText>Feature Reel</AnnotationText>"),
        "{am}"
    );

    // find the two CPL ids in ASSETMAP order and check Alpha precedes Zeta
    let cpls = list_cpls(&out);
    let mut alpha_id = None;
    let mut zeta_id = None;
    for cpl in &cpls {
        let c = std::fs::read_to_string(cpl).unwrap();
        let id = tag(&c, "Id").unwrap();
        if c.contains("<ContentTitleText>Alpha</ContentTitleText>") {
            alpha_id = Some(id);
        } else if c.contains("<ContentTitleText>Zeta</ContentTitleText>") {
            zeta_id = Some(id);
        }
    }
    let alpha_id = alpha_id.unwrap();
    let zeta_id = zeta_id.unwrap();
    assert!(
        am.find(&alpha_id).unwrap() < am.find(&zeta_id).unwrap(),
        "sorted ASSETMAP must list Alpha before Zeta"
    );
}

#[test]
fn combine_interop_volume_validates_clean() {
    let root = tempfile::tempdir().unwrap();
    let a = make_dcp(
        root.path(),
        "a",
        "Interop A",
        3,
        dcpwizard_core::Standard::Interop,
    );
    let b = make_dcp(
        root.path(),
        "b",
        "Interop B",
        3,
        dcpwizard_core::Standard::Interop,
    );

    let out = root.path().join("combined");
    let cfg = CombineConfig {
        inputs: vec![a, b],
        output_dir: out.clone(),
        separate_pkls: false,
        sort: false,
        annotation: None,
        signer: None,
    };
    assert_eq!(combine(&cfg), 0);
    assert!(out.join("ASSETMAP").exists(), "interop ASSETMAP filename");
    assert_eq!(list_cpls(&out).len(), 2);

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(
        result.errors.is_empty(),
        "dcpdoctor errors: {:?}",
        result.errors
    );
}

#[test]
fn combine_relocates_interop_loose_subtitles_without_touching_cpl() {
    let root = tempfile::tempdir().unwrap();
    // two synthetic interop inputs whose loose subtitle/font files collide by name
    let a = make_interop_subtitle_dcp(&root.path().join("a"), "Show A");
    let b = make_interop_subtitle_dcp(&root.path().join("b"), "Show B");

    let a_cpl_before = std::fs::read(a.dir.join("CPL_cpl.xml")).unwrap();
    let a_sub_before = std::fs::read(a.dir.join("sub.xml")).unwrap();

    let out = root.path().join("combined");
    let cfg = CombineConfig {
        inputs: vec![a.dir.clone(), b.dir.clone()],
        output_dir: out.clone(),
        separate_pkls: false,
        sort: false,
        annotation: None,
        signer: None,
    };
    assert_eq!(combine(&cfg), 0);

    let paths = assetmap_paths(&out);
    let a_sub_path = paths.get(&a.sub_id).expect("A subtitle in ASSETMAP");
    let b_sub_path = paths.get(&b.sub_id).expect("B subtitle in ASSETMAP");
    // each landed in its own per-CPL subdir, so identical filenames do not clash
    assert!(
        a_sub_path.contains('/') && a_sub_path.ends_with("/sub.xml"),
        "{a_sub_path}"
    );
    assert!(
        b_sub_path.contains('/') && b_sub_path.ends_with("/sub.xml"),
        "{b_sub_path}"
    );
    assert_ne!(
        a_sub_path, b_sub_path,
        "subtitles relocated to distinct subdirs"
    );

    // subtitle XML copied byte-identical (its relative font ref is untouched)
    assert_eq!(std::fs::read(out.join(a_sub_path)).unwrap(), a_sub_before);
    // the font sits next to the subtitle in the same subdir
    let sub_dir = out.join(a_sub_path).parent().unwrap().to_path_buf();
    assert!(
        sub_dir.join("font.ttf").exists(),
        "font moved alongside subtitle"
    );

    // CPL copied byte-identical (never rewritten)
    let a_cpl_path = paths.get(&a.cpl_id).expect("A CPL in ASSETMAP");
    assert_eq!(std::fs::read(out.join(a_cpl_path)).unwrap(), a_cpl_before);
}

struct InteropInput {
    dir: PathBuf,
    cpl_id: String,
    sub_id: String,
}

/// Hand-build a minimal interop DCP directory with a loose subtitle XML that
/// references a font by relative filename, plus a fake picture MXF. Not a fully
/// valid DCP (no real essence), but structurally enough to exercise combine's
/// ASSETMAP/PKL parsing and interop relocation.
fn make_interop_subtitle_dcp(dir: &Path, title: &str) -> InteropInput {
    std::fs::create_dir_all(dir).unwrap();
    let cpl_id = uuid::Uuid::new_v4().to_string();
    let pic_id = uuid::Uuid::new_v4().to_string();
    let sub_id = uuid::Uuid::new_v4().to_string();
    let font_id = uuid::Uuid::new_v4().to_string();
    let pkl_id = uuid::Uuid::new_v4().to_string();

    let cpl = format!(
        "<?xml version=\"1.0\"?>\n<CompositionPlaylist xmlns=\"http://www.digicine.com/PROTO-ASDCP-CPL-20040511#\">\n  <Id>urn:uuid:{cpl_id}</Id>\n  <ContentTitleText>{title}</ContentTitleText>\n  <ReelList><Reel><AssetList><MainPicture><Id>urn:uuid:{pic_id}</Id></MainPicture><MainSubtitle><Id>urn:uuid:{sub_id}</Id></MainSubtitle></AssetList></Reel></ReelList>\n</CompositionPlaylist>\n"
    );
    std::fs::write(dir.join("CPL_cpl.xml"), &cpl).unwrap();

    // interop subtitle XML references its font by filename (relative)
    let sub = "<?xml version=\"1.0\"?>\n<DCSubtitle Version=\"1.0\">\n  <LoadFont Id=\"F1\" URI=\"font.ttf\"/>\n  <Subtitle>Hi</Subtitle>\n</DCSubtitle>\n";
    std::fs::write(dir.join("sub.xml"), sub).unwrap();
    std::fs::write(dir.join("font.ttf"), format!("FONT-{title}")).unwrap();
    std::fs::write(dir.join("picture.mxf"), format!("PIC-{title}")).unwrap();

    let h = |p: &Path| dcpwizard_core::hash::hash_file(p).unwrap();
    let sz = |p: &Path| std::fs::metadata(p).unwrap().len();
    let cpl_p = dir.join("CPL_cpl.xml");
    let sub_p = dir.join("sub.xml");
    let font_p = dir.join("font.ttf");
    let pic_p = dir.join("picture.mxf");

    let pkl = format!(
        "<?xml version=\"1.0\"?>\n<PackingList xmlns=\"http://www.digicine.com/PROTO-ASDCP-PKL-20040311#\">\n  <Id>urn:uuid:{pkl_id}</Id>\n  <AssetList>\n    <Asset><Id>urn:uuid:{cpl_id}</Id><Hash>{}</Hash><Size>{}</Size><Type>text/xml</Type></Asset>\n    <Asset><Id>urn:uuid:{pic_id}</Id><Hash>{}</Hash><Size>{}</Size><Type>application/mxf</Type></Asset>\n    <Asset><Id>urn:uuid:{sub_id}</Id><Hash>{}</Hash><Size>{}</Size><Type>text/xml</Type></Asset>\n    <Asset><Id>urn:uuid:{font_id}</Id><Hash>{}</Hash><Size>{}</Size><Type>application/x-font-opentype</Type></Asset>\n  </AssetList>\n</PackingList>\n",
        h(&cpl_p),
        sz(&cpl_p),
        h(&pic_p),
        sz(&pic_p),
        h(&sub_p),
        sz(&sub_p),
        h(&font_p),
        sz(&font_p)
    );
    std::fs::write(dir.join(format!("PKL_{pkl_id}.xml")), pkl).unwrap();

    let am = format!(
        "<?xml version=\"1.0\"?>\n<AssetMap xmlns=\"http://www.digicine.com/PROTO-ASDCP-AM-20040311#\">\n  <Id>urn:uuid:{}</Id>\n  <AssetList>\n    <Asset><Id>urn:uuid:{pkl_id}</Id><PackingList>true</PackingList><ChunkList><Chunk><Path>PKL_{pkl_id}.xml</Path></Chunk></ChunkList></Asset>\n    <Asset><Id>urn:uuid:{cpl_id}</Id><ChunkList><Chunk><Path>CPL_cpl.xml</Path></Chunk></ChunkList></Asset>\n    <Asset><Id>urn:uuid:{pic_id}</Id><ChunkList><Chunk><Path>picture.mxf</Path></Chunk></ChunkList></Asset>\n    <Asset><Id>urn:uuid:{sub_id}</Id><ChunkList><Chunk><Path>sub.xml</Path></Chunk></ChunkList></Asset>\n    <Asset><Id>urn:uuid:{font_id}</Id><ChunkList><Chunk><Path>font.ttf</Path></Chunk></ChunkList></Asset>\n  </AssetList>\n</AssetMap>\n",
        uuid::Uuid::new_v4()
    );
    std::fs::write(dir.join("ASSETMAP"), am).unwrap();

    InteropInput {
        dir: dir.to_path_buf(),
        cpl_id,
        sub_id,
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().filter_map(|e| e.ok()) {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}
