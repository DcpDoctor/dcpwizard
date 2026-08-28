//! Driving the xmlsec1 binary across the versions the CI runners carry.
//!
//! Linux and macOS are on different major versions: 1.3 searches keys strictly,
//! so a document whose KeyInfo does not name its key fails to verify unless
//! `--lax-key-search` is passed, and 1.3 stopped printing error detail unless
//! `--verbose` is. Neither flag exists in 1.2, so both are probed for.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::OnceLock;

const VERSION_FLAGS: [&str; 2] = ["--lax-key-search", "--verbose"];

/// The flags above that this machine's xmlsec1 actually accepts.
pub fn compatibility_flags() -> &'static [&'static str] {
    static FLAGS: OnceLock<Vec<&'static str>> = OnceLock::new();
    FLAGS
        .get_or_init(|| {
            // --help is a summary, the option list is under --help-all
            let mut help = String::new();
            for arg in ["--help", "--help-all"] {
                if let Ok(out) = Command::new("xmlsec1").arg(arg).output() {
                    help.push_str(&String::from_utf8_lossy(&out.stdout));
                    help.push_str(&String::from_utf8_lossy(&out.stderr));
                }
            }
            VERSION_FLAGS
                .into_iter()
                .filter(|flag| help.contains(flag))
                .collect()
        })
        .as_slice()
}

pub fn verify(doc: &Path, trusted_pem: &Path, extra: &[&str]) -> Output {
    Command::new("xmlsec1")
        .arg("--verify")
        .args(compatibility_flags())
        .args(extra)
        .arg("--trusted-pem")
        .arg(trusted_pem)
        .arg(doc)
        .output()
        .expect("run xmlsec1")
}

pub fn report(doc: &Path, out: &Output) -> String {
    format!(
        "{}\n  flags: {:?}\n  status: {}\n  stdout: {}\n  stderr: {}",
        doc.display(),
        compatibility_flags(),
        out.status,
        String::from_utf8_lossy(&out.stdout).trim(),
        String::from_utf8_lossy(&out.stderr).trim(),
    )
}

pub fn assert_verifies(doc: &Path, trusted_pem: &Path, extra: &[&str]) {
    let out = verify(doc, trusted_pem, extra);
    assert!(
        out.status.success(),
        "xmlsec1 must verify {}",
        report(doc, &out)
    );
}
