//! `create` with a signer must produce a CPL and PKL whose ds:Signature really
//! verifies, and a PKL whose CPL hash matches the signed file on disk. Without a
//! signer the package must come out exactly as before, unsigned.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use dcpwizard_core::package_signature::PackageSigner;
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 8;

fn make_frames(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let seed = dir.join("seed.j2c");
    dcpwizard_core::pad::generate_black_frame(WIDTH, HEIGHT, FPS, &seed).expect("encode frame");
    for i in 0..FRAMES {
        std::fs::copy(&seed, dir.join(format!("frame_{i:05}.j2c"))).unwrap();
    }
    std::fs::remove_file(&seed).unwrap();
}

fn signer_chain(dir: &Path) -> PackageSigner {
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

fn base_config(out: &Path, j2k: PathBuf) -> DcpConfig {
    DcpConfig {
        title: "SignedPackage".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(j2k),
        ..Default::default()
    }
}

fn only_file_matching(dir: &Path, prefix: &str) -> PathBuf {
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
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one {prefix}*.xml in {dir:?}"
    );
    hits.pop().unwrap()
}

/// The hash and size the PKL records for `asset_id`.
fn pkl_asset_record(pkl: &Path, asset_id: &str) -> (String, u64) {
    let text = std::fs::read_to_string(pkl).unwrap();
    let doc = roxmltree::Document::parse(&text).expect("parse PKL");
    let asset = doc
        .descendants()
        .filter(|n| n.has_tag_name("Asset"))
        .find(|n| {
            n.children()
                .find(|c| c.has_tag_name("Id"))
                .and_then(|c| c.text())
                .map(|t| t.trim().trim_start_matches("urn:uuid:").to_lowercase())
                == Some(asset_id.to_lowercase())
        })
        .unwrap_or_else(|| panic!("PKL has no asset {asset_id}"));
    let field = |name: &str| {
        asset
            .children()
            .find(|c| c.has_tag_name(name))
            .and_then(|c| c.text())
            .unwrap_or_else(|| panic!("PKL asset {asset_id} has no {name}"))
            .trim()
            .to_string()
    };
    (field("Hash"), field("Size").parse().expect("numeric Size"))
}

/// The CPL id the PKL and ASSETMAP share, taken from the CPL file itself.
fn cpl_id(cpl: &Path) -> String {
    let text = std::fs::read_to_string(cpl).unwrap();
    let doc = roxmltree::Document::parse(&text).expect("parse CPL");
    doc.root_element()
        .children()
        .find(|c| c.has_tag_name("Id"))
        .and_then(|c| c.text())
        .expect("CPL has an Id")
        .trim()
        .trim_start_matches("urn:uuid:")
        .to_lowercase()
}

fn xmlsec1_available() -> bool {
    std::process::Command::new("xmlsec1")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// xmlsec1 must accept the document against `trusted_pem`. The whole-document
/// enveloped profile needs no --id-attr hints.
fn assert_xmlsec1_verifies(doc: &Path, trusted_pem: &Path) {
    let result = std::process::Command::new("xmlsec1")
        .arg("--verify")
        .arg("--trusted-pem")
        .arg(trusted_pem)
        .arg(doc)
        .output()
        .expect("run xmlsec1");
    assert!(
        result.status.success(),
        "xmlsec1 must verify {}\n  status: {}\n  stdout: {}\n  stderr: {}",
        doc.display(),
        result.status,
        String::from_utf8_lossy(&result.stdout).trim(),
        String::from_utf8_lossy(&result.stderr).trim(),
    );
}

#[test]
fn signed_package_verifies_and_the_pkl_hash_matches_the_signed_cpl() {
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k);
    let certs = dir.path().join("certs");
    std::fs::create_dir_all(&certs).unwrap();
    let signer = signer_chain(&certs);

    let out = dir.path().join("dcp");
    let mut config = base_config(&out, j2k);
    config.signer = Some(signer.clone());
    assert_eq!(create_dcp(&config), 0, "signed create must succeed");

    let cpl = only_file_matching(&out, "CPL_");
    let pkl = only_file_matching(&out, "PKL_");
    let cpl_xml = std::fs::read_to_string(&cpl).unwrap();
    let pkl_xml = std::fs::read_to_string(&pkl).unwrap();
    assert!(cpl_xml.contains("<ds:Signature"), "CPL must be signed");
    assert!(pkl_xml.contains("<ds:Signature"), "PKL must be signed");

    // The signature is real, not a placeholder: it verifies against the leaf.
    postkit::xmldsig::verify_document_enveloped(&cpl_xml, Some(&signer.signer_cert))
        .expect("CPL signature must verify");
    postkit::xmldsig::verify_document_enveloped(&pkl_xml, Some(&signer.signer_cert))
        .expect("PKL signature must verify");

    // The ordering hazard: the PKL must hash the CPL as it is on disk, after
    // signing, not the unsigned bytes.
    let (recorded_hash, recorded_size) = pkl_asset_record(&pkl, &cpl_id(&cpl));
    assert_eq!(
        recorded_hash,
        dcpwizard_core::hash::hash_file(&cpl).unwrap(),
        "PKL hash must match the signed CPL on disk"
    );
    assert_eq!(
        recorded_size,
        std::fs::metadata(&cpl).unwrap().len(),
        "PKL size must match the signed CPL on disk"
    );

    // dcpdoctor over the finished package: no hash or signature errors.
    let report = dcpwizard_core::verify::verify_dcp(&out);
    let hash_or_signature_errors: Vec<&String> = report
        .errors
        .iter()
        .filter(|e| {
            let lower = e.to_lowercase();
            lower.contains("hash") || lower.contains("signature") || lower.contains("signed")
        })
        .collect();
    assert!(
        hash_or_signature_errors.is_empty(),
        "signed package must have no hash or signature errors, got {hash_or_signature_errors:?}"
    );

    if xmlsec1_available() {
        assert_xmlsec1_verifies(&cpl, &certs.join("root.pem"));
        assert_xmlsec1_verifies(&pkl, &certs.join("root.pem"));
    } else {
        eprintln!("skipping xmlsec1 cross-check: xmlsec1 not installed");
    }
}

#[test]
fn tampering_with_a_signed_cpl_breaks_its_signature() {
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k);
    let certs = dir.path().join("certs");
    std::fs::create_dir_all(&certs).unwrap();
    let signer = signer_chain(&certs);

    let out = dir.path().join("dcp");
    let mut config = base_config(&out, j2k);
    config.signer = Some(signer);
    assert_eq!(create_dcp(&config), 0);

    let cpl = only_file_matching(&out, "CPL_");
    let signed = std::fs::read_to_string(&cpl).unwrap();
    let tampered = signed.replacen("SignedPackage", "TamperedPackage", 1);
    assert_ne!(signed, tampered, "the tamper must change the document");

    let err = postkit::xmldsig::verify_document_enveloped(&tampered, None)
        .expect_err("a tampered CPL must not verify");
    assert!(err.contains("digest mismatch"), "got: {err}");

    if xmlsec1_available() {
        let tampered_path = out.join("CPL_tampered.xml");
        std::fs::write(&tampered_path, &tampered).unwrap();
        let ok = std::process::Command::new("xmlsec1")
            .arg("--verify")
            .arg("--trusted-pem")
            .arg(certs.join("root.pem"))
            .arg(&tampered_path)
            .output()
            .expect("run xmlsec1")
            .status
            .success();
        assert!(!ok, "xmlsec1 must reject the tampered CPL");
    }
}

#[test]
fn create_without_a_signer_writes_an_unsigned_package() {
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k);

    let out = dir.path().join("dcp");
    let config = base_config(&out, j2k);
    assert!(config.signer.is_none(), "no signer by default");
    assert_eq!(create_dcp(&config), 0);

    let cpl = only_file_matching(&out, "CPL_");
    let pkl = only_file_matching(&out, "PKL_");
    for path in [&cpl, &pkl, &out.join("ASSETMAP.xml")] {
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(
            !xml.contains("Signature"),
            "{} must be untouched by signing",
            path.display()
        );
    }

    // The write order is the same as before: the PKL still records the CPL as
    // it lies on disk.
    let (recorded_hash, recorded_size) = pkl_asset_record(&pkl, &cpl_id(&cpl));
    assert_eq!(
        recorded_hash,
        dcpwizard_core::hash::hash_file(&cpl).unwrap()
    );
    assert_eq!(recorded_size, std::fs::metadata(&cpl).unwrap().len());
}

#[test]
fn a_key_that_does_not_match_the_certificate_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k);
    let certs = dir.path().join("certs");
    std::fs::create_dir_all(&certs).unwrap();
    let mut signer = signer_chain(&certs);
    // the root's key, which does not belong to the signer leaf certificate
    signer.signer_key = certs.join("root.key");

    let out = dir.path().join("dcp");
    let mut config = base_config(&out, j2k);
    config.signer = Some(signer);
    assert_ne!(create_dcp(&config), 0, "a mismatched key must fail the run");
    assert!(
        !out.exists(),
        "nothing may be written when the signer is unusable"
    );
}

#[test]
fn every_cpl_of_a_versioned_package_is_signed_and_hashed_after_signing() {
    let dir = tempfile::tempdir().unwrap();
    let j2k = dir.path().join("j2k");
    make_frames(&j2k);
    let certs = dir.path().join("certs");
    std::fs::create_dir_all(&certs).unwrap();
    let signer = signer_chain(&certs);

    let out = dir.path().join("dcp");
    let mut config = base_config(&out, j2k);
    config.signer = Some(signer.clone());
    let versions: Vec<dcpwizard_core::versions::VersionSpec> = ["Original", "Alternate"]
        .into_iter()
        .map(|title| dcpwizard_core::versions::VersionSpec {
            title: title.into(),
            subtitle: None,
            subtitle_language: None,
            ccap: None,
            audio: None,
            kind: None,
        })
        .collect();
    assert_eq!(
        dcpwizard_core::versions::create_versioned_dcp(&config, &versions),
        0,
        "signed versioned create must succeed"
    );

    let pkl = only_file_matching(&out, "PKL_");
    let pkl_xml = std::fs::read_to_string(&pkl).unwrap();
    postkit::xmldsig::verify_document_enveloped(&pkl_xml, Some(&signer.signer_cert))
        .expect("PKL signature must verify");

    let cpls: Vec<PathBuf> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_") && n.ends_with(".xml"))
        })
        .collect();
    assert_eq!(cpls.len(), 2, "one CPL per version");
    for cpl in &cpls {
        let xml = std::fs::read_to_string(cpl).unwrap();
        postkit::xmldsig::verify_document_enveloped(&xml, Some(&signer.signer_cert))
            .unwrap_or_else(|e| panic!("{} signature must verify: {e}", cpl.display()));
        let (recorded_hash, _) = pkl_asset_record(&pkl, &cpl_id(cpl));
        assert_eq!(
            recorded_hash,
            dcpwizard_core::hash::hash_file(cpl).unwrap(),
            "PKL hash must match the signed {}",
            cpl.display()
        );
    }
}
