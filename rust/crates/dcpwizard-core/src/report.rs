//! HTML QC report generation.
//!
//! [`postkit::report`] is a generic report renderer (severity/category entries).
//! This builds a DCP-specific HTML report directly from dcpdoctor's QC results
//! ([`crate::qc`]), so it stays local.

use std::path::Path;

/// ISO 21727 ceiling for an advertisement, in dB Leq(m).
const ADVERTISEMENT_LEQ_M_LIMIT_DB: f64 = 82.0;
/// ISO 21727 ceiling for a trailer, in dB Leq(m).
const TRAILER_LEQ_M_LIMIT_DB: f64 = 85.0;
/// Reference playback level a feature is mixed to, in dB Leq(m). Not a ceiling.
const FEATURE_LEQ_M_REFERENCE_DB: f64 = 85.0;

const ADVERTISEMENT_CONTENT_KIND: &str = "advertisement";
const TRAILER_CONTENT_KIND: &str = "trailer";
const FEATURE_CONTENT_KIND: &str = "feature";

/// One sound track file of a package, measured against its composition's kind.
struct SoundLevel {
    composition: String,
    content_kind: String,
    track_file: String,
    /// None when the essence is encrypted or ffmpeg could not decode it, with
    /// the reason in `note`.
    leq_m_db: Option<f64>,
    note: String,
}

/// One picture track file of a package, scanned for black and frozen runs.
struct PictureScan {
    composition: String,
    reel_number: u32,
    track_file: String,
    /// One line per run, empty for a clean track. None when nothing scanned it,
    /// with the reason in `note`.
    runs: Option<Vec<String>>,
    note: String,
}

/// Generate an HTML QC report from dcpdoctor verification results.
///
/// `scan_picture` decodes every picture track to find black and frozen runs,
/// which costs hours for a feature, so a report without it says the picture was
/// not scanned rather than leaving the section out.
pub fn generate_report(dcp_dir: &Path, output_html: &Path, scan_picture: bool) -> i32 {
    let qc = crate::qc::run_qc(dcp_dir);

    let dcp_name = dcp_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown DCP");

    let pass_class = if qc.passed { "pass" } else { "fail" };
    let pass_text = if qc.passed { "PASSED" } else { "FAILED" };

    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("<meta charset=\"UTF-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
    html.push_str(&format!(
        "<title>QC Report — {}</title>\n",
        escape_html(dcp_name)
    ));
    html.push_str("<style>\n");
    html.push_str("body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; margin: 2em; background: #f5f5f5; }\n");
    html.push_str(".container { max-width: 900px; margin: 0 auto; background: white; padding: 2em; border-radius: 8px; box-shadow: 0 2px 8px rgba(0,0,0,0.1); }\n");
    html.push_str("h1 { margin-top: 0; }\n");
    html.push_str(".pass { color: #2e7d32; }\n");
    html.push_str(".fail { color: #c62828; }\n");
    html.push_str(".summary { display: flex; gap: 2em; margin: 1em 0; }\n");
    html.push_str(".summary-item { padding: 1em; border-radius: 4px; background: #f0f0f0; }\n");
    html.push_str(".error { color: #c62828; }\n");
    html.push_str(".warning { color: #f57f17; }\n");
    html.push_str(".info { color: #1565c0; }\n");
    html.push_str("table { width: 100%; border-collapse: collapse; margin-top: 1em; }\n");
    html.push_str(
        "th, td { padding: 0.5em 1em; text-align: left; border-bottom: 1px solid #e0e0e0; }\n",
    );
    html.push_str("th { background: #fafafa; font-weight: 600; }\n");
    html.push_str("footer { margin-top: 2em; color: #888; font-size: 0.85em; }\n");
    html.push_str("</style>\n</head>\n<body>\n<div class=\"container\">\n");

    html.push_str("<h1>QC Report</h1>\n");
    html.push_str(&format!("<h2>{}</h2>\n", escape_html(dcp_name)));
    html.push_str(&format!(
        "<p class=\"{pass_class}\"><strong>Result: {pass_text}</strong></p>\n"
    ));

    html.push_str("<div class=\"summary\">\n");
    html.push_str(&format!(
        "<div class=\"summary-item\"><strong class=\"error\">{}</strong> Errors</div>\n",
        qc.error_count
    ));
    html.push_str(&format!(
        "<div class=\"summary-item\"><strong class=\"warning\">{}</strong> Warnings</div>\n",
        qc.warning_count
    ));
    html.push_str(&format!(
        "<div class=\"summary-item\"><strong class=\"info\">{}</strong> Info</div>\n",
        qc.info_count
    ));
    html.push_str("</div>\n");

    if !qc.results.is_empty() {
        html.push_str("<table>\n<thead><tr><th>Level</th><th>Code</th><th>Message</th></tr></thead>\n<tbody>\n");

        for result in &qc.results {
            let (level_class, level_text) = match result.level {
                crate::qc::QcLevel::Error => ("error", "Error"),
                crate::qc::QcLevel::Warning => ("warning", "Warning"),
                crate::qc::QcLevel::Info => ("info", "Info"),
            };
            html.push_str(&format!(
                "<tr><td class=\"{level_class}\">{level_text}</td><td>{}</td><td>{}</td></tr>\n",
                escape_html(&result.code),
                escape_html(&result.message)
            ));
        }

        html.push_str("</tbody>\n</table>\n");
    }

    html.push_str(&sound_level_section(&collect_sound_levels(dcp_dir)));
    if scan_picture {
        html.push_str(&picture_scan_section(&collect_picture_scans(dcp_dir)));
    } else {
        html.push_str(PICTURE_NOT_SCANNED_SECTION);
    }

    html.push_str("<footer>Generated by DCP Wizard</footer>\n");
    html.push_str("</div>\n</body>\n</html>\n");

    match std::fs::write(output_html, &html) {
        Ok(()) => {
            tracing::info!("Generated QC report: {}", output_html.display());
            0
        }
        Err(e) => {
            tracing::error!("Failed to write report: {e}");
            -1
        }
    }
}

/// The Leq(m) the report measures a content kind against: what to print, and
/// the ceiling to judge by. `None` for the ceiling means there is no maximum.
fn leq_m_limit(content_kind: &str) -> (String, Option<f64>) {
    match content_kind {
        ADVERTISEMENT_CONTENT_KIND => (
            format!("{ADVERTISEMENT_LEQ_M_LIMIT_DB:.0} dB maximum"),
            Some(ADVERTISEMENT_LEQ_M_LIMIT_DB),
        ),
        TRAILER_CONTENT_KIND => (
            format!("{TRAILER_LEQ_M_LIMIT_DB:.0} dB maximum"),
            Some(TRAILER_LEQ_M_LIMIT_DB),
        ),
        FEATURE_CONTENT_KIND => (
            format!("{FEATURE_LEQ_M_REFERENCE_DB:.0} dB reference, no maximum"),
            None,
        ),
        _ => ("no limit for this content kind".to_string(), None),
    }
}

/// Every sound track file the package's CPLs reference, with its Leq(m).
fn collect_sound_levels(dcp_dir: &Path) -> Vec<SoundLevel> {
    let mut levels = Vec::new();
    for cpl in crate::multi_cpl::list_cpls(dcp_dir) {
        let cpl_path = dcp_dir.join(&cpl.file_path);
        let cpl_xml = std::fs::read_to_string(&cpl_path).unwrap_or_default();
        let encrypted = encrypted_sound_asset_ids(&cpl_xml);
        for reel in crate::multi_cpl::get_timeline(&cpl_path) {
            if reel.sound_file.is_empty() {
                continue;
            }
            let track_file = Path::new(&reel.sound_file)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let (leq_m_db, note) = if encrypted.contains(&reel.sound_asset_id) {
                (None, "encrypted essence".to_string())
            } else {
                let measured = postkit::loudness::measure_leq_m(Path::new(&reel.sound_file));
                if measured.success {
                    (Some(measured.leq_m_db), String::new())
                } else {
                    (None, measured.error)
                }
            };
            levels.push(SoundLevel {
                composition: cpl.content_title.clone(),
                content_kind: cpl.content_kind.clone(),
                track_file,
                leq_m_db,
                note,
            });
        }
    }
    levels
}

/// Bare asset ids of the MainSound tracks the CPL declares a KeyId for.
fn encrypted_sound_asset_ids(cpl_xml: &str) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    let mut rest = cpl_xml;
    while let Some(start) = rest.find("<MainSound") {
        let block = &rest[start..];
        let Some(end) = block.find("</MainSound>") else {
            break;
        };
        if block[..end].contains("<KeyId>")
            && let Some(id) = crate::multi_cpl::extract_xml_value(&block[..end], "Id")
        {
            ids.insert(id.replace("urn:uuid:", ""));
        }
        rest = &block[end..];
    }
    ids
}

fn sound_level_section(levels: &[SoundLevel]) -> String {
    if levels.is_empty() {
        return String::new();
    }
    let mut html = String::from("<h2>Sound level</h2>\n");
    html.push_str(
        "<p>Leq(m) per ISO 21727, CCIR 468 weighted, measured from the packaged sound. \
         Advertisements are limited to 82 dB and trailers to 85 dB by cinema advertising and \
         trailer distribution policy; a feature has no maximum, and 85 dB is the reference \
         playback level it is mixed against.</p>\n",
    );
    html.push_str("<table>\n<thead><tr><th>Composition</th><th>Content kind</th><th>Track file</th><th>Leq(m)</th><th>Limit</th><th>Result</th></tr></thead>\n<tbody>\n");
    for level in levels {
        let (limit_text, limit) = leq_m_limit(&level.content_kind);
        let measured = match level.leq_m_db {
            Some(db) => format!("{db:.1} dB"),
            None => format!("not measured ({})", level.note),
        };
        let (result_class, result_text) = match (level.leq_m_db, limit) {
            (Some(db), Some(ceiling)) if db <= ceiling => ("pass", "pass"),
            (Some(_), Some(_)) => ("fail", "fail"),
            _ => ("info", "not applicable"),
        };
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"{result_class}\">{result_text}</td></tr>\n",
            escape_html(&level.composition),
            escape_html(&level.content_kind),
            escape_html(&level.track_file),
            escape_html(&measured),
            escape_html(&limit_text),
        ));
    }
    html.push_str("</tbody>\n</table>\n");
    html
}

/// What the report says about the picture when nothing decoded it, so a reader
/// cannot mistake an unscanned package for a clean one.
const PICTURE_NOT_SCANNED_SECTION: &str = "<h2>Picture</h2>\n\
     <p>Not scanned. Finding black and frozen runs means decoding every frame, \
     which takes hours for a feature, so <code>report --scan-picture</code> asks \
     for it and this report was made without it.</p>\n";

/// Every picture track file the package's CPLs reference, scanned for black and
/// frozen runs. Encrypted essence is listed with the reason instead: ffmpeg
/// cannot decrypt AS-DCP.
fn collect_picture_scans(dcp_dir: &Path) -> Vec<PictureScan> {
    let mut scans = Vec::new();
    for cpl in crate::multi_cpl::list_cpls(dcp_dir) {
        let cpl_path = dcp_dir.join(&cpl.file_path);
        for reel in crate::multi_cpl::get_timeline(&cpl_path) {
            if reel.picture_file.is_empty() {
                continue;
            }
            let picture = Path::new(&reel.picture_file);
            let track_file = picture
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let (runs, note) = match postkit::preview::resolve_picture(picture) {
                Err(e) => (None, e.to_string()),
                Ok(resolved) if resolved.encrypted => (None, "encrypted essence".to_string()),
                Ok(resolved) => match postkit::picture_findings::detect_in_essence(
                    &resolved.mxf,
                    resolved.fps,
                    resolved.frame_count as u64,
                ) {
                    Ok(findings) => (Some(findings.describe(resolved.fps)), String::new()),
                    Err(e) => (None, e),
                },
            };
            scans.push(PictureScan {
                composition: cpl.content_title.clone(),
                reel_number: reel.reel_number,
                track_file,
                runs,
                note,
            });
        }
    }
    scans
}

fn picture_scan_section(scans: &[PictureScan]) -> String {
    if scans.is_empty() {
        return String::new();
    }
    let mut html = String::from("<h2>Picture</h2>\n");
    html.push_str(
        "<p>Black and frozen runs found by decoding the packaged picture, ffmpeg's \
         blackdetect and freezedetect at their own defaults: a run has to last 2 seconds \
         to be reported. Frames are numbered from the first frame of the track file, so a \
         reel the composition enters late still counts from zero. These are advisory. A \
         black head or tail and a held still are legitimate content, and it is a run in \
         the middle of a programme that wants a look.</p>\n",
    );
    html.push_str("<table>\n<thead><tr><th>Composition</th><th>Reel</th><th>Track file</th><th>Findings</th><th>Result</th></tr></thead>\n<tbody>\n");
    for scan in scans {
        let (findings_cell, result_class, result_text) = match &scan.runs {
            None => (
                format!("not scanned ({})", escape_html(&scan.note)),
                "info",
                "not applicable",
            ),
            Some(runs) if runs.is_empty() => ("none".to_string(), "pass", "clean"),
            Some(runs) => (
                runs.iter()
                    .map(|run| escape_html(run))
                    .collect::<Vec<_>>()
                    .join("<br>"),
                "warning",
                "review",
            ),
        };
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"{result_class}\">{result_text}</td></tr>\n",
            escape_html(&scan.composition),
            scan.reel_number,
            escape_html(&scan.track_file),
            findings_cell,
        ));
    }
    html.push_str("</tbody>\n</table>\n");
    html
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(content_kind: &str, leq_m_db: Option<f64>, note: &str) -> SoundLevel {
        SoundLevel {
            composition: "My Film".into(),
            content_kind: content_kind.into(),
            track_file: "sound_1.mxf".into(),
            leq_m_db,
            note: note.into(),
        }
    }

    #[test]
    fn a_trailer_over_its_leq_m_limit_fails_and_names_the_limit() {
        let html = sound_level_section(&[level(TRAILER_CONTENT_KIND, Some(86.4), "")]);
        assert!(html.contains("<h2>Sound level</h2>"));
        assert!(html.contains("86.4 dB"), "{html}");
        assert!(html.contains("85 dB maximum"), "{html}");
        assert!(html.contains(">fail</td>"), "{html}");
        assert!(
            html.contains("ISO 21727"),
            "the source of the measure is stated"
        );
    }

    #[test]
    fn an_advertisement_under_its_limit_passes() {
        let html = sound_level_section(&[level(ADVERTISEMENT_CONTENT_KIND, Some(81.0), "")]);
        assert!(html.contains("82 dB maximum"), "{html}");
        assert!(html.contains(">pass</td>"), "{html}");
    }

    #[test]
    fn a_feature_is_measured_against_a_reference_not_a_limit() {
        let html = sound_level_section(&[level(FEATURE_CONTENT_KIND, Some(90.0), "")]);
        assert!(html.contains("85 dB reference, no maximum"), "{html}");
        assert!(html.contains("not applicable"), "{html}");
    }

    #[test]
    fn a_kind_with_no_limit_says_so() {
        let html = sound_level_section(&[level("test", Some(70.0), "")]);
        assert!(html.contains("no limit for this content kind"), "{html}");
        assert!(html.contains("not applicable"), "{html}");
    }

    #[test]
    fn encrypted_sound_is_skipped_with_the_reason() {
        let html = sound_level_section(&[level(TRAILER_CONTENT_KIND, None, "encrypted essence")]);
        assert!(html.contains("not measured (encrypted essence)"), "{html}");
        assert!(html.contains("not applicable"), "{html}");
    }

    #[test]
    fn a_package_with_no_sound_gets_no_section() {
        assert!(sound_level_section(&[]).is_empty());
    }

    fn scan(runs: Option<Vec<String>>, note: &str) -> PictureScan {
        PictureScan {
            composition: "My Film".into(),
            reel_number: 1,
            track_file: "picture_1.mxf".into(),
            runs,
            note: note.into(),
        }
    }

    #[test]
    fn a_black_run_is_listed_with_its_frames_and_wants_a_look() {
        let runs = vec!["black picture from 00:00:00:00 to 00:00:02:22 (frames 0 to 70)".into()];
        let html = picture_scan_section(&[scan(Some(runs), "")]);
        assert!(html.contains("<h2>Picture</h2>"));
        assert!(html.contains("frames 0 to 70"), "{html}");
        assert!(html.contains(">review</td>"), "{html}");
        assert!(
            html.contains("blackdetect"),
            "the report names what measured it"
        );
    }

    #[test]
    fn a_track_with_no_runs_reads_clean() {
        let html = picture_scan_section(&[scan(Some(Vec::new()), "")]);
        assert!(html.contains("<td>none</td>"), "{html}");
        assert!(html.contains(">clean</td>"), "{html}");
    }

    #[test]
    fn encrypted_picture_is_skipped_with_the_reason() {
        let html = picture_scan_section(&[scan(None, "encrypted essence")]);
        assert!(html.contains("not scanned (encrypted essence)"), "{html}");
        assert!(html.contains("not applicable"), "{html}");
    }

    #[test]
    fn a_package_with_no_picture_gets_no_section() {
        assert!(picture_scan_section(&[]).is_empty());
    }

    #[test]
    fn a_report_that_did_not_scan_says_so_and_names_the_flag() {
        assert!(PICTURE_NOT_SCANNED_SECTION.contains("<h2>Picture</h2>"));
        assert!(PICTURE_NOT_SCANNED_SECTION.contains("--scan-picture"));
    }

    #[test]
    fn only_a_main_sound_block_carrying_a_key_id_counts_as_encrypted() {
        let cpl = "\
<Reel><AssetList>\
<MainSound><Id>urn:uuid:aaa</Id><KeyId>urn:uuid:kkk</KeyId></MainSound>\
</AssetList></Reel>\
<Reel><AssetList>\
<MainSound><Id>urn:uuid:bbb</Id></MainSound>\
</AssetList></Reel>";
        let ids = encrypted_sound_asset_ids(cpl);
        assert!(ids.contains("aaa"));
        assert!(!ids.contains("bbb"));
    }
}
