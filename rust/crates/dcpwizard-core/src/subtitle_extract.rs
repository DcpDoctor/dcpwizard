//! Extract timed text from a DCP (or a bare subtitle asset) back to SRT or plain
//! text. Handles SMPTE ST 428-7 DCST (in an MXF or loose XML) and Interop
//! DCSubtitle XML. For a DCP directory, subtitle assets are resolved via the CPL
//! and reels are concatenated with their timeline offsets applied.

use std::path::Path;

/// A subtitle cue with millisecond timing on the composition timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Cue text, multiple lines joined with '\n'.
    pub text: String,
}

/// Extract subtitles from `input` (a DCP directory or a subtitle asset) and write
/// `output`. `.srt` preserves timing; `.txt` writes text only. The extension of
/// `output` picks the format.
pub fn extract(input: &Path, output: &Path) -> Result<(), String> {
    let cues = extract_cues(input)?;
    if cues.is_empty() {
        return Err(format!("no subtitles found in {}", input.display()));
    }
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let body = match ext.as_str() {
        "txt" => to_txt(&cues),
        "srt" => to_srt(&cues),
        other => {
            return Err(format!(
                "unsupported output extension '.{other}', use .srt or .txt"
            ));
        }
    };
    std::fs::write(output, body).map_err(|e| format!("failed to write {}: {e}", output.display()))
}

/// Resolve `input` to a flat list of cues on the composition timeline.
pub fn extract_cues(input: &Path) -> Result<Vec<Cue>, String> {
    if input.is_dir() {
        extract_from_dcp(input)
    } else {
        read_subtitle_asset(input)
    }
}

/// Resolve every reel's subtitle asset via the CPL and concatenate them with the
/// per-reel timeline offset applied.
fn extract_from_dcp(dcp_dir: &Path) -> Result<Vec<Cue>, String> {
    let dcp = dcpdoctor_core::dcp::open_dcp(dcp_dir).map_err(|notes| {
        notes
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })?;

    let mut out = Vec::new();
    for (_path, cpl) in &dcp.cpls {
        let mut reel_start_ms = 0u64;
        for reel in &cpl.reels {
            let reel_ms =
                frames_to_ms(reel.picture.duration.max(0) as u64, &reel.picture.edit_rate);
            let sub = &reel.subtitle;
            if !sub.id.is_empty()
                && let Some(rel) = dcp.assetmap.assets.iter().find(|a| a.id == sub.id)
            {
                let asset_path = dcp_dir.join(&rel.path);
                let entry_ms = frames_to_ms(sub.entry_point.unwrap_or(0).max(0) as u64, &sub.edit_rate);
                for mut cue in read_subtitle_asset(&asset_path)? {
                    // rebase asset-absolute cue onto reel-local playback, then the
                    // reel's place on the composition timeline
                    let start = cue.start_ms.saturating_sub(entry_ms);
                    let end = cue.end_ms.saturating_sub(entry_ms);
                    cue.start_ms = reel_start_ms + start;
                    cue.end_ms = reel_start_ms + end;
                    out.push(cue);
                }
            }
            reel_start_ms += reel_ms;
        }
    }
    out.sort_by_key(|c| c.start_ms);
    Ok(out)
}

/// Read cues from a subtitle asset: an MXF-wrapped ST 428-7 resource, or a loose
/// SMPTE/Interop XML file.
pub fn read_subtitle_asset(path: &Path) -> Result<Vec<Cue>, String> {
    let is_mxf = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mxf"));
    let xml = if is_mxf {
        read_timed_text_mxf(path)?
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?
    };
    Ok(parse_subtitle_xml(&xml))
}

/// Pull the timed-text XML resource out of an MXF via asdcplib.
fn read_timed_text_mxf(path: &Path) -> Result<String, String> {
    let name = path.to_str().ok_or("non-UTF-8 path")?;
    let mut reader = asdcplib::timed_text::MxfReader::new();
    reader.open_read(name).map_err(|e| e.to_string())?;

    // grow the buffer to the size the reader reports it needs
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        match reader.read_timed_text_resource(&mut buf, None, None) {
            Ok(size) => {
                buf.truncate(size);
                break;
            }
            Err(asdcplib::Error::BufferTooSmall { needed, .. }) => buf.resize(needed, 0),
            Err(e) => return Err(e.to_string()),
        }
    }
    String::from_utf8(buf).map_err(|e| format!("subtitle resource is not UTF-8: {e}"))
}

/// Parse SMPTE ST 428-7 DCST or Interop DCSubtitle XML into cues.
pub fn parse_subtitle_xml(xml: &str) -> Vec<Cue> {
    let interop = xml.contains("DCSubtitle");
    // SMPTE times are HH:MM:SS:EEE editable units at TimeCodeRate per second;
    // Interop times are HH:MM:SS.mmm or HH:MM:SS:TTT ticks at 250/s.
    let tcr = read_u32_tag(xml, "TimeCodeRate")
        .or_else(|| edit_rate_num(xml))
        .unwrap_or(24);

    let mut cues = Vec::new();
    let mut from = 0;
    while let Some(rel) = xml[from..].find("TimeIn=\"") {
        let attr_pos = from + rel;
        let open_start = xml[..attr_pos].rfind('<').unwrap_or(attr_pos);
        let Some(gt) = xml[attr_pos..].find('>') else {
            break;
        };
        let open_end = attr_pos + gt;
        let open_tag = &xml[open_start..=open_end];

        let time_in = attr(open_tag, "TimeIn");
        let time_out = attr(open_tag, "TimeOut");

        // content runs to the element's own close tag, e.g. </dcst:Subtitle>
        let content_start = open_end + 1;
        let close = xml[content_start..]
            .find("Subtitle>")
            .map(|p| content_start + p);
        let content_end = close.and_then(|c| xml[..c].rfind('<')).unwrap_or(xml.len());
        let content = &xml[content_start..content_end];

        if let (Some(ti), Some(to)) = (time_in, time_out) {
            let start = parse_time(&ti, interop, tcr);
            let end = parse_time(&to, interop, tcr);
            if let (Some(start_ms), Some(end_ms)) = (start, end) {
                let text = text_lines(content).join("\n");
                if !text.is_empty() {
                    cues.push(Cue {
                        start_ms,
                        end_ms,
                        text,
                    });
                }
            }
        }
        from = close.map(|c| c + "Subtitle>".len()).unwrap_or(xml.len());
    }
    cues
}

/// Value of the `name="..."` attribute in an opening tag.
fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// Inner text of each `Text` element in a Subtitle's content, one per line.
fn text_lines(content: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut from = 0;
    while let Some(rel) = content[from..].find("Text") {
        let pos = from + rel;
        // must be an opening tag: the run is preceded by '<' or '<prefix:'
        let Some(gt) = content[pos..].find('>') else {
            break;
        };
        let inner_start = pos + gt + 1;
        let Some(close_rel) = content[inner_start..].find("Text>") else {
            break;
        };
        let close_lt = content[inner_start..inner_start + close_rel]
            .rfind('<')
            .map(|p| inner_start + p)
            .unwrap_or(inner_start + close_rel);
        let raw = &content[inner_start..close_lt];
        let line = unescape_xml(&strip_tags(raw));
        let line = line.trim();
        if !line.is_empty() {
            lines.push(line.to_string());
        }
        from = inner_start + close_rel + "Text>".len();
    }
    lines
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Parse a subtitle time expression to milliseconds.
fn parse_time(s: &str, interop: bool, tcr: u32) -> Option<u64> {
    let s = s.trim();
    // Interop dot form: HH:MM:SS.mmm
    if let Some((hms, frac)) = s.split_once('.') {
        let (h, m, sec) = parse_hms(hms)?;
        return Some(hms_ms(h, m, sec) + frac_ms(frac)?);
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 4 {
        return None;
    }
    let h: u64 = parts[0].parse().ok()?;
    let m: u64 = parts[1].parse().ok()?;
    let sec: u64 = parts[2].parse().ok()?;
    let sub: u64 = parts[3].parse().ok()?;
    // Interop colon form is ticks at 250/s; SMPTE is editable units at TimeCodeRate/s
    let per_second = if interop { 250 } else { tcr.max(1) as u64 };
    Some(hms_ms(h, m, sec) + sub * 1000 / per_second)
}

fn parse_hms(hms: &str) -> Option<(u64, u64, u64)> {
    let p: Vec<&str> = hms.split(':').collect();
    if p.len() != 3 {
        return None;
    }
    Some((p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?))
}

fn hms_ms(h: u64, m: u64, s: u64) -> u64 {
    ((h * 60 + m) * 60 + s) * 1000
}

/// Fractional seconds string to milliseconds (pad/truncate to 3 digits).
fn frac_ms(frac: &str) -> Option<u64> {
    let digits: String = frac.chars().take(3).collect();
    let scale = 10u64.pow(3 - digits.len() as u32);
    digits.parse::<u64>().ok().map(|v| v * scale)
}

/// Frame count to milliseconds at an EditRate string like "24 1" or "24".
fn frames_to_ms(frames: u64, edit_rate: &str) -> u64 {
    let mut it = edit_rate.split_whitespace();
    let num: u64 = it.next().and_then(|n| n.parse().ok()).unwrap_or(24);
    let den: u64 = it.next().and_then(|d| d.parse().ok()).unwrap_or(1);
    if num == 0 {
        return 0;
    }
    frames * 1000 * den / num
}

fn read_u32_tag(xml: &str, local: &str) -> Option<u32> {
    let open = format!("{local}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find("</")? + start;
    xml[start..end].trim().parse().ok()
}

/// EditRate numerator from a `<...EditRate>N D</...EditRate>` element.
fn edit_rate_num(xml: &str) -> Option<u32> {
    let open = "EditRate>";
    let start = xml.find(open)? + open.len();
    let end = xml[start..].find("</")? + start;
    xml[start..end].split_whitespace().next()?.parse().ok()
}

fn to_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_time(cue.start_ms),
            srt_time(cue.end_ms),
            cue.text,
        ));
    }
    out
}

fn to_txt(cues: &[Cue]) -> String {
    cues.iter()
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
        + "\n"
}

fn srt_time(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let millis = ms % 1000;
    format!("{h:02}:{m:02}:{s:02},{millis:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMPTE_XML: &str = r#"<?xml version="1.0"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:EditRate>24 1</dcst:EditRate>
  <dcst:TimeCodeRate>24</dcst:TimeCodeRate>
  <dcst:SubtitleList>
    <dcst:Font ID="f1">
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:01:00" TimeOut="00:00:02:00">
        <dcst:Text Valign="bottom">Hello &amp; welcome</dcst:Text>
      </dcst:Subtitle>
      <dcst:Subtitle SpotNumber="2" TimeIn="00:00:03:12" TimeOut="00:00:04:00">
        <dcst:Text>line one</dcst:Text>
        <dcst:Text>line two</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#;

    #[test]
    fn parses_smpte_dcst() {
        let cues = parse_subtitle_xml(SMPTE_XML);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 2000);
        assert_eq!(cues[0].text, "Hello & welcome");
        // frame 12 at 24fps == 500ms
        assert_eq!(cues[1].start_ms, 3500);
        assert_eq!(cues[1].text, "line one\nline two");
    }

    #[test]
    fn parses_interop_dcsubtitle() {
        let xml = r#"<DCSubtitle Version="1.0">
  <Font>
    <Subtitle SpotNumber="1" TimeIn="00:00:01.000" TimeOut="00:00:02.500">
      <Text>Interop cue</Text>
    </Subtitle>
  </Font>
</DCSubtitle>"#;
        let cues = parse_subtitle_xml(xml);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 2500);
        assert_eq!(cues[0].text, "Interop cue");
    }

    #[test]
    fn srt_round_trip_through_dcst() {
        // whole-second timings are frame-exact at any integer fps, so ms survive
        // SRT -> DCST -> extract unchanged
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:05,000\ntwo\nlines\n";
        let dir = tempfile::tempdir().unwrap();
        let srt_path = dir.path().join("in.srt");
        let xml_path = dir.path().join("out.xml");
        std::fs::write(&srt_path, srt).unwrap();
        crate::subtitle::convert_srt_to_dcp_xml(&srt_path, &xml_path, "en", 24, 0.0).unwrap();

        let cues = read_subtitle_asset(&xml_path).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 2000);
        assert_eq!(cues[0].text, "Hello");
        assert_eq!(cues[1].start_ms, 3000);
        assert_eq!(cues[1].end_ms, 5000);
        assert_eq!(cues[1].text, "two\nlines");
    }

    #[test]
    fn writes_srt_and_txt() {
        let cues = vec![Cue {
            start_ms: 1000,
            end_ms: 2000,
            text: "a\nb".to_string(),
        }];
        assert_eq!(to_srt(&cues), "1\n00:00:01,000 --> 00:00:02,000\na\nb\n\n");
        assert_eq!(to_txt(&cues), "a\nb\n");
    }
}
