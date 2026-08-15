//! shared helpers for the kdm distribution data files: xdg data dir, atomic
//! full-file writes (temp + rename), and untrusted-cert parsing.

use postkit::certificate::CertInfo;
use std::path::{Path, PathBuf};

/// base data dir for dcpwizard state (~/.local/share/dcpwizard on linux).
pub fn data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("dcpwizard")
}

pub fn default_db_path() -> PathBuf {
    data_dir().join("cinemas.json")
}

pub fn default_history_path() -> PathBuf {
    data_dir().join("kdm-history.jsonl")
}

pub fn default_templates_path() -> PathBuf {
    data_dir().join("kdm-templates.json")
}

/// write bytes to `path` atomically: temp file in the same dir, then rename.
/// a reader either sees the old file or the fully written new one, never a
/// half-written db.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| format!("cannot create temp file in {}: {e}", dir.display()))?;
    use std::io::Write;
    tmp.write_all(bytes)
        .map_err(|e| format!("cannot write temp file: {e}"))?;
    tmp.flush()
        .map_err(|e| format!("cannot flush temp file: {e}"))?;
    tmp.persist(path)
        .map_err(|e| format!("cannot rename into {}: {e}", path.display()))?;
    Ok(())
}

/// parse an X.509 cert from a file, treating it as untrusted input. postkit's
/// read_certificate returns a default (empty thumbprint) on any parse failure,
/// so an empty thumbprint means "not a valid certificate".
pub fn cert_info_from_file(path: &Path) -> Result<CertInfo, String> {
    let info = postkit::certificate::read_certificate(path);
    if info.thumbprint.is_empty() {
        return Err(format!(
            "{} is not a parseable X.509 certificate",
            path.display()
        ));
    }
    Ok(info)
}

/// parse an X.509 cert from an in-memory PEM string. writes to a temp file so
/// the existing file-based parser is reused; errors on non-cert input.
pub fn cert_info_from_pem(pem: &str) -> Result<CertInfo, String> {
    let mut tmp =
        tempfile::NamedTempFile::new().map_err(|e| format!("cannot create temp file: {e}"))?;
    use std::io::Write;
    tmp.write_all(pem.as_bytes())
        .map_err(|e| format!("cannot write temp cert: {e}"))?;
    tmp.flush().ok();
    cert_info_from_file(tmp.path())
}

/// re-wrap base64 DER (as found in FLM ds:X509Certificate) into a PEM block.
pub fn der_base64_to_pem(b64: &str) -> String {
    let clean: String = b64.split_whitespace().collect();
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in clean.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_then_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("db.json");
        atomic_write(&p, b"hello").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        // overwrite leaves no temp files behind
        atomic_write(&p, b"world").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"world");
        let leftovers = std::fs::read_dir(p.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1, "only the db file remains, no temp files");
    }

    #[test]
    fn bad_cert_errors() {
        assert!(cert_info_from_pem("not a certificate").is_err());
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("junk.pem");
        std::fs::write(&p, b"garbage").unwrap();
        assert!(cert_info_from_file(&p).is_err());
    }
}
