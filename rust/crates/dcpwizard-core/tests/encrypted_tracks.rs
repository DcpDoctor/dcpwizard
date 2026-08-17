//! `--encrypt` covers every essence. Picture, sound, subtitle, closed caption
//! and Atmos each get their own content key, every CPL asset block declares its
//! KeyId, the keys file carries all five, and the KDM built from it lists one
//! key per essence with the ST 430-1 type string that essence uses.

use dcpwizard_core::dcp::{DcpConfig, create_dcp};
use postkit::certificate::{CertOptions, CertType, generate_certificate, generate_chain};
use std::path::{Path, PathBuf};

const FPS: u32 = 24;
const WIDTH: u32 = 2048;
const HEIGHT: u32 = 1080;
const FRAMES: usize = 4;
const SRT: &str = "1\n00:00:00,100 --> 00:00:00,150\nHello\n\n";

/// Schema fault this package has with or without encryption: postkit's KDM
/// omits NonCriticalExtensions.
const KNOWN_SCHEMA_VIOLATIONS: [&str; 1] = ["}AuthenticatedPublic': Missing child element(s)"];

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

fn make_wav(path: &Path) -> PathBuf {
    let sample_rate = 48_000u32;
    let channels = 2u16;
    let bits = 24u16;
    let block_align = (bits / 8) * channels;
    let n_samples = FRAMES as u64 * (sample_rate as u64 / FPS as u64);
    let data_len = n_samples * block_align as u64;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data_len as u32).to_le_bytes());
    w.resize(w.len() + data_len as usize, 0);
    std::fs::write(path, &w).unwrap();
    path.to_path_buf()
}

/// One Atmos input file per picture frame, which is what the aux-data wrap and
/// its frame-count check expect.
fn make_atmos(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    for i in 0..FRAMES {
        std::fs::write(dir.join(format!("atmos_{i:05}.bin")), [i as u8; 64]).unwrap();
    }
    dir.to_path_buf()
}

fn base_config(root: &Path, out: &Path) -> DcpConfig {
    DcpConfig {
        title: "Track Test".into(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Test,
        frame_rate_num: FPS,
        frame_rate_den: 1,
        output_dir: out.to_path_buf(),
        j2k_dir: Some(make_frames(&root.join("frames"))),
        audio_path: Some(make_wav(&root.join("audio.wav"))),
        subtitle_opts: dcpwizard_core::subtitle::SubtitleOptions {
            // a font in the repo, so the package does not depend on the machine's
            font_path: Some(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/LiberationSans-Regular.ttf"),
            ),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn mxf_with_prefix(dir: &Path, prefix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".mxf"))
        })
}

/// Whether the MXF header flags its essence as encrypted, read with the reader
/// that matches the essence kind.
fn essence_is_encrypted(mxf: &Path, prefix: &str) -> bool {
    let name = mxf.to_string_lossy().to_string();
    let info = match prefix {
        "picture" => {
            let mut r = asdcplib::jp2k::MxfReader::new();
            r.open_read(&name).expect("open picture");
            r.writer_info()
        }
        "sound" => {
            let mut r = asdcplib::pcm::MxfReader::new();
            r.open_read(&name).expect("open sound");
            r.writer_info()
        }
        "atmos" => {
            let mut r = asdcplib::atmos::MxfReader::new();
            r.open_read(&name).expect("open atmos");
            r.writer_info()
        }
        _ => {
            let mut r = asdcplib::timed_text::MxfReader::new();
            r.open_read(&name).expect("open timed text");
            r.writer_info()
        }
    };
    info.expect("writer info").encrypted_essence
}

/// The body of the first `<element>` block in the CPL.
fn cpl_asset_block<'a>(cpl: &'a str, element: &str) -> &'a str {
    let open = cpl
        .find(&format!("<{element}"))
        .unwrap_or_else(|| panic!("CPL has no {element} block"));
    let close = cpl[open..]
        .find(&format!("</{element}>"))
        .unwrap_or_else(|| panic!("CPL has no closing {element}"));
    &cpl[open..open + close]
}

/// Generate a recipient leaf cert + key under `dir` issued by the chain root.
fn recipient(dir: &Path, chain_dir: &Path) -> (PathBuf, PathBuf) {
    let cert = dir.join("screen.pem");
    let key = dir.join("screen.key");
    let opts = CertOptions {
        cert_type: CertType::Leaf,
        common_name: "screen".into(),
        organization: "Cinema".into(),
        output_cert: cert.clone(),
        output_key: key.clone(),
        issuer_cert: chain_dir.join("root.pem"),
        issuer_key: chain_dir.join("root.key"),
        ..Default::default()
    };
    assert_eq!(generate_certificate(&opts), 0, "recipient cert gen failed");
    (cert, key)
}

#[test]
fn an_encrypted_dcp_keys_every_essence_and_the_kdm_carries_them_all() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let out = root.join("dcp");
    let srt = root.join("sub.srt");
    std::fs::write(&srt, SRT).unwrap();
    let ccap_srt = root.join("cc.srt");
    std::fs::write(&ccap_srt, SRT).unwrap();
    let keys_file = root.join("KEYS.json");
    let chain = root.join("chain");
    assert_eq!(generate_chain("Acme", &chain), 0, "chain gen");

    let config = DcpConfig {
        encrypt: true,
        key_out: Some(keys_file.clone()),
        subtitle_path: Some(srt),
        ccap_path: Some(ccap_srt),
        atmos_path: Some(make_atmos(&root.join("atmos"))),
        // an encrypted package must ship a signed CPL and PKL
        signer: Some(dcpwizard_core::package_signature::PackageSigner {
            signer_cert: chain.join("signer.pem"),
            signer_key: chain.join("signer.key"),
            signer_chain: vec![chain.join("intermediate.pem"), chain.join("root.pem")],
        }),
        ..base_config(root, &out)
    };
    assert_eq!(create_dcp(&config), 0, "every track must be encryptable");

    // 1. every essence file is flagged encrypted in its own MXF header
    for prefix in ["picture", "sound", "subtitle", "ccap", "atmos"] {
        let mxf = mxf_with_prefix(&out, prefix).unwrap_or_else(|| panic!("{prefix} MXF"));
        assert!(
            essence_is_encrypted(&mxf, prefix),
            "{prefix} essence must be encrypted"
        );
    }

    // 2. every CPL asset block declares its KeyId
    let cpl_path = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("CPL_"))
        })
        .expect("CPL");
    let cpl = std::fs::read_to_string(&cpl_path).unwrap();
    for element in [
        "MainPicture",
        "MainSound",
        "MainSubtitle",
        "tt:ClosedCaption",
        "axd:AuxData",
    ] {
        let block = cpl_asset_block(&cpl, element);
        assert!(
            block.contains("<KeyId>urn:uuid:"),
            "{element} must declare a KeyId:\n{block}"
        );
    }
    // ST 429-7 orders KeyId before Hash in a track file asset
    let subtitle_block = cpl_asset_block(&cpl, "MainSubtitle");
    assert!(
        subtitle_block.find("<KeyId>").unwrap() < subtitle_block.find("<Hash>").unwrap(),
        "KeyId must precede Hash:\n{subtitle_block}"
    );

    // 3. the keys file carries one key per essence with the right type string
    let cpl_id = dcpwizard_core::multi_cpl::list_cpls(&out)[0].id.clone();
    let content_keys = dcpwizard_core::kdm::load_content_keys(&keys_file, &cpl_id).unwrap();
    let mut types: Vec<String> = content_keys
        .iter()
        .map(|k| String::from_utf8(k.key_type.to_vec()).unwrap())
        .collect();
    types.sort();
    assert_eq!(types, ["MDAK", "MDEK", "MDIK", "MDSK", "MDSK"]);

    // 4. the KDM lists every one of them
    let (recipient_cert, recipient_key) = recipient(root, &chain);
    let kdm_path = root.join("screen.kdm.xml");
    // postkit follows libdcp and compares at day granularity, so a chain minted
    // today cannot sign a window that starts today
    let start = chrono::Utc::now() + chrono::Duration::days(1);
    let end = start + chrono::Duration::days(7);
    assert_eq!(
        dcpwizard_core::kdm::generate_kdm(
            cpl_id.clone(),
            config.title.clone(),
            recipient_cert,
            chain.join("signer.pem"),
            chain.join("signer.key"),
            vec![chain.join("intermediate.pem"), chain.join("root.pem")],
            start.format("%Y-%m-%dT%H:%M:%S+00:00").to_string(),
            end.format("%Y-%m-%dT%H:%M:%S+00:00").to_string(),
            content_keys.clone(),
            kdm_path.clone(),
            dcpwizard_core::kdm::KdmFormat::Smpte,
            None,
            None,
            Vec::new(),
            Default::default(),
        ),
        0,
        "KDM generation must succeed"
    );
    let kdm = std::fs::read_to_string(&kdm_path).unwrap();
    assert_eq!(kdm.matches("<KeyType>MDIK</KeyType>").count(), 1);
    assert_eq!(kdm.matches("<KeyType>MDAK</KeyType>").count(), 1);
    assert_eq!(kdm.matches("<KeyType>MDSK</KeyType>").count(), 2);
    assert_eq!(kdm.matches("<KeyType>MDEK</KeyType>").count(), 1);
    for key in &content_keys {
        assert!(
            kdm.contains(&format!("<KeyId>urn:uuid:{}</KeyId>", key.key_id)),
            "KDM must name every KeyId"
        );
    }

    // 5. the package verifies against that KDM, and nothing is left in the clear
    let report = dcpdoctor_core::verify(
        &out,
        &dcpdoctor_core::VerifyOptions {
            check_hashes: true,
            check_signatures: true,
            check_picture_details: true,
            scan_every_frame: false,
            strict_smpte: false,
            ov: None,
            kdm: Some(kdm_path),
            recipient_key: Some(recipient_key),
        },
    );
    let errors: Vec<String> = report
        .notes
        .iter()
        .filter(|n| n.severity == dcpdoctor_core::Severity::Error)
        .map(|n| format!("{}: {}", n.code.as_str(), n.message))
        .filter(|e| {
            !KNOWN_SCHEMA_VIOLATIONS
                .iter()
                .any(|known| e.contains(known))
        })
        .collect();
    assert!(errors.is_empty(), "verify with the KDM reported {errors:?}");
    assert!(
        !report
            .notes
            .iter()
            .any(|n| n.code == dcpdoctor_core::Code::PartiallyEncrypted),
        "an all-encrypted package must not look partially encrypted"
    );
}

#[test]
fn an_unencrypted_dcp_still_packages_a_subtitle() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dcp");
    let srt = dir.path().join("sub.srt");
    std::fs::write(&srt, SRT).unwrap();

    let config = DcpConfig {
        subtitle_path: Some(srt),
        ..base_config(dir.path(), &out)
    };
    assert_eq!(create_dcp(&config), 0, "an unencrypted DCP is unchanged");
    let subtitle = mxf_with_prefix(&out, "subtitle").expect("the subtitle track is still packaged");
    assert!(
        !essence_is_encrypted(&subtitle, "subtitle"),
        "an unencrypted package must not key its timed text"
    );
}
