//! Conform assembly end-to-end: a tiny 2-reel CMX3600 EDL over synthetic media is
//! driven to a finished multi-reel DCP (per-reel encode + wrap + assembly). Fast
//! (a few frames per reel) but exercises the real grok encode + create + assemble
//! path, then verifies the output with dcpdoctor.

use dcpwizard_core::conform::{assemble_dcp, build_reel_plan, parse_timeline};
use dcpwizard_core::package_signature::PackageSigner;
use std::path::{Path, PathBuf};
use std::process::Command;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A short 2048x1080 24fps clip via ffmpeg testsrc.
fn make_clip(path: &Path, frames: u32) {
    let ok = Command::new("ffmpeg")
        .args(["-y", "-f", "lavfi", "-i", "testsrc=size=2048x1080:rate=24"])
        .args(["-frames:v", &frames.to_string(), "-pix_fmt", "yuv420p"])
        .arg(path)
        .output()
        .expect("run ffmpeg")
        .status
        .success();
    assert!(ok, "ffmpeg testsrc generation failed");
}

fn signer_chain(dir: &Path) -> PackageSigner {
    std::fs::create_dir_all(dir).unwrap();
    assert_eq!(
        postkit::certificate::generate_chain("Acme", dir),
        0,
        "chain generation failed"
    );
    PackageSigner {
        signer_cert: dir.join("signer.pem"),
        signer_key: dir.join("signer.key"),
        signer_chain: vec![dir.join("intermediate.pem"), dir.join("root.pem")],
    }
}

fn file_starting_with(dir: &Path, prefix: &str) -> PathBuf {
    let mut hits: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".xml"))
        })
        .collect();
    hits.sort();
    assert_eq!(hits.len(), 1, "expected one {prefix}*.xml in {dir:?}");
    hits.pop().unwrap()
}

fn xmlsec1_available() -> bool {
    Command::new("xmlsec1")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The delivered CPL and PKL each carry a ds:Signature that really verifies.
fn assert_signed(out: &Path, trusted_pem: &Path) {
    for prefix in ["CPL_", "PKL_"] {
        let doc = file_starting_with(out, prefix);
        let text = std::fs::read_to_string(&doc).unwrap();
        assert!(
            text.contains("</Signature>") || text.contains("</ds:Signature>"),
            "{} carries no ds:Signature",
            doc.display()
        );
        if !xmlsec1_available() {
            continue;
        }
        let result = Command::new("xmlsec1")
            .args(["--verify", "--trusted-pem"])
            .arg(trusted_pem)
            .arg(&doc)
            .output()
            .expect("run xmlsec1");
        assert!(
            result.status.success(),
            "xmlsec1 must verify {}\n  stderr: {}",
            doc.display(),
            String::from_utf8_lossy(&result.stderr).trim(),
        );
    }
}

/// One reel is moved straight out, so nothing downstream would add a signature.
#[test]
fn a_single_reel_conform_signs_the_dcp_it_moves_out() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping single-reel conform signing test");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let media = root.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    make_clip(&media.join("REEL001.mov"), 8);

    let edl = root.path().join("cut.edl");
    std::fs::write(
        &edl,
        "TITLE: Conform Signed\nFCM: NON-DROP FRAME\n\n\
         001  REEL001  V  C        00:00:00:00 00:00:00:06 00:00:00:00 00:00:00:06\n",
    )
    .unwrap();

    let timeline = parse_timeline(&edl).expect("parse edl");
    let plan = build_reel_plan(&timeline, &media).expect("resolve reels");
    let certs = root.path().join("certs");
    let signer = signer_chain(&certs);

    let out = root.path().join("dcp");
    assert_eq!(
        assemble_dcp(&plan, &out, Some(&signer)),
        0,
        "signed single-reel conform"
    );
    assert_signed(&out, &certs.join("root.pem"));

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}

/// More reels means `assemble` writes the delivered CPL and PKL, not `create`.
#[test]
fn a_multi_reel_conform_signs_the_assembled_cpl() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping multi-reel conform signing test");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let media = root.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    make_clip(&media.join("REEL001.mov"), 8);
    make_clip(&media.join("REEL002.mov"), 8);

    let edl = root.path().join("cut.edl");
    std::fs::write(
        &edl,
        "TITLE: Conform Signed Multi\nFCM: NON-DROP FRAME\n\n\
         001  REEL001  V  C        00:00:00:00 00:00:00:06 00:00:00:00 00:00:00:06\n\
         002  REEL002  V  C        00:00:00:00 00:00:00:06 00:00:00:06 00:00:00:12\n",
    )
    .unwrap();

    let timeline = parse_timeline(&edl).expect("parse edl");
    let plan = build_reel_plan(&timeline, &media).expect("resolve reels");
    let certs = root.path().join("certs");
    let signer = signer_chain(&certs);

    let out = root.path().join("dcp");
    assert_eq!(
        assemble_dcp(&plan, &out, Some(&signer)),
        0,
        "signed multi-reel conform"
    );
    assert_signed(&out, &certs.join("root.pem"));

    let cpl = std::fs::read_to_string(file_starting_with(&out, "CPL_")).unwrap();
    assert_eq!(cpl.matches("<Reel>").count(), 2, "two reels in the CPL");

    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}

/// A signer that cannot sign has to stop the run before any reel is encoded.
#[test]
fn an_unusable_signer_fails_before_encoding() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping unusable signer test");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let media = root.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    make_clip(&media.join("REEL001.mov"), 8);

    let edl = root.path().join("cut.edl");
    std::fs::write(
        &edl,
        "TITLE: Conform Bad Signer\nFCM: NON-DROP FRAME\n\n\
         001  REEL001  V  C        00:00:00:00 00:00:00:06 00:00:00:00 00:00:00:06\n",
    )
    .unwrap();

    let timeline = parse_timeline(&edl).expect("parse edl");
    let plan = build_reel_plan(&timeline, &media).expect("resolve reels");
    let signer = PackageSigner {
        signer_cert: root.path().join("nope.pem"),
        signer_key: root.path().join("nope.key"),
        signer_chain: vec![],
    };

    let out = root.path().join("dcp");
    assert_ne!(
        assemble_dcp(&plan, &out, Some(&signer)),
        0,
        "an unusable signer must fail the conform"
    );
    assert!(
        !out.join(".conform_work").exists(),
        "nothing should have been encoded"
    );
}

#[test]
fn two_reel_edl_conforms_to_a_dcp() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping conform assembly test");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let media = root.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    make_clip(&media.join("REEL001.mov"), 8);
    make_clip(&media.join("REEL002.mov"), 8);

    // two video reels, each trimmed to source frames 0..6
    let edl = root.path().join("cut.edl");
    std::fs::write(
        &edl,
        "TITLE: Conform Test\nFCM: NON-DROP FRAME\n\n\
         001  REEL001  V  C        00:00:00:00 00:00:00:06 00:00:00:00 00:00:00:06\n\
         002  REEL002  V  C        00:00:00:00 00:00:00:06 00:00:00:06 00:00:00:12\n",
    )
    .unwrap();

    let timeline = parse_timeline(&edl).expect("parse edl");
    let plan = build_reel_plan(&timeline, &media).expect("resolve reels");
    assert_eq!(plan.reels.len(), 2, "two resolved reels");

    let out = root.path().join("dcp");
    assert_eq!(assemble_dcp(&plan, &out, None), 0, "conform assembly");

    // the reel plan artifact is kept next to the assembled DCP
    // (written by the CLI, not assemble_dcp; assert the DCP structure instead)
    let cpls: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_") && n.ends_with(".xml"))
        })
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    assert_eq!(cpls.len(), 1, "one assembled CPL");
    assert_eq!(
        cpls[0].matches("<Reel>").count(),
        2,
        "assembled CPL has two reels"
    );

    // and the assembled OV is verify-clean
    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "dcpdoctor errors: {:?}", result.errors);
}
