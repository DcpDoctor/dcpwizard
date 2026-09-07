//! End-to-end decrypt: build a real encrypted DCP via the create path, generate
//! a KDM for a generated recipient cert, then decrypt it and prove the output
//! is cleartext, validates, and byte-for-byte recovers the source codestream.
//! Also proves a wrong recipient key and a KDM missing a KeyId both fail loud,
//! and that transcode-dcp can decrypt an encrypted source in memory.

use dcpwizard_core::decrypt::{DcpDecryptConfig, decrypt_dcp};
use postkit::certificate::{CertOptions, CertType, generate_certificate, generate_chain};
use std::path::{Path, PathBuf};

mod small_dcp;
use small_dcp::{base_config, find_mxf, make_frames, make_wav, read_picture_frame0};

/// Generate a recipient leaf cert + key under `dir` issued by the chain root.
fn recipient(dir: &Path, chain_dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let cert = dir.join(format!("{name}.pem"));
    let key = dir.join(format!("{name}.key"));
    let opts = CertOptions {
        cert_type: CertType::Leaf,
        common_name: name.into(),
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

fn is_encrypted(mxf: &Path, kind: &str) -> bool {
    let info = match kind {
        "picture" => {
            let mut r = asdcplib::jp2k::MxfReader::new();
            r.open_read(&mxf.to_string_lossy()).unwrap();
            r.writer_info().unwrap()
        }
        _ => {
            let mut r = asdcplib::pcm::MxfReader::new();
            r.open_read(&mxf.to_string_lossy()).unwrap();
            r.writer_info().unwrap()
        }
    };
    info.encrypted_essence
}

/// A KDM window a chain minted moments ago can sign.
fn tomorrow_for_seven_days() -> (String, String) {
    let start = chrono::Utc::now() + chrono::Duration::days(1);
    let end = start + chrono::Duration::days(7);
    (
        start.format("%Y-%m-%dT%H:%M:%S+00:00").to_string(),
        end.format("%Y-%m-%dT%H:%M:%S+00:00").to_string(),
    )
}

/// Generate a signer chain and a KDM covering `content_keys` for `recipient_cert`.
#[allow(clippy::too_many_arguments)]
fn make_kdm(
    chain_dir: &Path,
    cpl_id: &str,
    recipient_cert: &Path,
    content_keys: Vec<postkit::certificate::KdmContentKey>,
    out: &Path,
) {
    let code = dcpwizard_core::kdm::generate_kdm(
        cpl_id.into(),
        "Secret".into(),
        recipient_cert.to_path_buf(),
        chain_dir.join("signer.pem"),
        chain_dir.join("signer.key"),
        vec![
            chain_dir.join("intermediate.pem"),
            chain_dir.join("root.pem"),
        ],
        // postkit follows libdcp and compares at day granularity, so a chain
        // minted today cannot sign a window that starts today
        tomorrow_for_seven_days().0,
        tomorrow_for_seven_days().1,
        content_keys,
        out.to_path_buf(),
        dcpwizard_core::kdm::KdmFormat::Smpte,
        None,
        None,
        Vec::new(),
        Default::default(),
    );
    assert_eq!(code, 0, "KDM generation must succeed");
}

#[test]
fn decrypt_recovers_cleartext_validates_and_fails_loud() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // 1. build a small encrypted DCP via the create path
    let j2k = root.join("frames");
    let src_codestream = make_frames(&j2k);
    let wav = root.join("audio.wav");
    make_wav(&wav);
    let enc_dcp = root.join("enc");
    let keys_file = root.join("KEYS.json");
    let config = base_config(&enc_dcp, j2k, wav, Some(&keys_file));
    assert_eq!(
        dcpwizard_core::dcp::create_dcp(&config),
        0,
        "create encrypted DCP"
    );

    // sanity: the source picture + sound are actually encrypted
    let enc_pic = find_mxf(&enc_dcp, "picture").expect("encrypted picture");
    let enc_snd = find_mxf(&enc_dcp, "sound").expect("encrypted sound");
    assert!(
        is_encrypted(&enc_pic, "picture"),
        "source picture must be encrypted"
    );
    assert!(
        is_encrypted(&enc_snd, "sound"),
        "source sound must be encrypted"
    );

    let cpl_id = dcpwizard_core::multi_cpl::list_cpls(&enc_dcp)[0].id.clone();

    // 2. cert chain + recipient cert, then a KDM bound to the DCP's content keys
    let chain = root.join("chain");
    assert_eq!(generate_chain("Acme", &chain), 0, "chain gen");
    let (rcpt_cert, rcpt_key) = recipient(root, &chain, "screen");
    let content_keys = dcpwizard_core::kdm::load_content_keys(&keys_file, &cpl_id).unwrap();
    assert_eq!(content_keys.len(), 2, "picture + sound keys in the DCP");
    let kdm = root.join("screen.kdm.xml");
    make_kdm(&chain, &cpl_id, &rcpt_cert, content_keys.clone(), &kdm);

    // 3. decrypt with the KDM + recipient key
    let dec_dcp = root.join("dec");
    let dc = DcpDecryptConfig {
        input_dir: enc_dcp.clone(),
        output_dir: dec_dcp.clone(),
        kdm: Some(kdm.clone()),
        recipient_key: Some(rcpt_key.clone()),
        keys: None,
    };
    assert_eq!(decrypt_dcp(&dc), 0, "decrypt must succeed");

    // output validates
    let result = dcpwizard_core::verify::verify_dcp(&dec_dcp);
    assert!(
        result.valid,
        "decrypted DCP must validate: {:?}",
        result.errors
    );

    assert_main_sound_carried_over(&enc_dcp, &dec_dcp, &result.warnings);

    // picture + sound are cleartext now
    let dec_pic = find_mxf(&dec_dcp, "picture").expect("decrypted picture");
    let dec_snd = find_mxf(&dec_dcp, "sound").expect("decrypted sound");
    assert!(
        !is_encrypted(&dec_pic, "picture"),
        "decrypted picture must be cleartext"
    );
    assert!(
        !is_encrypted(&dec_snd, "sound"),
        "decrypted sound must be cleartext"
    );

    // frame 0 codestream byte-for-byte equals the pre-encryption source
    let frame0 = read_picture_frame0(&dec_pic);
    assert_eq!(
        frame0, src_codestream,
        "decrypted frame 0 must equal the source codestream"
    );

    // 4. the KEYS.json is an equivalent key source
    let dec_keys_dcp = root.join("dec_keys");
    let dck = DcpDecryptConfig {
        input_dir: enc_dcp.clone(),
        output_dir: dec_keys_dcp.clone(),
        kdm: None,
        recipient_key: None,
        keys: Some(keys_file.clone()),
    };
    assert_eq!(decrypt_dcp(&dck), 0, "decrypt via KEYS.json must succeed");
    assert!(dcpwizard_core::verify::verify_dcp(&dec_keys_dcp).valid);

    // 5. wrong recipient key fails loud (OAEP unwrap rejects it)
    let (_c2, wrong_key) = recipient(root, &chain, "other");
    let bad = DcpDecryptConfig {
        input_dir: enc_dcp.clone(),
        output_dir: root.join("dec_wrongkey"),
        kdm: Some(kdm.clone()),
        recipient_key: Some(wrong_key),
        keys: None,
    };
    assert_eq!(decrypt_dcp(&bad), -1, "wrong recipient key must fail loud");

    // 6. a KDM missing a KeyId (sound key dropped) fails loud on the sound track
    let picture_only: Vec<_> = content_keys
        .into_iter()
        .filter(|k| k.key_type == *b"MDIK")
        .collect();
    assert_eq!(picture_only.len(), 1);
    let partial_kdm = root.join("partial.kdm.xml");
    make_kdm(&chain, &cpl_id, &rcpt_cert, picture_only, &partial_kdm);
    let miss = DcpDecryptConfig {
        input_dir: enc_dcp.clone(),
        output_dir: root.join("dec_partial"),
        kdm: Some(partial_kdm),
        recipient_key: Some(rcpt_key),
        keys: None,
    };
    assert_eq!(decrypt_dcp(&miss), -1, "KDM missing a KeyId must fail loud");
}

#[test]
fn transcode_dcp_decrypts_encrypted_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let j2k = root.join("frames");
    make_frames(&j2k);
    let wav = root.join("audio.wav");
    make_wav(&wav);
    let enc_dcp = root.join("enc");
    let keys_file = root.join("KEYS.json");
    let config = base_config(&enc_dcp, j2k, wav, Some(&keys_file));
    assert_eq!(
        dcpwizard_core::dcp::create_dcp(&config),
        0,
        "create encrypted DCP"
    );

    // transcode the encrypted source using the KEYS.json as the key material
    let out = root.join("transcoded");
    let tc = dcpwizard_core::j2k_transcode::DcpTranscodeConfig {
        input_dir: enc_dcp.clone(),
        output_dir: out.clone(),
        target_bitrate_mbps: Some(50),
        target_width: 0,
        target_height: 0,
        kdm: None,
        recipient_key: None,
        keys: Some(keys_file),
        watermark: None,
    };
    assert_eq!(
        dcpwizard_core::j2k_transcode::transcode_dcp(&tc),
        0,
        "transcode of encrypted source must succeed"
    );

    // output validates and its picture is cleartext (decrypted, re-encoded)
    let result = dcpwizard_core::verify::verify_dcp(&out);
    assert!(result.valid, "transcoded DCP must validate");
    assert_main_sound_carried_over(&enc_dcp, &out, &result.warnings);
    let pic = find_mxf(&out, "picture").expect("transcoded picture");
    assert!(
        !is_encrypted(&pic, "picture"),
        "transcoded picture must be cleartext"
    );

    // without any key material, an encrypted source is refused loud
    let out2 = root.join("transcoded_nokey");
    let tc2 = dcpwizard_core::j2k_transcode::DcpTranscodeConfig {
        input_dir: enc_dcp,
        output_dir: out2,
        target_bitrate_mbps: Some(50),
        target_width: 0,
        target_height: 0,
        kdm: None,
        recipient_key: None,
        keys: None,
        watermark: None,
    };
    assert_eq!(
        dcpwizard_core::j2k_transcode::transcode_dcp(&tc2),
        -1,
        "encrypted source without keys must fail loud"
    );
}

/// The CPL a package was written with.
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

/// Assert the rebuilt CPL declares the sound layout the source declared, which is
/// what dcpdoctor's CompositionMetadataAsset check reads.
fn assert_main_sound_carried_over(source_dir: &Path, rebuilt_dir: &Path, warnings: &[String]) {
    let source = dcpwizard_core::cpl::main_sound_from_cpl(&cpl_xml(source_dir))
        .expect("the source CPL declares its sound layout");
    let rebuilt = dcpwizard_core::cpl::main_sound_from_cpl(&cpl_xml(rebuilt_dir))
        .expect("the rebuilt CPL must declare a sound layout");
    assert_eq!(rebuilt.configuration, source.configuration);
    assert_eq!(rebuilt.sample_rate, source.sample_rate);
    assert!(
        !warnings
            .iter()
            .any(|w| w.contains("missing_required_element")),
        "no element may be reported missing: {warnings:?}"
    );
}

/// Markers a package was authored with, as (label, offset) pairs in file order.
fn cpl_markers(dcp_dir: &Path) -> Vec<(String, u64)> {
    dcpwizard_core::markers::markers_from_cpl(&cpl_xml(dcp_dir))
        .into_iter()
        .flatten()
        .map(|entry| (entry.marker.label().to_string(), entry.frame))
        .collect()
}

/// A CPL rebuilt from an existing package must keep the markers it carried:
/// decrypt and transcode-dcp both re-author the CPL around the same composition.
#[test]
fn markers_survive_decrypt_and_transcode() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let j2k = root.join("frames");
    make_frames(&j2k);
    let wav = root.join("audio.wav");
    make_wav(&wav);
    let enc_dcp = root.join("enc");
    let keys_file = root.join("KEYS.json");
    let mut config = base_config(&enc_dcp, j2k, wav, Some(&keys_file));
    // a distributor-requested set, not the FFOC/LFOC default, so a rebuild that
    // silently re-derived the defaults would not pass
    config.markers = vec!["FFEC=00:00:00:04".into(), "LFEC=20".into()];
    assert_eq!(
        dcpwizard_core::dcp::create_dcp(&config),
        0,
        "create encrypted DCP with markers"
    );
    let source_markers = cpl_markers(&enc_dcp);
    assert_eq!(
        source_markers,
        vec![("FFEC".to_string(), 4), ("LFEC".to_string(), 20)],
        "the source package carries the authored markers"
    );

    let dec_dcp = root.join("dec");
    assert_eq!(
        decrypt_dcp(&DcpDecryptConfig {
            input_dir: enc_dcp.clone(),
            output_dir: dec_dcp.clone(),
            kdm: None,
            recipient_key: None,
            keys: Some(keys_file.clone()),
        }),
        0,
        "decrypt must succeed"
    );
    assert_eq!(
        cpl_markers(&dec_dcp),
        source_markers,
        "decrypt must carry the markers through"
    );

    let transcoded = root.join("transcoded");
    assert_eq!(
        dcpwizard_core::j2k_transcode::transcode_dcp(
            &dcpwizard_core::j2k_transcode::DcpTranscodeConfig {
                input_dir: enc_dcp,
                output_dir: transcoded.clone(),
                target_bitrate_mbps: Some(50),
                target_width: 0,
                target_height: 0,
                kdm: None,
                recipient_key: None,
                keys: Some(keys_file),
                watermark: None,
            }
        ),
        0,
        "transcode must succeed"
    );
    assert_eq!(
        cpl_markers(&transcoded),
        source_markers,
        "re-encoding keeps the frame count, so it keeps the offsets"
    );
}
