//! Thin delegation over postkit's SMPTE 430-1 KDM implementation.
//!
//! postkit generates a fresh content key, RSA-OAEP encrypts it to the recipient
//! and signs the message per SMPTE 430-3. This layer only maps the CLI's inputs
//! onto postkit's config types and turns its `Result` into a dcpwizard exit code.

use std::path::{Path, PathBuf};

pub use postkit::certificate::KdmFormat;
use postkit::certificate::{AudioForensicMarking, KdmFormulation, PictureForensicMarking};

/// The KDM choices beyond the certificates and the validity window: which
/// devices the KDM names, and how each essence is forensically marked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KdmOptions {
    /// None derives the formulation from the device certificates, so a caller
    /// that names none keeps the assume-trust KDM it always got.
    pub formulation: Option<KdmFormulation>,
    pub picture_forensic_marking: PictureForensicMarking,
    pub audio_forensic_marking: AudioForensicMarking,
}

/// The formulation a KDM carries: the caller's explicit choice, or the one
/// derived from whether any device certificate was supplied.
///
/// A choice that disagrees with the device list is refused here, naming the
/// flags a dcpwizard user would change. postkit refuses the same combination,
/// but it cannot know what those flags are called on this command line.
pub fn resolve_formulation(
    explicit: Option<KdmFormulation>,
    device_cert_count: usize,
) -> Result<KdmFormulation, String> {
    let has_devices = device_cert_count > 0;
    let Some(formulation) = explicit else {
        let assume_trust = KdmFormulation::default();
        return Ok(if has_devices {
            assume_trust.device_list_counterpart()
        } else {
            assume_trust
        });
    };
    if formulation.lists_supplied_devices() == has_devices {
        return Ok(formulation);
    }
    let counterpart = formulation.device_list_counterpart();
    Err(if has_devices {
        format!(
            "--formulation {formulation} carries the assume-trust thumbprint instead of a device \
             list, so the {device_cert_count} --device-cert certificate(s) would be dropped: use \
             --formulation {counterpart}, or drop --device-cert"
        )
    } else {
        format!(
            "--formulation {formulation} lists the devices given by --device-cert, but none were \
             given: pass --device-cert, or use --formulation {counterpart}"
        )
    })
}

/// Parse the `--format` flag ("smpte" default, or "interop").
pub fn parse_format(s: &str) -> Result<KdmFormat, String> {
    match s.to_ascii_lowercase().as_str() {
        "smpte" | "" => Ok(KdmFormat::Smpte),
        "interop" => Ok(KdmFormat::Interop),
        other => Err(format!(
            "unknown KDM format '{other}' (use smpte or interop)"
        )),
    }
}

/// Load the content keys from a DCP keys file (written by `create --encrypt`)
/// for KDM generation, checking they belong to `cpl_id`.
pub fn load_content_keys(
    keys_file: &Path,
    cpl_id: &str,
) -> Result<Vec<postkit::certificate::KdmContentKey>, String> {
    let bundle = crate::encrypt::KeyBundle::read(keys_file)?;
    let want = cpl_id.trim().trim_start_matches("urn:uuid:");
    let have = bundle.cpl_id.trim().trim_start_matches("urn:uuid:");
    if !have.is_empty() && !want.is_empty() && have != want {
        return Err(format!(
            "keys file {} is for CPL {have}, not {want}",
            keys_file.display()
        ));
    }
    bundle
        .keys
        .iter()
        .map(|k| {
            let (key_type, key_id, content_key) = k.to_raw()?;
            Ok(postkit::certificate::KdmContentKey {
                key_type,
                key_id,
                content_key,
            })
        })
        .collect()
}

/// Parse a human-friendly duration ("2 weeks", "30 days", "7d", "24h", "2w").
/// Duplicates postkit's private parser so we can resolve the window ourselves.
pub fn parse_duration(s: &str) -> Result<chrono::Duration, String> {
    let s = s.trim().to_lowercase();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() == 2 {
        let n: i64 = parts[0]
            .parse()
            .map_err(|_| format!("invalid number in duration: '{}'", parts[0]))?;
        let unit = parts[1].trim_end_matches('s');
        return match unit {
            "second" | "sec" => Ok(chrono::Duration::seconds(n)),
            "minute" | "min" => Ok(chrono::Duration::minutes(n)),
            "hour" | "hr" => Ok(chrono::Duration::hours(n)),
            "day" => Ok(chrono::Duration::days(n)),
            "week" | "wk" => Ok(chrono::Duration::weeks(n)),
            _ => Err(format!("unknown duration unit: '{unit}'")),
        };
    }
    for (suffix, mult) in [('h', 1i64), ('d', 24), ('w', 24 * 7)] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            let n: i64 = stripped
                .parse()
                .map_err(|_| format!("invalid duration: '{s}'"))?;
            return Ok(chrono::Duration::hours(n * mult));
        }
    }
    Err(format!("cannot parse duration: '{s}'"))
}

/// Resolve the KDM validity window so a duration-based end time keeps the
/// start's UTC offset. postkit's duration path formats the end with a literal
/// +00:00 even when the start is in another offset (mislabeling the instant);
/// handing it absolute ISO timestamps avoids that branch.
fn resolve_validity(valid_from: &str, valid_to: &str) -> Result<(String, String), String> {
    let from = if valid_from.is_empty() || valid_from == "now" {
        chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S+00:00")
            .to_string()
    } else {
        valid_from.to_string()
    };
    // an absolute ISO end passes through; a duration is added in the start's offset
    let looks_absolute = valid_to.contains('T')
        || (valid_to.len() >= 10 && valid_to.as_bytes().get(4) == Some(&b'-'));
    let to = if valid_to.is_empty() || looks_absolute {
        valid_to.to_string()
    } else {
        let start = chrono::DateTime::parse_from_rfc3339(&from)
            .map_err(|e| format!("cannot parse valid-from '{from}': {e}"))?;
        let dur = parse_duration(valid_to)?;
        (start + dur).format("%Y-%m-%dT%H:%M:%S%:z").to_string()
    };
    Ok((from, to))
}

/// SMPTE/Interop format as a stable lowercase string for the history log.
fn format_str(f: KdmFormat) -> &'static str {
    match f {
        KdmFormat::Smpte => "smpte",
        KdmFormat::Interop => "interop",
    }
}

/// Append a history record for a KDM just written (dom#1014). Reads the
/// recipient cert only for its subject/serial; never touches key material. A
/// logging failure warns but does not fail the (already written) KDM.
#[allow(clippy::too_many_arguments)]
fn log_history(
    history: &Path,
    cpl_id: &str,
    content_title: &str,
    recipient_cert: &Path,
    valid_from: &str,
    valid_to: &str,
    output: &Path,
    format: KdmFormat,
) {
    let info = postkit::certificate::read_certificate(recipient_cert);
    let rec = crate::kdm_log::Record::now(
        cpl_id,
        content_title,
        &info.subject_cn,
        &info.serial,
        valid_from,
        valid_to,
        &output.display().to_string(),
        format_str(format),
    );
    if let Err(e) = crate::kdm_log::append(history, &rec) {
        tracing::warn!("could not append KDM history: {e}");
    }
}

/// Generate a signed KDM. `content_keys` (from the DCP's keys file) binds the
/// KDM to the encrypted essence; an empty vec makes postkit mint a fresh key.
/// `valid_from`/`valid_to` accept "now", ISO 8601 or a relative duration
/// ("2 weeks"); the window is resolved here so a duration keeps the start offset.
/// `history`, when set, appends one metadata record per successful KDM.
/// `device_certs` names the playback devices the KDM is restricted to; empty
/// emits the DCI assume-trust thumbprint, which restricts nothing.
#[allow(clippy::too_many_arguments)]
pub fn generate_kdm(
    cpl_id: String,
    content_title: String,
    recipient_cert: PathBuf,
    signer_cert: PathBuf,
    signer_key: PathBuf,
    signer_chain: Vec<PathBuf>,
    valid_from: String,
    valid_to: String,
    content_keys: Vec<postkit::certificate::KdmContentKey>,
    output: PathBuf,
    format: KdmFormat,
    annotation: Option<String>,
    history: Option<PathBuf>,
    device_certs: Vec<PathBuf>,
    options: KdmOptions,
) -> i32 {
    let (valid_from, valid_to) = match resolve_validity(&valid_from, &valid_to) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let formulation = match resolve_formulation(options.formulation, device_certs.len()) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let config = postkit::certificate::KdmConfig {
        cpl_id,
        content_title,
        annotation,
        recipient_cert_file: recipient_cert,
        signer_cert_file: signer_cert,
        signer_key_file: signer_key,
        signer_chain_files: signer_chain,
        output_file: output,
        valid_from,
        valid_to,
        formulation,
        content_keys,
        format,
        // the formulation above decides whether these are listed or the
        // assume-trust thumbprint is written instead
        device_cert_files: device_certs,
        picture_forensic_marking: options.picture_forensic_marking,
        audio_forensic_marking: options.audio_forensic_marking,
    };
    match postkit::certificate::generate_kdm(&config) {
        Ok(()) => {
            if let Some(h) = &history {
                log_history(
                    h,
                    &config.cpl_id,
                    &config.content_title,
                    &config.recipient_cert_file,
                    &config.valid_from,
                    &config.valid_to,
                    &config.output_file,
                    format,
                );
            }
            0
        }
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

/// Generate one KDM per recipient certificate in a single pass. Each KDM is
/// written to `output_dir/<cert-stem>.kdm.xml`. Returns 0 only if every
/// recipient succeeded, otherwise 1.
#[allow(clippy::too_many_arguments)]
pub fn generate_kdm_batch(
    cpl_id: String,
    content_title: String,
    recipient_certs: Vec<PathBuf>,
    signer_cert: PathBuf,
    signer_key: PathBuf,
    signer_chain: Vec<PathBuf>,
    valid_from: String,
    valid_to: String,
    content_keys: Vec<postkit::certificate::KdmContentKey>,
    output_dir: PathBuf,
    format: KdmFormat,
    annotation: Option<String>,
    history: Option<PathBuf>,
    device_certs: Vec<PathBuf>,
    options: KdmOptions,
) -> i32 {
    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        tracing::error!("Failed to create output directory: {e}");
        return 1;
    }

    let mut failures = 0;
    for (i, cert) in recipient_certs.iter().enumerate() {
        let stem = cert
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("recipient");
        // index-prefix keeps outputs unique even when recipient certs share a filename
        let output = output_dir.join(format!("{:03}_{stem}.kdm.xml", i + 1));
        let code = generate_kdm(
            cpl_id.clone(),
            content_title.clone(),
            cert.clone(),
            signer_cert.clone(),
            signer_key.clone(),
            signer_chain.clone(),
            valid_from.clone(),
            valid_to.clone(),
            content_keys.clone(),
            output.clone(),
            format,
            annotation.clone(),
            history.clone(),
            device_certs.clone(),
            options.clone(),
        );
        if code == 0 {
            tracing::info!("KDM for {} -> {}", cert.display(), output.display());
        } else {
            tracing::error!("KDM generation failed for {}", cert.display());
            failures += 1;
        }
    }

    if failures == 0 {
        tracing::info!("Generated {} KDM(s)", recipient_certs.len());
        0
    } else {
        tracing::error!("{failures} of {} KDM(s) failed", recipient_certs.len());
        1
    }
}

/// Collect recipient certificate paths from a directory: every *.pem/*.crt/*.cer
/// in it, sorted for a deterministic KDM order. Errors if the directory cannot be
/// read or holds no certificate, so a mistyped path fails loud instead of
/// silently producing zero KDMs.
pub fn certs_in_dir(dir: &Path) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read cert directory {}: {e}", dir.display()))?;
    let mut certs: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| matches!(x.to_ascii_lowercase().as_str(), "pem" | "crt" | "cer"))
                    .unwrap_or(false)
        })
        .filter_map(|p| p.to_str().map(String::from))
        .collect();
    certs.sort();
    if certs.is_empty() {
        return Err(format!(
            "no certificates (*.pem/*.crt/*.cer) found in {}",
            dir.display()
        ));
    }
    Ok(certs)
}

/// Re-wrap a DKDM to a new recipient: decrypt its content keys with the DKDM
/// recipient's private key, re-encrypt to `recipient_cert` and sign. Empty
/// `valid_from`/`valid_to` preserve the DKDM's validity window. `device_certs`
/// names the new recipient's devices, since the DKDM's own list names someone
/// else's.
#[allow(clippy::too_many_arguments)]
pub fn rewrap_dkdm(
    dkdm: PathBuf,
    dkdm_key: PathBuf,
    recipient_cert: PathBuf,
    signer_cert: PathBuf,
    signer_key: PathBuf,
    signer_chain: Vec<PathBuf>,
    valid_from: String,
    valid_to: String,
    output: PathBuf,
    device_certs: Vec<PathBuf>,
    options: KdmOptions,
) -> i32 {
    let formulation = match resolve_formulation(options.formulation, device_certs.len()) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let config = postkit::certificate::RewrapConfig {
        dkdm_file: dkdm,
        dkdm_recipient_key_file: dkdm_key,
        recipient_cert_file: recipient_cert,
        signer_cert_file: signer_cert,
        signer_key_file: signer_key,
        signer_chain_files: signer_chain,
        output_file: output,
        valid_from,
        valid_to,
        device_cert_files: device_certs,
        formulation,
        picture_forensic_marking: options.picture_forensic_marking,
        audio_forensic_marking: options.audio_forensic_marking,
    };
    match postkit::certificate::rewrap_dkdm_to_file(&config) {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These exercise the real postkit call and the Result->exit-code mapping.
    // The crypto/XML/signature behaviour is postkit's and is tested there.

    #[test]
    fn generate_kdm_empty_cpl_id_fails() {
        let out = tempfile::NamedTempFile::new().unwrap();
        let code = generate_kdm(
            String::new(),
            "Test".into(),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
            Vec::new(),
            "now".into(),
            "2 weeks".into(),
            Vec::new(),
            out.path().to_path_buf(),
            KdmFormat::Smpte,
            None,
            None,
            Vec::new(),
            Default::default(),
        );
        assert_ne!(code, 0);
    }

    #[test]
    fn duration_end_keeps_start_offset() {
        // start in +02:00; a 1-day duration must land at the same wall clock in
        // +02:00, not be relabeled +00:00 (which would be 2 hours off).
        let (from, to) = resolve_validity("2024-06-01T00:00:00+02:00", "1 day").unwrap();
        assert_eq!(from, "2024-06-01T00:00:00+02:00");
        assert_eq!(to, "2024-06-02T00:00:00+02:00");
    }

    #[test]
    fn absolute_end_passes_through() {
        let (_, to) =
            resolve_validity("2024-06-01T00:00:00+00:00", "2024-06-15T00:00:00+00:00").unwrap();
        assert_eq!(to, "2024-06-15T00:00:00+00:00");
    }

    #[test]
    fn rewrap_dkdm_missing_file_fails() {
        let out = tempfile::NamedTempFile::new().unwrap();
        let code = rewrap_dkdm(
            PathBuf::from("/nonexistent/dkdm.xml"),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
            Vec::new(),
            String::new(),
            String::new(),
            out.path().to_path_buf(),
            Vec::new(),
            Default::default(),
        );
        assert_ne!(code, 0);
    }

    #[test]
    fn certs_in_dir_lists_only_certs_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.pem"), "x").unwrap();
        std::fs::write(dir.path().join("a.crt"), "x").unwrap();
        std::fs::write(dir.path().join("k.key"), "x").unwrap(); // private key, excluded
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let found = certs_in_dir(dir.path()).unwrap();
        assert_eq!(found.len(), 2, "only cert extensions counted");
        assert!(
            found[0].ends_with("a.crt") && found[1].ends_with("b.pem"),
            "sorted"
        );
    }

    #[test]
    fn certs_in_dir_empty_or_missing_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            certs_in_dir(dir.path()).is_err(),
            "empty dir must fail loud"
        );
        assert!(certs_in_dir(Path::new("/nonexistent/certs")).is_err());
    }

    #[test]
    fn batch_reports_failure_for_a_bad_recipient() {
        let dir = tempfile::tempdir().unwrap();
        let code = generate_kdm_batch(
            "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9".into(),
            "T".into(),
            vec![PathBuf::from("/nonexistent/recipient.pem")],
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
            Vec::new(),
            signable_window().0,
            signable_window().1,
            Vec::new(),
            dir.path().join("out"),
            KdmFormat::Smpte,
            None,
            None,
            Vec::new(),
            Default::default(),
        );
        assert_ne!(code, 0);
    }

    use postkit::certificate::{CertOptions, CertType, generate_certificate, generate_chain};

    /// Generate a signer chain in `dir` plus `n` recipient leaf certs in a
    /// `recipients/` subdir. Returns (signer_cert, signer_key, chain, recipient_dir).
    fn batch_fixtures(dir: &Path, n: usize) -> (PathBuf, PathBuf, Vec<PathBuf>, PathBuf) {
        assert_eq!(generate_chain("Acme", dir), 0, "chain generation failed");
        let recipients = dir.join("recipients");
        std::fs::create_dir_all(&recipients).unwrap();
        for i in 0..n {
            let opts = CertOptions {
                cert_type: CertType::Leaf,
                common_name: format!("Screen {i}"),
                organization: "Cinema".into(),
                output_cert: recipients.join(format!("screen_{i}.pem")),
                output_key: recipients.join(format!("screen_{i}.key")),
                issuer_cert: dir.join("root.pem"),
                issuer_key: dir.join("root.key"),
                ..Default::default()
            };
            assert_eq!(generate_certificate(&opts), 0, "recipient {i} gen failed");
        }
        (
            dir.join("signer.pem"),
            dir.join("signer.key"),
            vec![dir.join("intermediate.pem"), dir.join("root.pem")],
            recipients,
        )
    }

    /// A KDM window a freshly minted chain can actually sign: postkit follows
    /// libdcp and compares at day granularity, counting an equal day as a
    /// failure, so a certificate minted today cannot sign a window starting
    /// today. Real chains are years old; test chains are minutes old.
    fn signable_window() -> (String, String) {
        let start = chrono::Utc::now() + chrono::Duration::days(1);
        let end = start + chrono::Duration::days(7);
        (
            start.format("%Y-%m-%dT%H:%M:%S+00:00").to_string(),
            end.format("%Y-%m-%dT%H:%M:%S+00:00").to_string(),
        )
    }

    /// xmlsec 1.3 searches keys strictly and prints no error detail, so a KDM
    /// whose KeyInfo does not name its key needs --lax-key-search and the
    /// failure text needs --verbose. Neither flag exists in 1.2.
    fn xmlsec1_compatibility_flags() -> &'static [&'static str] {
        static FLAGS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
        FLAGS
            .get_or_init(|| {
                // --help is a summary, the option list is under --help-all
                let mut help = String::new();
                for arg in ["--help", "--help-all"] {
                    if let Ok(out) = std::process::Command::new("xmlsec1").arg(arg).output() {
                        help.push_str(&String::from_utf8_lossy(&out.stdout));
                        help.push_str(&String::from_utf8_lossy(&out.stderr));
                    }
                }
                ["--lax-key-search", "--verbose"]
                    .into_iter()
                    .filter(|flag| help.contains(flag))
                    .collect()
            })
            .as_slice()
    }

    /// A KDM signs two subtrees by Id, so the verify needs both --id-attr hints.
    fn assert_kdm_verifies(kdm: &Path, root_pem: &Path, what: &str) {
        let out = std::process::Command::new("xmlsec1")
            .arg("--verify")
            .args(xmlsec1_compatibility_flags())
            .arg("--trusted-pem")
            .arg(root_pem)
            .args(["--id-attr:Id", "AuthenticatedPublic"])
            .args(["--id-attr:Id", "AuthenticatedPrivate"])
            .arg(kdm)
            .output()
            .expect("run xmlsec1");
        assert!(
            out.status.success(),
            "xmlsec1 must verify the {what} against the root: {}\n  flags: {:?}\n  status: {}\n  stdout: {}\n  stderr: {}",
            kdm.display(),
            xmlsec1_compatibility_flags(),
            out.status,
            String::from_utf8_lossy(&out.stdout).trim(),
            String::from_utf8_lossy(&out.stderr).trim(),
        );
    }

    // Real end-to-end batch: distinct recipients, bound content KeyId, signed.
    #[test]
    fn batch_generates_one_signed_kdm_per_recipient_bound_to_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let (signer_cert, signer_key, chain, recipients) = batch_fixtures(dir.path(), 2);
        let certs = certs_in_dir(&recipients).expect("recipients found");
        assert_eq!(certs.len(), 2, "cert_dir globbing skips the .key files");

        // A known content KeyId, as if taken from the DCP's KEYS.json.
        let key_id = uuid::Uuid::new_v4();
        let content_keys = vec![postkit::certificate::KdmContentKey {
            key_type: *b"MDIK",
            key_id,
            content_key: [7u8; 16],
        }];

        let cpl_id = "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9";
        let out = dir.path().join("kdms");
        let history = dir.path().join("history.jsonl");
        let code = generate_kdm_batch(
            cpl_id.into(),
            "Test Feature".into(),
            certs.iter().map(PathBuf::from).collect(),
            signer_cert,
            signer_key,
            chain,
            signable_window().0,
            signable_window().1,
            content_keys,
            out.clone(),
            KdmFormat::Smpte,
            None,
            Some(history.clone()),
            Vec::new(),
            Default::default(),
        );
        assert_eq!(code, 0, "batch must succeed for every recipient");

        // dom#1014: one history record per successful recipient, metadata only
        let recs = crate::kdm_log::read_all(&history).unwrap();
        assert_eq!(recs.len(), 2, "one history record per KDM");
        assert_eq!(recs[0].content_title, "Test Feature");
        assert!(!recs[0].recipient_serial.is_empty(), "serial cached in log");
        let raw = std::fs::read_to_string(&history).unwrap();
        assert!(
            !raw.to_lowercase().contains("key"),
            "no key material in history"
        );

        let kdms: Vec<PathBuf> = std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("xml"))
            .collect();
        assert_eq!(kdms.len(), 2, "one KDM per recipient");

        for kdm in &kdms {
            let xml = std::fs::read_to_string(kdm).unwrap();
            assert!(
                xml.contains(&format!(
                    "<CompositionPlaylistId>urn:uuid:{cpl_id}</CompositionPlaylistId>"
                )),
                "KDM must reference the CPL"
            );
            assert!(
                xml.contains(
                    "<MessageType>http://www.smpte-ra.org/430-1/2006/KDM#kdm-key-type</MessageType>"
                ),
                "standard SMPTE KDM MessageType"
            );
            assert!(
                xml.contains(&format!("<KeyId>urn:uuid:{key_id}</KeyId>")),
                "KeyId must match the DCP's content key"
            );
            assert!(xml.contains("<ds:Signature"), "KDM must be signed");

            assert_kdm_verifies(kdm, &dir.path().join("root.pem"), "batch KDM");
        }
    }

    // --annotation override lands as the KDM's AnnotationText (escaped).
    #[test]
    fn annotation_override_lands_in_kdm_xml() {
        let dir = tempfile::tempdir().unwrap();
        let (signer_cert, signer_key, chain, recipients) = batch_fixtures(dir.path(), 1);
        let recipient = certs_in_dir(&recipients).unwrap()[0].clone();
        let out = dir.path().join("annotated.kdm.xml");
        let code = generate_kdm(
            "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9".into(),
            "Feature".into(),
            PathBuf::from(recipient),
            signer_cert,
            signer_key,
            chain,
            signable_window().0,
            signable_window().1,
            Vec::new(),
            out.clone(),
            KdmFormat::Smpte,
            Some("Release KDM <v2> & final".into()),
            None,
            Vec::new(),
            Default::default(),
        );
        assert_eq!(code, 0, "annotated KDM must generate");
        let xml = std::fs::read_to_string(&out).unwrap();
        assert!(
            xml.contains("<AnnotationText>Release KDM &lt;v2&gt; &amp; final</AnnotationText>"),
            "the --annotation override must appear, escaped, in the KDM"
        );
    }

    // --device-cert must reach postkit's AuthorizedDeviceInfo, and naming any
    // device must drop the assume-trust entry rather than sit beside it.
    #[test]
    fn device_certs_replace_the_assume_trust_thumbprint() {
        // DCI DCSS 9.4.3.5 assume-trust marker: base64 SHA-1 of nothing.
        const ASSUME_TRUST: &str = "2jmj7l5rSw0yVb/vlWAYkK/YBwk=";

        let dir = tempfile::tempdir().unwrap();
        let (signer_cert, signer_key, chain, recipients) = batch_fixtures(dir.path(), 2);
        let certs = certs_in_dir(&recipients).unwrap();
        let recipient = PathBuf::from(&certs[0]);
        let device = PathBuf::from(&certs[1]);

        let kdm_for = |name: &str, devices: Vec<PathBuf>| {
            let out = dir.path().join(name);
            let code = generate_kdm(
                "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9".into(),
                "Device Test".into(),
                recipient.clone(),
                signer_cert.clone(),
                signer_key.clone(),
                chain.clone(),
                signable_window().0,
                signable_window().1,
                Vec::new(),
                out.clone(),
                KdmFormat::Smpte,
                None,
                None,
                devices,
                Default::default(),
            );
            assert_eq!(code, 0, "{name} must generate");
            std::fs::read_to_string(&out).unwrap()
        };

        let open = kdm_for("open.kdm.xml", Vec::new());
        assert!(
            open.contains(ASSUME_TRUST),
            "no --device-cert leaves the KDM playable on any trusted device"
        );

        let restricted = kdm_for("restricted.kdm.xml", vec![device.clone()]);
        assert!(
            !restricted.contains(ASSUME_TRUST),
            "a listed device must disable assume-trust, not join it"
        );
        assert_eq!(
            restricted.matches("<CertificateThumbprint>").count(),
            1,
            "one thumbprint for the one device named"
        );
        // The thumbprint has to be that device's, not the recipient's.
        let recipient_only = kdm_for("recipient-device.kdm.xml", vec![recipient.clone()]);
        let thumbprint_of = |xml: &str| {
            let start =
                xml.find("<CertificateThumbprint>").unwrap() + "<CertificateThumbprint>".len();
            let end = xml[start..].find('<').unwrap() + start;
            xml[start..end].to_string()
        };
        assert_ne!(
            thumbprint_of(&restricted),
            thumbprint_of(&recipient_only),
            "the device list must follow --device-cert, not the recipient"
        );
    }

    // Interop KDM: digicine namespace, signed, xmlsec1-verifiable. The 134-byte
    // (vs SMPTE 138) key block is inside the RSA-encrypted CipherData and is
    // asserted in postkit's own interop_kdm_key_block_is_134_bytes test.
    #[test]
    fn interop_kdm_uses_digicine_namespace_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let (signer_cert, signer_key, chain, recipients) = batch_fixtures(dir.path(), 1);
        let recipient = certs_in_dir(&recipients).unwrap()[0].clone();

        let key_id = uuid::Uuid::new_v4();
        let content_keys = vec![postkit::certificate::KdmContentKey {
            key_type: *b"MDIK",
            key_id,
            content_key: [3u8; 16],
        }];

        let out = dir.path().join("interop.kdm.xml");
        let code = generate_kdm(
            "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9".into(),
            "Interop Feature".into(),
            PathBuf::from(recipient),
            signer_cert,
            signer_key,
            chain,
            signable_window().0,
            signable_window().1,
            content_keys,
            out.clone(),
            KdmFormat::Interop,
            None,
            None,
            Vec::new(),
            Default::default(),
        );
        assert_eq!(code, 0, "interop KDM generation must succeed");

        let xml = std::fs::read_to_string(&out).unwrap();
        assert!(
            xml.contains("http://www.digicine.com/PROTO-ASDCP-KDM-20040311#"),
            "interop KDM must use the digicine namespace"
        );
        assert!(xml.contains("<ds:Signature"), "interop KDM must be signed");

        assert_kdm_verifies(&out, &dir.path().join("root.pem"), "interop KDM");
    }
}
