//! Driving the xmlsec1 binary across the versions the CI runners carry.
//!
//! Linux and macOS are on different major versions: 1.3 searches keys strictly,
//! so a document whose KeyInfo does not name its key fails to verify unless
//! `--lax-key-search` is passed, and 1.3 stopped printing error detail unless
//! `--verbose` is. Neither flag exists in 1.2, so both are probed for. 1.3 also
//! does not chain through the sibling X509Data a DCP signature puts the
//! intermediate in, so the chain directory's intermediate.pem goes in as
//! `--untrusted-pem`. The msys2 build on Windows defaults to the mscrypto
//! backend, which cannot load a pem certificate, so it is told to use openssl.

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

/// Verify `doc` against the root.pem in `chain_dir`, as written by
/// `postkit::certificate::generate_chain`.
pub fn verify(doc: &Path, chain_dir: &Path, extra: &[&str]) -> Output {
    let mut command = Command::new("xmlsec1");
    command.arg("--verify");
    if cfg!(windows) {
        command.args(["--crypto", "openssl"]);
    }
    command
        .args(compatibility_flags())
        .args(extra)
        .arg("--trusted-pem")
        .arg(chain_dir.join("root.pem"))
        .arg("--untrusted-pem")
        .arg(chain_dir.join("intermediate.pem"))
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

pub fn assert_verifies(doc: &Path, chain_dir: &Path, extra: &[&str]) {
    let out = verify(doc, chain_dir, extra);
    assert!(
        out.status.success(),
        "xmlsec1 must verify {}",
        report(doc, &out)
    );
}
