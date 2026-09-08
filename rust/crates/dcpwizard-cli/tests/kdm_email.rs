//! `kdm-batch --smtp-config` against a local SMTP sink: one email per cinema in
//! the cinema db, each carrying only that cinema's KDMs, and a refused recipient
//! named in the failure.

use assert_cmd::Command;
use base64::Engine;
use postkit::certificate::{CertOptions, CertType, generate_certificate, generate_chain};
use predicates::prelude::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const CPL_ID: &str = "urn:uuid:6f2b1a5e-4c8d-4f3a-9b1e-7c0a2d5e8f31";
const TITLE: &str = "Big Feature";
const ODEON_EMAIL: &str = "keys@odeon.test";
const REX_EMAIL: &str = "keys@rex.test";

// one message the sink accepted, as the envelope and the DATA body
#[derive(Clone)]
struct Received {
    recipients: Vec<String>,
    body: String,
}

impl Received {
    fn header(&self, name: &str) -> String {
        let prefix = format!("{name}: ");
        self.body
            .lines()
            .find(|l| l.starts_with(&prefix))
            .unwrap_or_else(|| panic!("no {name} header in\n{}", self.body))
            .strip_prefix(&prefix)
            .unwrap()
            .trim_end()
            .to_string()
    }

    // the base64 attachment part, decoded
    fn attachment(&self) -> Vec<u8> {
        let zip_part = self
            .body
            .find("Content-Type: application/zip")
            .unwrap_or_else(|| panic!("no zip attachment in\n{}", self.body));
        let after_headers = self.body[zip_part..]
            .find("\r\n\r\n")
            .expect("attachment part has no body");
        let payload = &self.body[zip_part + after_headers + 4..];
        let encoded: String = payload
            .lines()
            .take_while(|l| !l.starts_with("--"))
            .collect();
        base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .expect("attachment is not base64")
    }

    fn zip_entries(&self) -> Vec<(String, String)> {
        let bytes = self.attachment();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("not a zip");
        let names: Vec<String> = archive.file_names().map(|n| n.to_string()).collect();
        names
            .into_iter()
            .map(|name| {
                let mut file = archive.by_name(&name).unwrap();
                let mut text = String::new();
                std::io::Read::read_to_string(&mut file, &mut text).unwrap();
                (name, text)
            })
            .collect()
    }
}

// enough of RFC 5321 to record what a client sends, refusing `refused` at RCPT TO
struct SmtpSink {
    port: u16,
    received: Arc<Mutex<Vec<Received>>>,
}

impl SmtpSink {
    fn start(refused: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = received.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &refused, &sink);
            }
        });
        Self { port, received }
    }

    fn messages(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }

    fn message_to(&self, address: &str) -> Received {
        let all = self.messages();
        all.iter()
            .find(|m| m.recipients.iter().any(|r| r == address))
            .unwrap_or_else(|| panic!("no message to {address}, got {} in all", all.len()))
            .clone()
    }
}

fn serve(stream: TcpStream, refused: &[String], sink: &Arc<Mutex<Vec<Received>>>) {
    let mut out = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    let mut say = |text: &str| {
        out.write_all(text.as_bytes()).unwrap();
        out.flush().unwrap();
    };
    say("220 sink ESMTP\r\n");
    let mut recipients: Vec<String> = Vec::new();
    let mut line = String::new();
    while {
        line.clear();
        reader.read_line(&mut line).unwrap_or(0) > 0
    } {
        let command = line.trim_end().to_string();
        let upper = command.to_ascii_uppercase();
        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            say("250 sink\r\n");
        } else if upper.starts_with("MAIL FROM") {
            say("250 sender ok\r\n");
        } else if upper.starts_with("RCPT TO") {
            let address = angle_address(&command);
            if refused.contains(&address) {
                say("550 no such mailbox here\r\n");
            } else {
                recipients.push(address);
                say("250 recipient ok\r\n");
            }
        } else if upper.starts_with("DATA") {
            say("354 send it\r\n");
            let mut body = String::new();
            loop {
                let mut data_line = String::new();
                if reader.read_line(&mut data_line).unwrap_or(0) == 0 {
                    return;
                }
                if data_line.trim_end() == "." {
                    break;
                }
                body.push_str(&data_line);
            }
            sink.lock().unwrap().push(Received {
                recipients: std::mem::take(&mut recipients),
                body,
            });
            say("250 queued\r\n");
        } else if upper.starts_with("QUIT") {
            say("221 bye\r\n");
            return;
        } else if upper.starts_with("RSET") || upper.starts_with("NOOP") {
            say("250 ok\r\n");
        } else {
            say("502 not implemented\r\n");
        }
    }
}

fn angle_address(command: &str) -> String {
    let open = command.find('<').map(|i| i + 1).unwrap_or(0);
    let close = command.find('>').unwrap_or(command.len());
    command[open..close].to_string()
}

// a KDM starting on the day its signer certificate does is refused
fn tomorrow() -> String {
    (chrono::Utc::now() + chrono::Duration::days(1))
        .format("%Y-%m-%dT%H:%M:%S+00:00")
        .to_string()
}

fn recipient_cert(dir: &Path, chain: &Path, stem: &str) -> PathBuf {
    let cert = dir.join(format!("{stem}.pem"));
    let options = CertOptions {
        cert_type: CertType::Leaf,
        common_name: stem.into(),
        organization: "Cinema".into(),
        output_cert: cert.clone(),
        output_key: dir.join(format!("{stem}.key")),
        issuer_cert: chain.join("root.pem"),
        issuer_key: chain.join("root.key"),
        ..Default::default()
    };
    assert_eq!(generate_certificate(&options), 0, "recipient cert {stem}");
    cert
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    chain: PathBuf,
    db: PathBuf,
}

impl Fixture {
    // two cinemas, one screen each, in a cinema db built through the CLI
    fn build() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let chain = root.join("chain");
        assert_eq!(generate_chain("Studio", &chain), 0, "signer chain");
        let db = root.join("cinemas.json");
        let fixture = Self {
            _temp: temp,
            root,
            chain,
            db,
        };
        for (cinema, email, stem) in [
            ("Odeon", ODEON_EMAIL, "odeon-screen1"),
            ("Rex", REX_EMAIL, "rex-screen1"),
        ] {
            let cert = recipient_cert(&fixture.root, &fixture.chain, stem);
            fixture
                .command()
                .args(["cinema", "add", "--name", cinema, "--email", email])
                .arg("--db")
                .arg(&fixture.db)
                .assert()
                .success();
            fixture
                .command()
                .args([
                    "cinema",
                    "add-screen",
                    "--cinema",
                    cinema,
                    "--name",
                    "Screen 1",
                    "--cert",
                    cert.to_str().unwrap(),
                ])
                .arg("--db")
                .arg(&fixture.db)
                .assert()
                .success();
        }
        fixture
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("dcpwizard").unwrap();
        command
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"));
        command
    }

    fn smtp_config(&self, port: u16) -> PathBuf {
        let path = self.root.join("smtp.toml");
        std::fs::write(
            &path,
            format!(
                "host = \"127.0.0.1\"\n\
                 port = {port}\n\
                 security = \"none\"\n\
                 from = \"keys@studio.test\"\n\
                 subject_template = \"Keys for {{title}} at {{cinema}}\"\n\
                 body_template = \"KDMs for {{title}}, {{cinema}}\"\n"
            ),
        )
        .unwrap();
        path
    }

    fn email_batch(&self, port: u16) -> Command {
        let output = self.root.join("kdms");
        let mut command = self.command();
        command
            .args(["kdm-batch", "--cpl-id", CPL_ID, "--content-title", TITLE])
            .args(["--valid-from", &tomorrow(), "--valid-to", "2 weeks"])
            .arg("--db")
            .arg(&self.db)
            .args(["--cinema", "Odeon", "--cinema", "Rex"])
            .arg("--signer-cert")
            .arg(self.chain.join("signer.pem"))
            .arg("--signer-key")
            .arg(self.chain.join("signer.key"))
            .arg("--signer-chain")
            .arg(self.chain.join("intermediate.pem"))
            .arg("--signer-chain")
            .arg(self.chain.join("root.pem"))
            .arg("--output-dir")
            .arg(&output)
            .arg("--smtp-config")
            .arg(self.smtp_config(port));
        command
    }
}

#[test]
fn each_cinema_in_the_list_gets_its_own_kdms() {
    let fixture = Fixture::build();
    let sink = SmtpSink::start(Vec::new());

    fixture.email_batch(sink.port).assert().success();

    let messages = sink.messages();
    assert_eq!(messages.len(), 2, "one email per cinema");

    let odeon = sink.message_to(ODEON_EMAIL);
    assert_eq!(odeon.header("Subject"), "Keys for Big Feature at Odeon");
    assert_eq!(odeon.header("To"), ODEON_EMAIL);
    let entries = odeon.zip_entries();
    assert_eq!(entries.len(), 1, "only Odeon's KDM: {entries:?}");
    assert_eq!(entries[0].0, "001_odeon-screen1.kdm.xml");
    assert!(
        entries[0].1.contains("<KDMRequiredExtensions"),
        "the attachment holds a KDM"
    );
    assert!(
        entries[0]
            .1
            .contains(CPL_ID.trim_start_matches("urn:uuid:")),
        "the KDM is for the requested composition"
    );

    let rex = sink.message_to(REX_EMAIL);
    assert_eq!(rex.header("Subject"), "Keys for Big Feature at Rex");
    let rex_entries = rex.zip_entries();
    assert_eq!(rex_entries.len(), 1, "only Rex's KDM: {rex_entries:?}");
    assert_eq!(rex_entries[0].0, "001_rex-screen1.kdm.xml");
    assert_ne!(
        rex_entries[0].1, entries[0].1,
        "each cinema gets keys wrapped to its own certificate"
    );
}

#[test]
fn a_refused_recipient_names_its_cinema_and_the_rest_still_go() {
    let fixture = Fixture::build();
    let sink = SmtpSink::start(vec![REX_EMAIL.to_string()]);

    fixture
        .email_batch(sink.port)
        .assert()
        .failure()
        .stdout(predicate::str::contains("Rex").and(predicate::str::contains("550")));

    let messages = sink.messages();
    assert_eq!(messages.len(), 1, "the accepted cinema is still emailed");
    assert_eq!(messages[0].recipients, vec![ODEON_EMAIL.to_string()]);
}
