//! ds:Signature over a DCP's CPL and PKL (SMPTE ST 429-7 / 429-8), delegated to
//! postkit's xmldsig whole-document enveloped profile.
//!
//! The PKL carries the CPL's hash, so a CPL has to be signed before it is
//! hashed. Signing is opt-in: no signer leaves the package exactly as it was.

use postkit::packaging::ns;
use postkit::xmldsig::SignatureProfile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Identity that signs a DCP's CPL and PKL: leaf certificate, its RSA private
/// key, and the CA certificates above the leaf (intermediate(s) then root).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageSigner {
    pub signer_cert: PathBuf,
    pub signer_key: PathBuf,
    pub signer_chain: Vec<PathBuf>,
}

/// Fixed document `check_usable` signs. Its signature is discarded, so nothing
/// an attacker chooses is ever signed by the check.
const SIGNER_CHECK_DOCUMENT: &str = "<SignerCheck></SignerCheck>";

impl PackageSigner {
    fn as_xml_signer(&self) -> postkit::xmldsig::XmlSigner {
        postkit::xmldsig::XmlSigner {
            cert_file: self.signer_cert.clone(),
            key_file: self.signer_key.clone(),
            chain_files: self.signer_chain.clone(),
        }
    }

    /// Prove the certificate parses, the private key loads and matches it, and
    /// the chain meets ST 430-2, by running the real signer over a fixed
    /// in-memory document. Call before any package file is written so a bad
    /// signer cannot leave a half-signed DCP.
    pub fn check_usable(&self) -> Result<(), String> {
        let signed = postkit::xmldsig::sign_document_enveloped(
            SIGNER_CHECK_DOCUMENT,
            &self.as_xml_signer(),
        )?;
        self.check_chain(&signed)
    }

    /// Sign the XML document at `path` in place, adding the `Signer` element and
    /// ds:Signature as the last two children of its root.
    pub fn sign_file(&self, path: &Path) -> Result<(), String> {
        let xml = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {} to sign it: {e}", path.display()))?;
        let (insert_offset, profile) =
            read_root(&xml).map_err(|e| format!("cannot sign {}: {e}", path.display()))?;

        let mut with_signer = String::with_capacity(xml.len() + SIGNER_ELEMENT_HEADROOM);
        with_signer.push_str(&xml[..insert_offset]);
        with_signer.push_str("  ");
        with_signer.push_str(&postkit::xmldsig::dcp_signer_element(&self.signer_cert)?);
        with_signer.push('\n');
        with_signer.push_str(&xml[insert_offset..]);

        let signed = postkit::xmldsig::sign_document_enveloped_as(
            &with_signer,
            &self.as_xml_signer(),
            profile,
        )?;
        self.check_chain(&signed)?;
        std::fs::write(path, signed)
            .map_err(|e| format!("cannot write the signed {}: {e}", path.display()))
    }

    /// Hold the supplied chain to the ST 430-2 certificate profile, over the
    /// chain exactly as ds:KeyInfo embeds it, reusing dcpdoctor's rules rather
    /// than a second copy of them: signature algorithm, RSA 2048 with e=65537,
    /// BasicConstraints and KeyUsage for the role each certificate plays, a
    /// signer role token distinct from the CAs', a dnQualifier that matches the
    /// public-key thumbprint when one is present, and one Organization across
    /// the chain. It judges certificates, not chain completeness or validity
    /// dates, which dcpdoctor's verify covers on the finished package.
    ///
    /// There is no flag to skip this. A chain that breaks one of these rules is
    /// one a DCI-compliant verifier rejects, so signing with it would only move
    /// the failure to a screening room.
    fn check_chain(&self, signed_xml: &str) -> Result<(), String> {
        let violations: Vec<String> =
            dcpdoctor_core::cert_rules::check_certificates(signed_xml, &self.signer_cert)
                .into_iter()
                .filter(|note| matches!(note.severity, dcpdoctor_core::Severity::Error))
                .map(|note| format!("{} [{}]", note.message, note.code))
                .collect();
        if violations.is_empty() {
            return Ok(());
        }
        Err(format!(
            "the signer chain does not meet SMPTE ST 430-2: {}",
            violations.join(", ")
        ))
    }
}

/// Room for the `Signer` element so inserting it does not reallocate.
const SIGNER_ELEMENT_HEADROOM: usize = 512;

/// Where a last child of the root element goes, and the signature profile the
/// document's own namespace calls for.
///
/// The algorithm follows the document rather than a command-line standard flag,
/// so a package can never be written Interop and signed as if it were SMPTE.
fn read_root(xml: &str) -> Result<(usize, SignatureProfile), String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("not valid XML: {e}"))?;
    let root = doc.root_element();
    let profile = match root.tag_name().namespace().unwrap_or("") {
        ns::CPL_INTEROP | ns::PKL_INTEROP => SignatureProfile::RsaSha1,
        _ => SignatureProfile::RsaSha256,
    };

    // The root's end tag is the last thing in its source range, so the last '<'
    // in that range opens it. A root with no children has none to find.
    let range = root.range();
    let end_tag = xml[..range.end]
        .rfind('<')
        .ok_or("the root element has no end tag")?;
    if end_tag == range.start {
        return Err("the root element is self-closing, so nothing can be added to it".into());
    }
    Ok((end_tag, profile))
}

/// Sign `path` when there is a signer, and do nothing when there is not. Returns
/// whether the caller may carry on, so an unusable signer stops a half-signed
/// package rather than leaving one behind.
///
/// A CPL has to go through this before anything hashes it into a PKL.
pub fn sign_if_configured(signer: Option<&PackageSigner>, path: &Path, what: &str) -> bool {
    let Some(signer) = signer else {
        return true;
    };
    match signer.sign_file(path) {
        Ok(()) => true,
        Err(e) => {
            tracing::error!("failed to sign the {what}: {e}");
            false
        }
    }
}

/// Drop a document's Signature, and the Signer that only means anything beside
/// it, whatever namespace prefix they carry. Returns whether it was signed.
///
/// Anything that rewrites a signed document has to call this: a signature left
/// over an edited document no longer matches the bytes, and a verifier reports
/// that as tampering rather than as an unsigned package.
pub fn strip_signature(xml: &mut String) -> bool {
    if !remove_element(xml, "Signature") {
        return false;
    }
    remove_element(xml, "Signer");
    true
}

/// Find an element's open tag by local name, whatever prefix it carries.
/// Returns the offset of its `<` and the prefix, so the close tag can be built.
fn find_open_tag(xml: &str, local: &str) -> Option<(usize, String)> {
    let mut from = 0;
    while let Some(rel) = xml[from..].find('<') {
        let open = from + rel;
        let rest = &xml[open + 1..];
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len());
        // a close tag starts with '/', so its name reads empty and never matches
        let (prefix, name) = match rest[..name_end].split_once(':') {
            Some((p, n)) => (format!("{p}:"), n),
            None => (String::new(), &rest[..name_end]),
        };
        if name == local {
            return Some((open, prefix));
        }
        from = open + 1;
    }
    None
}

/// Cut the first element with this local name, content and all, taking the
/// blank line it would otherwise leave behind.
fn remove_element(xml: &mut String, local: &str) -> bool {
    let Some((start, prefix)) = find_open_tag(xml, local) else {
        return false;
    };
    let close = format!("</{prefix}{local}>");
    let Some(rel) = xml[start..].find(&close) else {
        return false;
    };
    let end = start + rel + close.len();
    let line_start = xml[..start].rfind('\n').map_or(0, |n| n + 1);
    let cut_from = if xml[line_start..start].trim().is_empty() {
        line_start
    } else {
        start
    };
    let cut_to = if xml[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    xml.replace_range(cut_from..cut_to, "");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use postkit::certificate::generate_chain;
    use postkit::packaging::{DcpCpl, DcpCplReel, PackingList, PklAsset};

    fn chain(dir: &Path) -> PackageSigner {
        chain_for("Acme", dir)
    }

    fn chain_for(organization: &str, dir: &Path) -> PackageSigner {
        assert_eq!(
            generate_chain(organization, dir),
            0,
            "chain generation failed"
        );
        PackageSigner {
            signer_cert: dir.join("signer.pem"),
            signer_key: dir.join("signer.key"),
            signer_chain: vec![dir.join("intermediate.pem"), dir.join("root.pem")],
        }
    }

    /// A one-reel CPL in `namespace`, the smallest document both the SMPTE and
    /// the Interop schema accept.
    fn cpl_xml(namespace: &str) -> String {
        DcpCpl {
            uuid: "3413599d-fed7-4d89-87a2-7c5e0929da5f".into(),
            namespace: namespace.into(),
            title: "Signing Test".into(),
            content_kind: "test".into(),
            issuer: "PostPerfection".into(),
            creator: "dcpwizard".into(),
            issue_date: "2026-08-12T11:14:22+00:00".into(),
            reels: vec![DcpCplReel {
                reel_id: "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9".into(),
                picture_id: "11111111-2222-3333-4444-555555555555".into(),
                picture_edit_rate_num: 24,
                picture_edit_rate_den: 1,
                picture_duration: 480,
                picture_entry_point: 0,
                picture_width: 1998,
                picture_height: 1080,
                sound_id: Some("66666666-7777-8888-9999-aaaaaaaaaaaa".into()),
                sound_edit_rate_num: 48000,
                sound_edit_rate_den: 1,
                sound_duration: 960000,
                sound_entry_point: 0,
                ..Default::default()
            }],
        }
        .to_xml()
    }

    fn pkl_xml(namespace: &str) -> String {
        PackingList {
            uuid: "9376cda5-cc7e-49ee-b903-6cbbfa2d2ca0".into(),
            namespace: namespace.into(),
            issuer: "PostPerfection".into(),
            creator: "dcpwizard".into(),
            issue_date: "2026-08-12T11:14:22+00:00".into(),
            annotation: None,
            assets: vec![PklAsset {
                id: "11111111-2222-3333-4444-555555555555".into(),
                hash: "TlXtJYjfMZR8quOzhOc/0QxweHI=".into(),
                size: 1234,
                asset_type: "application/x-smpte-mxf;asdcpKind=Picture".into(),
            }],
        }
        .to_xml()
    }

    fn sign_to_string(signer: &PackageSigner, dir: &Path, name: &str, xml: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, xml).unwrap();
        signer.sign_file(&path).expect("sign the document");
        std::fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn a_signed_document_carries_a_signer_naming_the_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let signer = chain(dir.path());
        let signed = sign_to_string(&signer, dir.path(), "cpl.xml", &cpl_xml(ns::CPL_SMPTE));

        let signer_at = signed.find("<Signer ").expect("a Signer must be written");
        let signature_at = signed.find("<ds:Signature").expect("a Signature too");
        assert!(
            signer_at < signature_at,
            "both schemas put Signer before ds:Signature"
        );
        assert!(signed[signer_at..].contains("<ds:X509IssuerSerial>"));

        postkit::xmldsig::verify_document_enveloped(&signed, Some(&signer.signer_cert))
            .expect("the signed document must verify");

        // The enveloped reference covers the whole document, so the Signer is
        // signed over rather than sitting beside the signature unprotected.
        let end = signed.find("</Signer>").unwrap();
        let tampered = format!("{}9{}", &signed[..end - 1], &signed[end - 1..]);
        assert!(
            postkit::xmldsig::verify_document_enveloped(&tampered, None).is_err(),
            "editing the Signer must break the signature"
        );
    }

    #[test]
    fn interop_is_signed_rsa_sha1_and_smpte_rsa_sha256() {
        const RSA_SHA1: &str = "http://www.w3.org/2000/09/xmldsig#rsa-sha1";
        const RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";

        let dir = tempfile::tempdir().unwrap();
        let signer = chain(dir.path());
        for (name, xml, expected) in [
            ("cpl-interop.xml", cpl_xml(ns::CPL_INTEROP), RSA_SHA1),
            ("pkl-interop.xml", pkl_xml(ns::PKL_INTEROP), RSA_SHA1),
            ("cpl-smpte.xml", cpl_xml(ns::CPL_SMPTE), RSA_SHA256),
            ("pkl-smpte.xml", pkl_xml(ns::PKL_SMPTE), RSA_SHA256),
        ] {
            let signed = sign_to_string(&signer, dir.path(), name, &xml);
            assert!(
                signed.contains(&format!(r#"<ds:SignatureMethod Algorithm="{expected}"/>"#)),
                "{name} must be signed {expected}"
            );
            postkit::xmldsig::verify_document_enveloped(&signed, Some(&signer.signer_cert))
                .unwrap_or_else(|e| panic!("{name} must verify: {e}"));
        }
    }

    #[test]
    fn a_chain_spanning_two_organizations_fails_the_sign() {
        let dir = tempfile::tempdir().unwrap();
        let mut signer = chain_for("Acme", &dir.path().join("acme"));
        let other = chain_for("Rival", &dir.path().join("rival"));
        signer.signer_chain = vec![other.signer_chain[1].clone()];

        let err = signer
            .check_usable()
            .expect_err("a chain from two organizations is not ST 430-2");
        assert!(err.contains("Organization"), "got: {err}");

        let doc = dir.path().join("cpl.xml");
        std::fs::write(&doc, cpl_xml(ns::CPL_SMPTE)).unwrap();
        assert!(
            signer.sign_file(&doc).is_err(),
            "the same chain must fail the sign itself, not only the pre-check"
        );
        assert!(
            !std::fs::read_to_string(&doc).unwrap().contains("Signature"),
            "a rejected chain must leave the document unsigned"
        );
    }

    #[test]
    fn a_signer_sharing_its_ca_role_token_fails_the_sign() {
        use postkit::certificate::{CertOptions, CertType, generate_certificate};

        // ST 430-2 puts a role token before the first '.' of the CommonName and
        // the signer's must differ from every CA's above it.
        let dir = tempfile::tempdir().unwrap();
        let root_cert = dir.path().join("root.pem");
        let root_key = dir.path().join("root.key");
        assert_eq!(
            generate_certificate(&CertOptions {
                cert_type: CertType::Root,
                common_name: "Shared.Role".into(),
                organization: "Acme".into(),
                validity_days: 365,
                output_cert: root_cert.clone(),
                output_key: root_key.clone(),
                ..Default::default()
            }),
            0
        );
        let signer = PackageSigner {
            signer_cert: dir.path().join("signer.pem"),
            signer_key: dir.path().join("signer.key"),
            signer_chain: vec![root_cert.clone()],
        };
        assert_eq!(
            generate_certificate(&CertOptions {
                cert_type: CertType::Signer,
                common_name: "Shared.Signer".into(),
                organization: "Acme".into(),
                validity_days: 365,
                output_cert: signer.signer_cert.clone(),
                output_key: signer.signer_key.clone(),
                issuer_cert: root_cert,
                issuer_key: root_key,
                ..Default::default()
            }),
            0
        );

        let err = signer
            .check_usable()
            .expect_err("a signer sharing its CA's role token is not ST 430-2");
        assert!(err.contains("not distinct"), "got: {err}");
    }

    #[test]
    fn a_leaf_with_no_chain_still_signs() {
        // The rules judge each certificate against the role it plays rather than
        // demanding a complete chain, so omitting the CAs is not itself a fault.
        let dir = tempfile::tempdir().unwrap();
        let full = chain(dir.path());
        PackageSigner {
            signer_cert: full.signer_cert,
            signer_key: full.signer_key,
            signer_chain: Vec::new(),
        }
        .check_usable()
        .expect("a leaf on its own still reads as a leaf");
    }

    /// The SMPTE and Interop schemas dcpdoctor vendors, with the catalog that
    /// resolves their xmldsig and xml imports offline.
    fn schema_dir() -> Option<PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extern/dcpdoctor/schemas")
            .canonicalize()
            .ok()?;
        dir.join("catalog.xml").exists().then_some(dir)
    }

    fn xmllint_available() -> bool {
        std::process::Command::new("xmllint")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn signed_documents_still_validate_against_their_schemas() {
        if !xmllint_available() {
            eprintln!("skipping: xmllint not installed");
            return;
        }
        let Some(schemas) = schema_dir() else {
            eprintln!("skipping: the dcpdoctor schemas checkout is absent");
            return;
        };

        let dir = tempfile::tempdir().unwrap();
        let signer = chain(dir.path());
        for (name, xml, xsd) in [
            (
                "cpl-smpte.xml",
                cpl_xml(ns::CPL_SMPTE),
                "SMPTE-429-7-2006-CPL.xsd",
            ),
            (
                "pkl-smpte.xml",
                pkl_xml(ns::PKL_SMPTE),
                "SMPTE-429-8-2006-PKL.xsd",
            ),
            (
                "cpl-interop.xml",
                cpl_xml(ns::CPL_INTEROP),
                "PROTO-ASDCP-CPL-20040511.xsd",
            ),
            (
                "pkl-interop.xml",
                pkl_xml(ns::PKL_INTEROP),
                "PROTO-ASDCP-PKL-20040311.xsd",
            ),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, &xml).unwrap();
            signer.sign_file(&path).expect("sign the document");

            let out = std::process::Command::new("xmllint")
                .arg("--nonet")
                .arg("--catalogs")
                .env("XML_CATALOG_FILES", schemas.join("catalog.xml"))
                .arg("--schema")
                .arg(schemas.join(xsd))
                .arg("--noout")
                .arg(&path)
                .output()
                .expect("run xmllint");
            assert!(
                out.status.success(),
                "signed {name} must validate against {xsd}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    #[test]
    fn a_matching_key_and_cert_is_usable() {
        let dir = tempfile::tempdir().unwrap();
        chain(dir.path()).check_usable().expect("chain must sign");
    }

    #[test]
    fn a_key_from_another_certificate_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut signer = chain(dir.path());
        signer.signer_key = dir.path().join("root.key");
        let err = signer
            .check_usable()
            .expect_err("the root key does not match the signer cert");
        assert!(err.contains("does not match"), "got: {err}");
    }

    #[test]
    fn an_unreadable_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut signer = chain(dir.path());
        signer.signer_key = dir.path().join("absent.key");
        let err = signer.check_usable().expect_err("missing key must fail");
        assert!(err.contains("cannot read signer private key"), "got: {err}");
    }

    #[test]
    fn an_encrypted_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut signer = chain(dir.path());
        let encrypted = dir.path().join("encrypted.key");
        std::fs::write(
            &encrypted,
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIBnjBIBgkqhkiG9w0BBQ0wOzAe\n-----END ENCRYPTED PRIVATE KEY-----\n",
        )
        .unwrap();
        signer.signer_key = encrypted;
        let err = signer.check_usable().expect_err("encrypted key must fail");
        assert!(err.contains("not a valid RSA private key"), "got: {err}");
    }

    #[test]
    fn stripping_a_real_signature_gives_back_the_document_that_was_signed() {
        let dir = tempfile::tempdir().unwrap();
        let signer = chain(dir.path());
        let original = "<Root>\n  <Value>x</Value>\n</Root>\n";
        let doc = dir.path().join("doc.xml");
        std::fs::write(&doc, original).unwrap();
        signer.sign_file(&doc).expect("sign the document");

        let mut signed = std::fs::read_to_string(&doc).unwrap();
        assert!(signed.contains("Signature"), "the document must be signed");
        assert!(strip_signature(&mut signed), "a signed document reports so");
        assert_eq!(signed, original, "stripping restores the signed-over bytes");
    }

    #[test]
    fn stripping_finds_the_signature_under_any_prefix() {
        for prefix in ["ds:", "dsig:", ""] {
            let mut xml = format!(
                "<Root>\n  <Value>x</Value>\n  <{prefix}Signature Id=\"s\">\n    <{prefix}SignatureValue>AAA</{prefix}SignatureValue>\n  </{prefix}Signature>\n</Root>\n"
            );
            assert!(strip_signature(&mut xml), "{prefix:?} must be recognised");
            assert_eq!(xml, "<Root>\n  <Value>x</Value>\n</Root>\n");
        }
    }

    #[test]
    fn stripping_takes_the_signer_along_with_the_signature() {
        let mut xml = "<Root>\n  <Signer>\n    <X509Data>c</X509Data>\n  </Signer>\n  <ds:Signature>v</ds:Signature>\n</Root>\n".to_string();
        assert!(strip_signature(&mut xml));
        assert_eq!(xml, "<Root>\n</Root>\n");
    }

    #[test]
    fn an_unsigned_document_is_reported_unsigned_and_left_alone() {
        let original = "<Root>\n  <SignerCheck>keep me</SignerCheck>\n</Root>\n";
        let mut xml = original.to_string();
        assert!(!strip_signature(&mut xml), "nothing to strip");
        assert_eq!(xml, original, "an unsigned document is untouched");
    }

    #[test]
    fn signing_a_file_rewrites_it_with_a_verifiable_signature() {
        let dir = tempfile::tempdir().unwrap();
        let signer = chain(dir.path());
        let doc = dir.path().join("doc.xml");
        std::fs::write(&doc, "<Root>\n  <Value>x</Value>\n</Root>\n").unwrap();
        signer.sign_file(&doc).expect("sign the document");

        let signed = std::fs::read_to_string(&doc).unwrap();
        assert!(signed.contains("<ds:Signature"), "signature was written");
        postkit::xmldsig::verify_document_enveloped(&signed, Some(&signer.signer_cert))
            .expect("the written signature must verify against the signer cert");
    }
}
