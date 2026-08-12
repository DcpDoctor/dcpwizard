//! ds:Signature over a DCP's CPL and PKL (SMPTE ST 429-7 / 429-8), delegated to
//! postkit's xmldsig whole-document enveloped profile.
//!
//! The PKL carries the CPL's hash, so a CPL has to be signed before it is
//! hashed. Signing is opt-in: no signer leaves the package exactly as it was.

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

    /// Prove the certificate parses and the private key loads and matches it, by
    /// running the real signer over a fixed in-memory document. Call before any
    /// package file is written so a bad signer cannot leave a half-signed DCP.
    pub fn check_usable(&self) -> Result<(), String> {
        postkit::xmldsig::sign_document_enveloped(SIGNER_CHECK_DOCUMENT, &self.as_xml_signer())
            .map(|_| ())
    }

    /// Sign the XML document at `path` in place, inserting ds:Signature as the
    /// last child of its root.
    pub fn sign_file(&self, path: &Path) -> Result<(), String> {
        let xml = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {} to sign it: {e}", path.display()))?;
        let signed = postkit::xmldsig::sign_document_enveloped(&xml, &self.as_xml_signer())?;
        std::fs::write(path, signed)
            .map_err(|e| format!("cannot write the signed {}: {e}", path.display()))
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

    fn chain(dir: &Path) -> PackageSigner {
        assert_eq!(generate_chain("Acme", dir), 0, "chain generation failed");
        PackageSigner {
            signer_cert: dir.join("signer.pem"),
            signer_key: dir.join("signer.key"),
            signer_chain: vec![dir.join("intermediate.pem"), dir.join("root.pem")],
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
