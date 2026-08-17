//! named kdm validity templates (dom#2424). a template is a relative window:
//! an optional start offset from now plus a duration, resolved in a chosen utc
//! offset. e.g. "preshow" = starts now, lasts 1 week; "movie" = 6 months.
//! expansion yields absolute ISO timestamps that flow through kdm::generate_kdm
//! unchanged, keeping the same utc-offset behaviour as the kdm module.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Template {
    pub name: String,
    /// offset from now to the start, e.g. "0 days", "2 days". empty = now.
    #[serde(default)]
    pub start_offset: String,
    /// window length, e.g. "1 week", "180 days", "12 hours".
    pub duration: String,
    /// utc offset for the emitted timestamps, e.g. "+00:00", "+02:00". empty = utc.
    #[serde(default)]
    pub tz_offset: String,
}

impl Template {
    /// resolve to (valid_from, valid_to) absolute ISO 8601 strings in the
    /// template's utc offset.
    pub fn expand(&self) -> Result<(String, String), String> {
        let offset = parse_offset(&self.tz_offset)?;
        let now = chrono::Utc::now().with_timezone(&offset);
        let start_delta = if self.start_offset.trim().is_empty() {
            chrono::Duration::zero()
        } else {
            crate::kdm::parse_duration(&self.start_offset)?
        };
        let dur = crate::kdm::parse_duration(&self.duration)?;
        let from = now + start_delta;
        let to = from + dur;
        let fmt = "%Y-%m-%dT%H:%M:%S%:z";
        Ok((from.format(fmt).to_string(), to.format(fmt).to_string()))
    }
}

fn parse_offset(s: &str) -> Result<chrono::FixedOffset, String> {
    if s.trim().is_empty() {
        return Ok(chrono::FixedOffset::east_opt(0).unwrap());
    }
    // accept "+02:00" / "-05:00"
    let t = format!("2000-01-01T00:00:00{}", s.trim());
    chrono::DateTime::parse_from_rfc3339(&t)
        .map(|dt| *dt.offset())
        .map_err(|_| format!("invalid tz offset '{s}' (use +HH:MM)"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TemplateStore {
    #[serde(default)]
    pub templates: Vec<Template>,
}

impl TemplateStore {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|e| format!("cannot parse templates {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("cannot read templates {}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        postkit::fs::write_atomic(path, &json)
    }

    pub fn get(&self, name: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.name == name)
    }

    pub fn add(&mut self, t: Template) -> Result<(), String> {
        if self.get(&t.name).is_some() {
            return Err(format!("template '{}' already exists", t.name));
        }
        self.templates.push(t);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        let before = self.templates.len();
        self.templates.retain(|t| t.name != name);
        if self.templates.len() == before {
            return Err(format!("template '{name}' not found"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_window_length_matches_duration() {
        let t = Template {
            name: "preshow".into(),
            start_offset: String::new(),
            duration: "1 week".into(),
            tz_offset: "+02:00".into(),
        };
        let (from, to) = t.expand().unwrap();
        assert!(
            from.ends_with("+02:00"),
            "start carries the tz offset: {from}"
        );
        let f = chrono::DateTime::parse_from_rfc3339(&from).unwrap();
        let tt = chrono::DateTime::parse_from_rfc3339(&to).unwrap();
        assert_eq!(tt - f, chrono::Duration::weeks(1));
    }

    #[test]
    fn start_offset_delays_the_window() {
        let t = Template {
            name: "late".into(),
            start_offset: "2 days".into(),
            duration: "30 days".into(),
            tz_offset: String::new(),
        };
        let (from, _) = t.expand().unwrap();
        let f = chrono::DateTime::parse_from_rfc3339(&from).unwrap();
        let now = chrono::Utc::now();
        let delta = f.with_timezone(&chrono::Utc) - now;
        // ~2 days from now (allow slack for test runtime)
        assert!(delta > chrono::Duration::days(1) && delta < chrono::Duration::days(3));
    }

    #[test]
    fn store_roundtrip_and_dupes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.json");
        let mut s = TemplateStore::default();
        s.add(Template {
            name: "movie".into(),
            start_offset: String::new(),
            duration: "180 days".into(),
            tz_offset: String::new(),
        })
        .unwrap();
        assert!(
            s.add(Template {
                name: "movie".into(),
                start_offset: String::new(),
                duration: "1 day".into(),
                tz_offset: String::new(),
            })
            .is_err()
        );
        s.save(&p).unwrap();
        let loaded = TemplateStore::load(&p).unwrap();
        assert_eq!(loaded.get("movie").unwrap().duration, "180 days");
    }

    #[test]
    fn bad_duration_errors() {
        let t = Template {
            name: "x".into(),
            start_offset: String::new(),
            duration: "banana".into(),
            tz_offset: String::new(),
        };
        assert!(t.expand().is_err());
    }
}
