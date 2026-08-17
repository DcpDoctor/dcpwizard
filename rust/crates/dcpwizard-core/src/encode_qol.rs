// Encode quality-of-life helpers (W5): scheduled start (dom#2359), runtime ETA
// (dom#502), shutdown on completion (dom#1394), and persisted resume state
// (dom#344). Kept together as one feature group; the actual encode loop lives in
// postkit::grok_encoder.

use chrono::{DateTime, Duration, Local, NaiveTime};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── scheduled start (dom#2359) ──────────────────────────────────────────────

/// Parse a `--start-at` spec into an absolute local time, relative to `now`.
///
/// Accepts an RFC 3339 timestamp (`2026-07-23T22:00:00Z` or with offset), a
/// `HH:MM`/`HH:MM:SS` wall-clock time today (rolled to tomorrow if already
/// past), or a `+<n><unit>` offset (`+30m`, `+2h`, `+45s`).
pub fn parse_start_at(spec: &str, now: DateTime<Local>) -> Result<DateTime<Local>, String> {
    let spec = spec.trim();
    if let Some(rel) = spec.strip_prefix('+') {
        let (num, unit) = rel.split_at(
            rel.find(|c: char| !c.is_ascii_digit())
                .ok_or_else(|| format!("start-at offset '{spec}' needs a unit (s/m/h)"))?,
        );
        let n: i64 = num
            .parse()
            .map_err(|_| format!("start-at offset '{spec}' has a bad number"))?;
        let d = match unit {
            "s" | "sec" => Duration::seconds(n),
            "m" | "min" => Duration::minutes(n),
            "h" | "hr" => Duration::hours(n),
            other => return Err(format!("start-at unit '{other}' must be s, m or h")),
        };
        return Ok(now + d);
    }
    if spec.contains('T') || spec.contains('Z') {
        return DateTime::parse_from_rfc3339(spec)
            .map(|dt| dt.with_timezone(&Local))
            .map_err(|e| format!("start-at '{spec}' is not an RFC 3339 timestamp: {e}"));
    }
    // HH:MM or HH:MM:SS wall-clock today, rolled forward if already past.
    let t = NaiveTime::parse_from_str(spec, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(spec, "%H:%M"))
        .map_err(|_| format!("start-at '{spec}' is not HH:MM, an RFC 3339 time, or +offset"))?;
    let today = now.date_naive().and_time(t);
    let local = today
        .and_local_timezone(Local)
        .single()
        .ok_or_else(|| format!("start-at '{spec}' is ambiguous in the local timezone"))?;
    Ok(if local <= now {
        local + Duration::days(1)
    } else {
        local
    })
}

/// Current local time (wrapper so CLI callers need no chrono dependency).
pub fn now_local() -> DateTime<Local> {
    Local::now()
}

/// Block until `target` (local time), sleeping in short chunks. Returns at once
/// if `target` is already past.
pub fn wait_until(target: DateTime<Local>) {
    loop {
        let now = Local::now();
        if now >= target {
            return;
        }
        let remaining = (target - now)
            .to_std()
            .unwrap_or(std::time::Duration::ZERO)
            .min(std::time::Duration::from_secs(1));
        std::thread::sleep(remaining);
    }
}

// ── runtime ETA (dom#502) ───────────────────────────────────────────────────

/// Format a duration in seconds as `H:MM:SS` (dropping the hour when zero).
pub fn format_eta(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "--:--".to_string();
    }
    let total = secs.round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// ETA in seconds from average fps: remaining frames / average fps. `None` when
/// it cannot be estimated yet (no throughput or nothing left).
pub fn eta_seconds(frames_done: u64, total_frames: u64, avg_fps: f64) -> Option<f64> {
    if total_frames == 0 || avg_fps <= 0.0 || frames_done >= total_frames {
        return None;
    }
    Some((total_frames - frames_done) as f64 / avg_fps)
}

// ── shutdown on completion (dom#1394) ───────────────────────────────────────

// find an executable named `name` on the given PATH string.
fn find_in_path(name: &str, path_var: &str) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Resolve the platform shutdown binary, failing loud if it is not installed.
/// Never runs anything.
pub fn resolve_shutdown_command() -> Result<PathBuf, String> {
    let name = if cfg!(windows) {
        "shutdown.exe"
    } else {
        "shutdown"
    };
    let path = std::env::var("PATH").unwrap_or_default();
    find_in_path(name, &path)
        // shutdown usually lives in /sbin, often absent from a login PATH.
        .or_else(|| {
            ["/sbin/shutdown", "/usr/sbin/shutdown", "/usr/bin/shutdown"]
                .iter()
                .map(PathBuf::from)
                .find(|p| p.is_file())
        })
        .ok_or_else(|| {
            format!("--shutdown-when-done: '{name}' not found on PATH; cannot power off")
        })
}

/// Run the platform shutdown command now. Resolves it first (fails loud if
/// missing). Not exercised by tests, which never power off the machine.
pub fn run_shutdown() -> Result<(), String> {
    let cmd = resolve_shutdown_command()?;
    let mut c = std::process::Command::new(&cmd);
    if cfg!(windows) {
        c.args(["/s", "/t", "0"]);
    } else {
        c.args(["-h", "now"]);
    }
    c.spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to run {}: {e}", cmd.display()))
}

// ── resume state (dom#344) ──────────────────────────────────────────────────

const STATE_FILE: &str = ".dcpwizard-encode.json";

/// Persisted description of an in-progress encode, so a `--resume` run can
/// verify it is continuing the same job before reusing the on-disk frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncodeState {
    pub source: String,
    pub total_frames: u64,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub bitrate_mbps: u32,
}

impl EncodeState {
    fn path(output_dir: &Path) -> PathBuf {
        output_dir.join(STATE_FILE)
    }

    /// Write the state next to the encode output (atomic).
    pub fn save(&self, output_dir: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        postkit::fs::write_atomic(&Self::path(output_dir), &bytes)
    }

    /// Load a previously saved state, if any.
    pub fn load(output_dir: &Path) -> Option<EncodeState> {
        let bytes = std::fs::read(Self::path(output_dir)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Remove the state file (called once the encode succeeds).
    pub fn clear(output_dir: &Path) {
        let _ = std::fs::remove_file(Self::path(output_dir));
    }

    /// Resuming is valid only when the saved job matches the current one exactly;
    /// a mismatch means the user pointed `--resume` at a different encode.
    pub fn check_resumable(&self, output_dir: &Path) -> Result<bool, String> {
        match EncodeState::load(output_dir) {
            None => Ok(false),
            Some(prev) if &prev == self => Ok(true),
            Some(prev) => Err(format!(
                "--resume: {} holds a different encode (source/params changed); \
                 remove it or run without --resume. saved: {prev:?}",
                Self::path(output_dir).display()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 7, 23, h, m, 0).unwrap()
    }

    #[test]
    fn relative_offsets() {
        let now = at(10, 0);
        assert_eq!(parse_start_at("+30m", now).unwrap(), at(10, 30));
        assert_eq!(parse_start_at("+2h", now).unwrap(), at(12, 0));
        assert_eq!(
            parse_start_at("+45s", now).unwrap(),
            now + Duration::seconds(45)
        );
    }

    #[test]
    fn wall_clock_rolls_to_tomorrow_when_past() {
        let now = at(22, 0);
        // 21:00 already passed today -> tomorrow.
        let t = parse_start_at("21:00", now).unwrap();
        assert_eq!(t, at(21, 0) + Duration::days(1));
        // 23:30 still ahead today.
        assert_eq!(parse_start_at("23:30", now).unwrap(), at(23, 30));
    }

    #[test]
    fn rfc3339_and_bad_specs() {
        assert!(parse_start_at("2026-07-23T22:00:00Z", at(10, 0)).is_ok());
        assert!(parse_start_at("nonsense", at(10, 0)).is_err());
        assert!(parse_start_at("+10x", at(10, 0)).is_err());
    }

    #[test]
    fn eta_formatting_and_estimate() {
        assert_eq!(format_eta(65.0), "01:05");
        assert_eq!(format_eta(3661.0), "1:01:01");
        assert_eq!(format_eta(-1.0), "--:--");
        assert_eq!(eta_seconds(0, 240, 24.0), Some(10.0));
        assert_eq!(eta_seconds(240, 240, 24.0), None);
        assert_eq!(eta_seconds(10, 240, 0.0), None);
    }

    #[test]
    fn find_in_path_resolves_only_present_names() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("myshutdown");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
        let path = dir.path().to_string_lossy().to_string();
        assert_eq!(find_in_path("myshutdown", &path), Some(exe));
        assert_eq!(find_in_path("does-not-exist", &path), None);
    }

    #[test]
    fn resume_state_roundtrip_and_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let s = EncodeState {
            source: "/in.mov".into(),
            total_frames: 240,
            fps: 24,
            width: 2048,
            height: 1080,
            bitrate_mbps: 250,
        };
        // no state yet -> not resumable, no error.
        assert!(!s.check_resumable(d).unwrap());
        s.save(d).unwrap();
        assert!(s.check_resumable(d).unwrap());
        // a different job pointed at the same dir is a loud error.
        let other = EncodeState {
            total_frames: 999,
            ..s.clone()
        };
        assert!(other.check_resumable(d).is_err());
        EncodeState::clear(d);
        assert!(!s.check_resumable(d).unwrap());
    }
}
