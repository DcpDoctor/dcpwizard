//! persistent cinema/screen database for kdm distribution (dom#776, dom#2707).
//! a cinema has contact emails and screens; each screen carries a recipient
//! certificate (a file path or inline PEM) plus cached serial/thumbprint/subject
//! so screens can be searched by name or by server certificate serial without
//! re-parsing every cert. no private key material is ever stored.

use crate::store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// where a screen's recipient certificate lives.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CertSource {
    /// a PEM/CRT file on disk, referenced by path.
    Path(PathBuf),
    /// the certificate PEM embedded directly in the db (e.g. from FLM import).
    Inline(String),
}

impl CertSource {
    /// produce a real file path for this cert. a Path source is used directly;
    /// an Inline source is written into `tmp_dir` so postkit can read it.
    pub fn materialize(&self, tmp_dir: &Path) -> Result<PathBuf, String> {
        match self {
            CertSource::Path(p) => Ok(p.clone()),
            CertSource::Inline(pem) => {
                let path = tmp_dir.join(format!("{}.pem", uuid::Uuid::new_v4()));
                std::fs::write(&path, pem).map_err(|e| format!("cannot write temp cert: {e}"))?;
                Ok(path)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Screen {
    pub name: String,
    pub cert: CertSource,
    /// cached from the certificate for search; not authoritative key material.
    pub cert_serial: String,
    pub cert_thumbprint: String,
    pub cert_subject: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Cinema {
    pub name: String,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub screens: Vec<Screen>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CinemaDb {
    #[serde(default)]
    pub cinemas: Vec<Cinema>,
}

/// a resolved recipient: which cinema/screen it came from, its contact emails,
/// and a usable cert file path. used by kdm-batch --cinema/--screen.
pub struct Recipient {
    pub cinema: String,
    pub emails: Vec<String>,
    pub screen: String,
    pub cert_path: PathBuf,
}

impl CinemaDb {
    /// load the db, returning an empty db if the file does not exist yet.
    /// corrupt json fails loud rather than silently discarding cinemas.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|e| format!("cannot parse cinema db {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("cannot read cinema db {}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| format!("serialize db: {e}"))?;
        store::atomic_write(path, &json)
    }

    pub fn find(&self, name: &str) -> Option<&Cinema> {
        self.cinemas.iter().find(|c| c.name == name)
    }

    pub fn add_cinema(
        &mut self,
        name: &str,
        emails: Vec<String>,
        notes: String,
    ) -> Result<(), String> {
        if self.find(name).is_some() {
            return Err(format!("cinema '{name}' already exists"));
        }
        self.cinemas.push(Cinema {
            name: name.to_string(),
            emails,
            notes,
            screens: Vec::new(),
        });
        Ok(())
    }

    pub fn remove_cinema(&mut self, name: &str) -> Result<(), String> {
        let before = self.cinemas.len();
        self.cinemas.retain(|c| c.name != name);
        if self.cinemas.len() == before {
            return Err(format!("cinema '{name}' not found"));
        }
        Ok(())
    }

    /// build a screen from a cert source, caching parsed serial/thumbprint/subject.
    /// the cert is validated as real X.509 here (untrusted input) and rejected
    /// otherwise.
    fn make_screen(name: &str, cert: CertSource) -> Result<Screen, String> {
        let info = match &cert {
            CertSource::Path(p) => store::cert_info_from_file(p)?,
            CertSource::Inline(pem) => store::cert_info_from_pem(pem)?,
        };
        Ok(Screen {
            name: name.to_string(),
            cert,
            cert_serial: info.serial,
            cert_thumbprint: info.thumbprint,
            cert_subject: info.subject_cn,
        })
    }

    pub fn add_screen(
        &mut self,
        cinema: &str,
        screen: &str,
        cert: CertSource,
    ) -> Result<(), String> {
        let s = Self::make_screen(screen, cert)?;
        let c = self
            .cinemas
            .iter_mut()
            .find(|c| c.name == cinema)
            .ok_or_else(|| format!("cinema '{cinema}' not found"))?;
        if c.screens.iter().any(|x| x.name == screen) {
            return Err(format!("screen '{screen}' already exists in '{cinema}'"));
        }
        c.screens.push(s);
        Ok(())
    }

    pub fn remove_screen(&mut self, cinema: &str, screen: &str) -> Result<(), String> {
        let c = self
            .cinemas
            .iter_mut()
            .find(|c| c.name == cinema)
            .ok_or_else(|| format!("cinema '{cinema}' not found"))?;
        let before = c.screens.len();
        c.screens.retain(|s| s.name != screen);
        if c.screens.len() == before {
            return Err(format!("screen '{screen}' not found in '{cinema}'"));
        }
        Ok(())
    }

    /// find cinemas/screens matching a query: cinema name substring, screen name
    /// substring, or certificate serial/thumbprint substring (dom#2707).
    /// case-insensitive.
    pub fn search(&self, query: &str) -> Vec<(String, String)> {
        let q = query.to_lowercase();
        let mut hits = Vec::new();
        for c in &self.cinemas {
            let cinema_match = c.name.to_lowercase().contains(&q);
            for s in &c.screens {
                if cinema_match
                    || s.name.to_lowercase().contains(&q)
                    || s.cert_serial.to_lowercase().contains(&q)
                    || s.cert_thumbprint.to_lowercase().contains(&q)
                    || s.cert_subject.to_lowercase().contains(&q)
                {
                    hits.push((c.name.clone(), s.name.clone()));
                }
            }
            // a cinema with no screens still matches by name
            if cinema_match && c.screens.is_empty() {
                hits.push((c.name.clone(), String::new()));
            }
        }
        hits
    }

    /// import a facility from an FLM-x file (dom#239): create/replace the cinema,
    /// adding a screen per device with its recipient (leaf) certificate.
    pub fn import_flm(&mut self, flm_path: &Path) -> Result<String, String> {
        let xml = std::fs::read_to_string(flm_path)
            .map_err(|e| format!("cannot read FLM {}: {e}", flm_path.display()))?;
        let parsed = crate::flm::parse(&xml)?;

        let mut cinema = Cinema {
            name: parsed.name.clone(),
            emails: parsed.emails,
            notes: String::new(),
            screens: Vec::new(),
        };
        for fs in parsed.screens {
            // pick the recipient leaf: the first non-CA cert that parses.
            let mut leaf: Option<Screen> = None;
            // skip CA certs in the chain; the recipient is the leaf (non-CA)
            for pem in &fs.cert_pems {
                if let Ok(info) = store::cert_info_from_pem(pem)
                    && !info.is_ca
                {
                    leaf = Some(Screen {
                        name: fs.name.clone(),
                        cert: CertSource::Inline(pem.clone()),
                        cert_serial: info.serial,
                        cert_thumbprint: info.thumbprint,
                        cert_subject: info.subject_cn,
                    });
                    break;
                }
            }
            match leaf {
                Some(s) => cinema.screens.push(s),
                None => {
                    return Err(format!(
                        "screen '{}' has no usable leaf certificate in the FLM",
                        fs.name
                    ));
                }
            }
        }

        // replace any existing cinema of the same name
        self.cinemas.retain(|c| c.name != cinema.name);
        let summary = format!("{} ({} screens)", cinema.name, cinema.screens.len());
        self.cinemas.push(cinema);
        Ok(summary)
    }

    /// resolve --cinema names and --screen "cinema/screen" specs into recipients
    /// with usable cert file paths (inline certs written under `tmp_dir`).
    pub fn resolve(
        &self,
        cinemas: &[String],
        screens: &[String],
        tmp_dir: &Path,
    ) -> Result<Vec<Recipient>, String> {
        let mut out = Vec::new();
        for name in cinemas {
            let c = self
                .find(name)
                .ok_or_else(|| format!("cinema '{name}' not found in db"))?;
            if c.screens.is_empty() {
                return Err(format!("cinema '{name}' has no screens"));
            }
            for s in &c.screens {
                out.push(Recipient {
                    cinema: c.name.clone(),
                    emails: c.emails.clone(),
                    screen: s.name.clone(),
                    cert_path: s.cert.materialize(tmp_dir)?,
                });
            }
        }
        for spec in screens {
            let (cn, sn) = spec
                .split_once('/')
                .ok_or_else(|| format!("--screen must be 'cinema/screen', got '{spec}'"))?;
            let c = self
                .find(cn)
                .ok_or_else(|| format!("cinema '{cn}' not found in db"))?;
            let s = c
                .screens
                .iter()
                .find(|s| s.name == sn)
                .ok_or_else(|| format!("screen '{sn}' not found in '{cn}'"))?;
            out.push(Recipient {
                cinema: c.name.clone(),
                emails: c.emails.clone(),
                screen: s.name.clone(),
                cert_path: s.cert.materialize(tmp_dir)?,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postkit::certificate::{CertOptions, CertType, generate_certificate, generate_chain};

    // generate a real leaf cert in `dir`, returning its path.
    fn leaf_cert(dir: &Path, cn: &str) -> PathBuf {
        assert_eq!(generate_chain("Acme", dir), 0);
        let cert = dir.join(format!("{cn}.pem"));
        let key = dir.join(format!("{cn}.key"));
        let opts = CertOptions {
            cert_type: CertType::Leaf,
            common_name: cn.to_string(),
            organization: "Cinema".into(),
            output_cert: cert.clone(),
            output_key: key,
            issuer_cert: dir.join("root.pem"),
            issuer_key: dir.join("root.key"),
            ..Default::default()
        };
        assert_eq!(generate_certificate(&opts), 0);
        cert
    }

    #[test]
    fn add_search_and_persist_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cert = leaf_cert(dir.path(), "screen1");
        let info = store::cert_info_from_file(&cert).unwrap();

        let db_path = dir.path().join("cinemas.json");
        let mut db = CinemaDb::default();
        db.add_cinema("Odeon", vec!["ops@odeon.test".into()], "notes".into())
            .unwrap();
        db.add_screen("Odeon", "Screen 1", CertSource::Path(cert.clone()))
            .unwrap();
        db.save(&db_path).unwrap();

        let loaded = CinemaDb::load(&db_path).unwrap();
        assert_eq!(loaded.cinemas.len(), 1);
        assert_eq!(loaded.cinemas[0].screens[0].cert_serial, info.serial);

        // dom#2707: search by cert serial finds the screen
        let by_serial = loaded.search(&info.serial);
        assert_eq!(
            by_serial,
            vec![("Odeon".to_string(), "Screen 1".to_string())]
        );
        // search by cinema name
        assert_eq!(loaded.search("odeon").len(), 1);
        // miss
        assert!(loaded.search("nonexistent").is_empty());
    }

    #[test]
    fn duplicate_and_missing_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let cert = leaf_cert(dir.path(), "s");
        let mut db = CinemaDb::default();
        db.add_cinema("A", vec![], String::new()).unwrap();
        assert!(db.add_cinema("A", vec![], String::new()).is_err());
        db.add_screen("A", "S1", CertSource::Path(cert.clone()))
            .unwrap();
        assert!(
            db.add_screen("A", "S1", CertSource::Path(cert.clone()))
                .is_err()
        );
        assert!(
            db.add_screen("Missing", "S1", CertSource::Path(cert))
                .is_err()
        );
        assert!(db.remove_cinema("Missing").is_err());
    }

    #[test]
    fn bad_cert_rejected_on_add() {
        let mut db = CinemaDb::default();
        db.add_cinema("A", vec![], String::new()).unwrap();
        let r = db.add_screen("A", "S1", CertSource::Inline("not a cert".into()));
        assert!(r.is_err());
    }

    #[test]
    fn import_flm_picks_real_leaf_cert() {
        let dir = tempfile::tempdir().unwrap();
        let cert = leaf_cert(dir.path(), "dev");
        let pem = std::fs::read_to_string(&cert).unwrap();
        // strip the PEM armor to the base64 DER the FLM carries
        let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let flm = format!(
            r#"<?xml version="1.0"?>
<flm:FacilityListMessage xmlns:flm="http://www.smpte-ra.org/schemas/430-7/20XX/FLM" xmlns:ds="http://www.w3.org/2000/09/xmldsig#">
  <flm:FacilityInfo><flm:FacilityName>Real Cinema</flm:FacilityName>
    <flm:ContactList><flm:Contact><flm:Email>a@real.test</flm:Email></flm:Contact></flm:ContactList>
  </flm:FacilityInfo>
  <flm:AuditoriumList><flm:Auditorium><flm:AuditoriumNumberOrName>1</flm:AuditoriumNumberOrName>
    <flm:SuiteList><flm:Suite><flm:Device><flm:DeviceSerial>1</flm:DeviceSerial>
      <flm:KeyInfoList><ds:KeyInfo><ds:X509Data><ds:X509Certificate>{b64}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></flm:KeyInfoList>
    </flm:Device></flm:Suite></flm:SuiteList>
  </flm:Auditorium></flm:AuditoriumList>
</flm:FacilityListMessage>"#
        );
        let flm_path = dir.path().join("f.xml");
        std::fs::write(&flm_path, flm).unwrap();

        let mut db = CinemaDb::default();
        let summary = db.import_flm(&flm_path).unwrap();
        assert!(summary.contains("Real Cinema"));
        let c = db.find("Real Cinema").unwrap();
        assert_eq!(c.emails, vec!["a@real.test"]);
        assert_eq!(c.screens.len(), 1);
        let info = store::cert_info_from_file(&cert).unwrap();
        // the leaf's cached serial matches the source cert, and it is stored inline
        assert_eq!(c.screens[0].cert_serial, info.serial);
        assert!(matches!(c.screens[0].cert, CertSource::Inline(_)));
    }

    #[test]
    fn resolve_cinema_and_screen() {
        let dir = tempfile::tempdir().unwrap();
        let cert = leaf_cert(dir.path(), "s");
        let pem = std::fs::read_to_string(&cert).unwrap();
        let mut db = CinemaDb::default();
        db.add_cinema("A", vec!["a@a.test".into()], String::new())
            .unwrap();
        db.add_screen("A", "S1", CertSource::Path(cert.clone()))
            .unwrap();
        db.add_screen("A", "S2", CertSource::Inline(pem)).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let recips = db.resolve(&["A".into()], &[], tmp.path()).unwrap();
        assert_eq!(recips.len(), 2);
        assert!(recips[0].cert_path.exists());
        // inline screen was materialized into tmp_dir
        assert!(recips[1].cert_path.exists());

        let one = db.resolve(&[], &["A/S1".into()], tmp.path()).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].screen, "S1");
        assert!(db.resolve(&[], &["A/nope".into()], tmp.path()).is_err());
    }
}
