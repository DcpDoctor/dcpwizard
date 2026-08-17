//! download a projector/server recipient certificate by vendor + serial
//! (dom#2705, dom#2706). only vendors whose public endpoint was verified live
//! are implemented; others return one clear line telling the user to get the
//! cert from the vendor. downloads are validated as parseable X.509 before being
//! written. ftp/sftp fetching is delegated to the system `curl` binary (same
//! transport dcp-o-matic uses via libcurl); the serial is validated first and
//! passed as a plain argument, never through a shell.
//!
//! verified endpoints (anonymous ftp, directory listing confirmed 2026-07):
//!   dolby/doremi: ftp://ftp.cinema.dolby.com/Certificates/<first3>xxx/
//!   qube:         ftp://certificates.qubecinema.com/SMPTE-<type>/
//!
//! credentialed vendors (vendor account required, --user/--password; the
//! password is embedded in the curl url but never logged, dom#2705/2706):
//!   christie: ftp://certificates.christiedigital.com/Certificates/{F-IMB,IMB-S2}/
//!   gdc:      ftp://ftp.gdc-tech.com/SHA256/<serial>.crt.pem
//!   barco:    sftp://certificates.barco.com/<serial[0:7]>xxx/<serial>/Barco-ICMP...

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Vendor {
    /// dolby / doremi digit-serial servers (DCP2000/IMB/IMS): sha256 leaf PEM.
    Dolby,
    /// qube cinema servers, keyed by device type (e.g. QXPD).
    Qube,
    /// christie F-IMB / IMB-S2 (credentialed ftp), serial zero-padded to 12.
    Christie,
    /// gdc SHA256 leaf (credentialed ftp).
    Gdc,
    /// barco ICMP (credentialed sftp), serial exactly 10 chars.
    Barco,
}

/// projector/server login for a credentialed vendor endpoint.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

const DOLBY_BASE: &str = "ftp://ftp.cinema.dolby.com/Certificates";
const QUBE_BASE: &str = "ftp://certificates.qubecinema.com";
const CHRISTIE_HOST: &str = "certificates.christiedigital.com";
const GDC_HOST: &str = "ftp.gdc-tech.com";
const BARCO_HOST: &str = "certificates.barco.com";

/// map a vendor name to a supported downloader, or a clear "get it from the
/// vendor" error for vendors without an automated endpoint.
pub fn parse_vendor(name: &str) -> Result<Vendor, String> {
    match name.to_ascii_lowercase().as_str() {
        "dolby" | "doremi" => Ok(Vendor::Dolby),
        "qube" => Ok(Vendor::Qube),
        "christie" => Ok(Vendor::Christie),
        "gdc" | "gdc-tech" => Ok(Vendor::Gdc),
        "barco" => Ok(Vendor::Barco),
        v @ ("sony" | "nec") => Err(format!(
            "automated download for {v} is not available; obtain the certificate from the vendor"
        )),
        other => Err(format!(
            "unknown vendor '{other}'; obtain the certificate from the vendor"
        )),
    }
}

/// true for vendors whose endpoint needs a vendor account (--user/--password).
pub fn needs_credentials(vendor: Vendor) -> bool {
    matches!(vendor, Vendor::Christie | Vendor::Gdc | Vendor::Barco)
}

/// reject anything but plain serial characters so nothing odd reaches curl or a
/// url path.
pub fn validate_serial(serial: &str) -> Result<(), String> {
    if serial.is_empty()
        || !serial
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(format!(
            "invalid serial '{serial}': use letters, digits and '-' only"
        ));
    }
    Ok(())
}

fn dolby_dir_url(serial: &str) -> Result<String, String> {
    let digits: String = serial.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 3 {
        return Err(format!(
            "dolby serial '{serial}' must start with at least 3 digits"
        ));
    }
    Ok(format!("{DOLBY_BASE}/{}xxx/", &digits[..3]))
}

/// among a directory listing, pick the sha256 leaf cert for this serial.
fn dolby_pick(entries: &[String], serial: &str) -> Option<String> {
    let want = format!("-{serial}.cert.sha256.pem");
    entries.iter().find(|e| e.ends_with(&want)).cloned()
}

fn qube_dir_url(qtype: &str) -> String {
    format!("{QUBE_BASE}/SMPTE-{qtype}/")
}

fn qube_pad(serial: &str) -> String {
    if serial.chars().all(|c| c.is_ascii_digit()) && serial.len() < 5 {
        format!("{serial:0>5}")
    } else {
        serial.to_string()
    }
}

/// pick the sha256 cert whose name starts with "<type>-<padded serial>-".
fn qube_pick(entries: &[String], qtype: &str, serial: &str) -> Option<String> {
    let prefix = format!("{qtype}-{}-", qube_pad(serial));
    let mut matches: Vec<&String> = entries
        .iter()
        .filter(|e| e.starts_with(&prefix) && e.contains("sha256"))
        .collect();
    matches.sort();
    matches.last().map(|s| (*s).clone())
}

/// christie serials are zero-padded to 12 digits; the leaf lives under F-IMB
/// first, then IMB-S2 as a fallback. returns both candidate url paths in order.
fn christie_paths(serial: &str) -> Result<Vec<String>, String> {
    if !serial.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("christie serial '{serial}' must be digits only"));
    }
    if serial.len() > 12 {
        return Err(format!(
            "christie serial '{serial}' is longer than 12 digits"
        ));
    }
    let s = format!("{serial:0>12}");
    Ok(vec![
        format!("/Certificates/F-IMB/F-IMB_{s}_sha256.pem"),
        format!("/Certificates/IMB-S2/IMB-S2_{s}_sha256.pem"),
    ])
}

fn gdc_path(serial: &str) -> String {
    format!("/SHA256/{serial}.crt.pem")
}

/// barco serials are exactly 10 chars; the leaf sits under a
/// <first7>xxx/<serial>/ directory.
fn barco_path(serial: &str) -> Result<String, String> {
    if serial.len() != 10 {
        return Err(format!(
            "barco serial '{serial}' must be exactly 10 characters"
        ));
    }
    Ok(format!(
        "/{}xxx/{serial}/Barco-ICMP.{serial}_cert.pem",
        &serial[..7]
    ))
}

/// download one credentialed url. credentials go through a curl config on stdin
/// (`-K -`), never argv or logs. sftp host-key checking is disabled to match the
/// vendor endpoints (same as dcp-o-matic's SSL-verify-off). the returned error
/// carries only the creds-free display url.
fn curl_creds(
    scheme: &str,
    host: &str,
    path: &str,
    creds: &Credentials,
    display_url: &str,
) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut cmd = Command::new("curl");
    cmd.arg("-s")
        .args(["--connect-timeout", "15"])
        .args(["--max-time", "45"]);
    if scheme == "sftp" {
        cmd.arg("-k");
    }
    cmd.arg("-K").arg("-");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot run curl (is it installed?): {e}"))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or("cannot pass credentials to curl")?;
        // curl config: url + user are read here so the password never hits argv.
        let config = format!(
            "url = \"{scheme}://{host}{path}\"\nuser = \"{}:{}\"\n",
            creds.user, creds.password
        );
        stdin
            .write_all(config.as_bytes())
            .map_err(|e| format!("cannot write curl config: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("curl failed: {e}"))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let code = out.status.code().unwrap_or(-1);
    let msg = match code {
        6 => "could not resolve host",
        7 => "could not connect to host",
        28 => "connection timed out",
        9 | 19 | 78 => "file not found on server",
        67 => "authentication failed (check --user/--password)",
        _ => "download failed",
    };
    Err(format!("{msg} at {display_url} (curl exit {code})"))
}

/// run `curl` for one url, returning stdout bytes. distinguishes an unreachable
/// host from a missing file so callers can report a useful message (dom#2705).
fn curl(url: &str, list: bool) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new("curl");
    cmd.arg("-s")
        .args(["--connect-timeout", "15"])
        .args(["--max-time", "45"]);
    if list {
        // "-l" name-only listing for an ftp directory url.
        cmd.arg("-l");
    }
    cmd.arg(url);
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run curl (is it installed?): {e}"))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let code = out.status.code().unwrap_or(-1);
    // curl exit codes: 6 resolve, 7 connect, 28 timeout -> network; else likely
    // a missing path.
    let msg = match code {
        6 => "could not resolve host",
        7 => "could not connect to host",
        28 => "connection timed out",
        9 | 19 | 78 => "file not found on server",
        _ => "download failed",
    };
    Err(format!("{msg} (curl exit {code})"))
}

/// fetch the raw PEM from a credentialed vendor, trying each candidate path in
/// order (christie has an F-IMB then IMB-S2 fallback).
fn fetch_credentialed(vendor: Vendor, serial: &str, creds: &Credentials) -> Result<String, String> {
    let (scheme, host, paths) = match vendor {
        Vendor::Christie => ("ftp", CHRISTIE_HOST, christie_paths(serial)?),
        Vendor::Gdc => ("ftp", GDC_HOST, vec![gdc_path(serial)]),
        Vendor::Barco => ("sftp", BARCO_HOST, vec![barco_path(serial)?]),
        _ => unreachable!("not a credentialed vendor"),
    };

    let mut last_err = String::new();
    for path in &paths {
        let display = format!("{scheme}://{host}{path}");
        match curl_creds(scheme, host, path, creds, &display) {
            Ok(bytes) => return Ok(String::from_utf8_lossy(&bytes).to_string()),
            Err(e) => last_err = e,
        }
    }
    Err(format!("no certificate for serial '{serial}': {last_err}"))
}

/// download, validate and write a recipient cert. returns "subject / serial" on
/// success. `qube_type` is required for qube and ignored for others; `creds` is
/// required for the credentialed vendors (christie/gdc/barco) and ignored for
/// the anonymous ones.
pub fn fetch(
    vendor: Vendor,
    serial: &str,
    qube_type: Option<&str>,
    creds: Option<&Credentials>,
    out: &Path,
) -> Result<String, String> {
    validate_serial(serial)?;

    if needs_credentials(vendor) {
        let creds = creds.ok_or_else(|| {
            "this vendor needs a vendor account; pass --user and --password".to_string()
        })?;
        let pem = fetch_credentialed(vendor, serial, creds)?;
        let info = postkit::certificate::cert_info_from_pem(&pem)?;
        postkit::fs::write_atomic(out, pem.as_bytes())?;
        return Ok(format!("{} / {}", info.subject_cn, info.serial));
    }

    let (dir_url, file) = match vendor {
        Vendor::Christie | Vendor::Gdc | Vendor::Barco => unreachable!("handled above"),
        Vendor::Dolby => {
            let dir = dolby_dir_url(serial)?;
            let listing = curl(&dir, true)?;
            let entries: Vec<String> = String::from_utf8_lossy(&listing)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            let file = dolby_pick(&entries, serial)
                .ok_or_else(|| format!("no certificate for dolby serial '{serial}' at {dir}"))?;
            (dir, file)
        }
        Vendor::Qube => {
            let qtype = qube_type.ok_or("qube requires --type (the device type, e.g. QXPD)")?;
            validate_serial(qtype).map_err(|_| {
                format!("invalid device type '{qtype}': use letters, digits and '-' only")
            })?;
            let dir = qube_dir_url(qtype);
            let listing = curl(&dir, true)?;
            let entries: Vec<String> = String::from_utf8_lossy(&listing)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            let file = qube_pick(&entries, qtype, serial).ok_or_else(|| {
                format!("no certificate for qube {qtype} serial '{serial}' at {dir}")
            })?;
            (dir, file)
        }
    };

    let bytes = curl(&format!("{dir_url}{file}"), false)?;
    let pem = String::from_utf8_lossy(&bytes).to_string();
    // untrusted download: must parse as X.509 before we store it.
    let info = postkit::certificate::cert_info_from_pem(&pem)?;
    postkit::fs::write_atomic(out, pem.as_bytes())?;
    Ok(format!("{} / {}", info.subject_cn, info.serial))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_parsing_routes_correctly() {
        assert_eq!(parse_vendor("dolby").unwrap(), Vendor::Dolby);
        assert_eq!(parse_vendor("Doremi").unwrap(), Vendor::Dolby);
        assert_eq!(parse_vendor("qube").unwrap(), Vendor::Qube);
        // known-but-unsupported vendors tell the user to get it from the vendor
        let e = parse_vendor("sony").unwrap_err();
        assert!(e.contains("obtain the certificate from the vendor"));
        assert!(parse_vendor("acme").is_err());
    }

    #[test]
    fn serial_validation_blocks_odd_input() {
        assert!(validate_serial("218281828").is_ok());
        assert!(validate_serial("H123-45").is_ok());
        assert!(validate_serial("").is_err());
        assert!(validate_serial("../etc/passwd").is_err());
        assert!(validate_serial("a b").is_err());
    }

    #[test]
    fn dolby_dir_and_pick() {
        assert_eq!(
            dolby_dir_url("218281828").unwrap(),
            "ftp://ftp.cinema.dolby.com/Certificates/218xxx/"
        );
        assert!(dolby_dir_url("ab").is_err());
        // matches the real dolby naming (Dolby-DCP2000-<serial>.cert.sha256.pem)
        let entries = vec![
            "Dolby-DCP2000-218281828.cert.mpeg.pem".to_string(),
            "Dolby-DCP2000-218281828.cert.sha256.pem".to_string(),
            "Dolby-DCP2000-218281828.dcicerts.zip".to_string(),
        ];
        assert_eq!(
            dolby_pick(&entries, "218281828").unwrap(),
            "Dolby-DCP2000-218281828.cert.sha256.pem"
        );
        assert!(dolby_pick(&entries, "999999999").is_none());
    }

    #[test]
    fn qube_pad_and_pick() {
        assert_eq!(qube_pad("54"), "00054");
        assert_eq!(qube_pad("00054"), "00054");
        // matches real qube naming (QXPD-00054-10-14-sha256.pem.crt)
        let entries = vec![
            "QXPD-00054-10-14-sha256.pem.crt".to_string(),
            "QXPD-00097-08-14-sha256.pem.crt".to_string(),
        ];
        assert_eq!(
            qube_pick(&entries, "QXPD", "54").unwrap(),
            "QXPD-00054-10-14-sha256.pem.crt"
        );
        assert!(qube_pick(&entries, "QXPD", "12345").is_none());
    }

    #[test]
    fn credentialed_vendors_parse_and_flag() {
        assert_eq!(parse_vendor("christie").unwrap(), Vendor::Christie);
        assert_eq!(parse_vendor("GDC").unwrap(), Vendor::Gdc);
        assert_eq!(parse_vendor("gdc-tech").unwrap(), Vendor::Gdc);
        assert_eq!(parse_vendor("barco").unwrap(), Vendor::Barco);
        for v in [Vendor::Christie, Vendor::Gdc, Vendor::Barco] {
            assert!(needs_credentials(v));
        }
        for v in [Vendor::Dolby, Vendor::Qube] {
            assert!(!needs_credentials(v));
        }
        // sony/nec still route to the get-it-from-the-vendor message
        assert!(
            parse_vendor("sony")
                .unwrap_err()
                .contains("obtain the certificate")
        );
    }

    #[test]
    fn christie_pads_serial_and_orders_fallback() {
        let paths = christie_paths("218281").unwrap();
        assert_eq!(
            paths,
            vec![
                "/Certificates/F-IMB/F-IMB_000000218281_sha256.pem".to_string(),
                "/Certificates/IMB-S2/IMB-S2_000000218281_sha256.pem".to_string(),
            ]
        );
        // already 12 digits stays put
        assert_eq!(
            christie_paths("123456789012").unwrap()[0],
            "/Certificates/F-IMB/F-IMB_123456789012_sha256.pem"
        );
        assert!(christie_paths("12A45").is_err()); // non-digit
        assert!(christie_paths("1234567890123").is_err()); // too long
    }

    #[test]
    fn gdc_path_uses_serial_as_is() {
        assert_eq!(gdc_path("H1234"), "/SHA256/H1234.crt.pem");
    }

    #[test]
    fn barco_requires_ten_chars_and_splits_dir() {
        assert_eq!(
            barco_path("1234567890").unwrap(),
            "/1234567xxx/1234567890/Barco-ICMP.1234567890_cert.pem"
        );
        assert!(barco_path("123").is_err());
        assert!(barco_path("12345678901").is_err());
    }
}
