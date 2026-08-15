use serde::{Deserialize, Serialize};

/// DCP marker types per SMPTE 429-10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Marker {
    /// First Frame of Composition
    Ffoc,
    /// Last Frame of Composition
    Lfoc,
    /// First Frame of Title Credits
    Fftc,
    /// Last Frame of Title Credits
    Lftc,
    /// First Frame of Intermission
    Ffoi,
    /// Last Frame of Intermission
    Lfoi,
    /// First Frame of End Credits
    Ffec,
    /// Last Frame of End Credits
    Lfec,
    /// First Frame of Moving Credits
    Ffmc,
    /// Last Frame of Moving Credits
    Lfmc,
}

impl Marker {
    pub fn label(&self) -> &'static str {
        match self {
            Marker::Ffoc => "FFOC",
            Marker::Lfoc => "LFOC",
            Marker::Fftc => "FFTC",
            Marker::Lftc => "LFTC",
            Marker::Ffoi => "FFOI",
            Marker::Lfoi => "LFOI",
            Marker::Ffec => "FFEC",
            Marker::Lfec => "LFEC",
            Marker::Ffmc => "FFMC",
            Marker::Lfmc => "LFMC",
        }
    }

    /// Parse a marker label (case-insensitive) against the defined ST 429-10 set.
    pub fn from_label(label: &str) -> Option<Marker> {
        Some(match label.to_ascii_uppercase().as_str() {
            "FFOC" => Marker::Ffoc,
            "LFOC" => Marker::Lfoc,
            "FFTC" => Marker::Fftc,
            "LFTC" => Marker::Lftc,
            "FFOI" => Marker::Ffoi,
            "LFOI" => Marker::Lfoi,
            "FFEC" => Marker::Ffec,
            "LFEC" => Marker::Lfec,
            "FFMC" => Marker::Ffmc,
            "LFMC" => Marker::Lfmc,
            _ => return None,
        })
    }
}

/// The ten defined marker labels, for error messages and help text.
pub const MARKER_LABELS: [&str; 10] = [
    "FFOC", "LFOC", "FFTC", "LFTC", "FFOI", "LFOI", "FFEC", "LFEC", "FFMC", "LFMC",
];

/// Parse a frame offset from either a plain frame number or an SMPTE
/// `HH:MM:SS:FF` timecode (needs fps). Frame-accurate; no drop-frame handling.
pub fn parse_frame_offset(value: &str, fps: u32) -> Result<u64, String> {
    let v = value.trim();
    if v.contains(':') {
        let fps = if fps == 0 { 24 } else { fps } as u64;
        let parts: Vec<&str> = v.split(':').collect();
        if parts.len() != 4 {
            return Err(format!(
                "invalid timecode '{v}': use HH:MM:SS:FF or a plain frame number"
            ));
        }
        let n = |s: &str| -> Result<u64, String> {
            s.parse::<u64>()
                .map_err(|_| format!("invalid timecode '{v}': non-numeric field"))
        };
        let (h, m, s, f) = (n(parts[0])?, n(parts[1])?, n(parts[2])?, n(parts[3])?);
        if f >= fps {
            return Err(format!("timecode '{v}': frame field must be < fps ({fps})"));
        }
        Ok(((h * 3600 + m * 60 + s) * fps) + f)
    } else {
        v.parse::<u64>()
            .map_err(|_| format!("invalid frame '{v}': use a number or HH:MM:SS:FF"))
    }
}

/// Parse a `LABEL=timecode` argument into a validated MarkerEntry. The label
/// must be one of the ten defined markers and the offset must fall inside the
/// composition (`0..=total_frames`).
pub fn parse_marker_arg(arg: &str, fps: u32, total_frames: u64) -> Result<MarkerEntry, String> {
    let (label, tc) = arg
        .split_once('=')
        .ok_or_else(|| format!("invalid --marker '{arg}': expected LABEL=timecode"))?;
    let marker = Marker::from_label(label).ok_or_else(|| {
        format!(
            "unknown marker label '{label}': use one of {}",
            MARKER_LABELS.join(", ")
        )
    })?;
    let frame = parse_frame_offset(tc, fps)?;
    if total_frames > 0 && frame > total_frames {
        return Err(format!(
            "marker {} at frame {frame} is past the composition length ({total_frames})",
            marker.label()
        ));
    }
    Ok(MarkerEntry::new(marker, frame))
}

/// A marker placed at a specific frame offset within a reel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkerEntry {
    pub marker: Marker,
    pub frame: u64,
}

impl MarkerEntry {
    pub fn new(marker: Marker, frame: u64) -> Self {
        Self { marker, frame }
    }
}

/// Generate default markers for a composition of the given frame count.
/// FFOC is 1 and LFOC is the last frame, matching libdcp's Bv2.1 verifier
/// (INCORRECT_FFOC fires unless FFOC == 1).
pub fn default_markers(total_frames: u64) -> Vec<MarkerEntry> {
    if total_frames == 0 {
        return Vec::new();
    }
    vec![
        MarkerEntry::new(Marker::Ffoc, 1),
        MarkerEntry::new(Marker::Lfoc, total_frames.saturating_sub(1)),
    ]
}

/// The markers a composition of `total_frames` carries: the parsed
/// `LABEL=timecode` specs, or the default FFOC/LFOC pair when none were given.
pub fn markers_for_composition(
    specs: &[String],
    fps: u32,
    total_frames: u64,
) -> Result<Vec<MarkerEntry>, String> {
    if specs.is_empty() {
        return Ok(default_markers(total_frames));
    }
    specs
        .iter()
        .map(|spec| parse_marker_arg(spec, fps, total_frames))
        .collect()
}

/// Generate the XML MarkerList block for a set of markers, every line prefixed
/// with `indent`. The Label carries no scope attribute: ST 429-7 defaults it to
/// the standard-markers scope, and that is the form libdcp writes.
pub fn markers_to_xml(markers: &[MarkerEntry], indent: &str) -> String {
    let mut xml = format!("{indent}<MarkerList>\n");
    for entry in markers {
        xml.push_str(&format!("{indent}  <Marker>\n"));
        xml.push_str(&format!(
            "{indent}    <Label>{}</Label>\n",
            entry.marker.label()
        ));
        xml.push_str(&format!("{indent}    <Offset>{}</Offset>\n", entry.frame));
        xml.push_str(&format!("{indent}  </Marker>\n"));
    }
    xml.push_str(&format!("{indent}</MarkerList>\n"));
    xml
}

/// Read each reel's markers back out of a CPL, in reel order. The read side of
/// [`markers_to_xml`], for the paths that rebuild a package's CPL: without it a
/// rewrite drops the marker track.
///
/// A marker whose label is not one of the ten defined ones, or that carries no
/// offset, is skipped: it cannot be rewritten faithfully. The Label's optional
/// scope attribute is ignored, so CPLs from other tools parse too.
pub fn markers_from_cpl(cpl_xml: &str) -> Vec<Vec<MarkerEntry>> {
    let Ok(doc) = roxmltree::Document::parse(cpl_xml) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|node| node.has_tag_name("Reel"))
        .map(|reel| {
            reel.descendants()
                .filter(|node| node.has_tag_name("Marker"))
                .filter_map(|marker| {
                    let child_text = |name: &str| {
                        marker
                            .children()
                            .find(|c| c.has_tag_name(name))
                            .and_then(|c| c.text())
                            .map(str::trim)
                    };
                    let label = Marker::from_label(child_text("Label")?)?;
                    let frame = child_text("Offset")?.parse().ok()?;
                    Some(MarkerEntry::new(label, frame))
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_survive_a_write_then_read() {
        let written = vec![
            MarkerEntry::new(Marker::Ffoc, 1),
            MarkerEntry::new(Marker::Ffec, 240),
        ];
        let cpl = format!(
            "<CompositionPlaylist><ReelList><Reel><AssetList><MainMarkers>\n{}\
             </MainMarkers></AssetList></Reel></ReelList></CompositionPlaylist>",
            markers_to_xml(&written, "")
        );
        let read = markers_from_cpl(&cpl);
        assert_eq!(read.len(), 1, "one reel");
        assert_eq!(read[0].len(), 2);
        assert_eq!(read[0][0].marker, Marker::Ffoc);
        assert_eq!(read[0][1].marker, Marker::Ffec);
        assert_eq!(read[0][1].frame, 240);
    }

    #[test]
    fn foreign_marker_forms_and_junk_are_handled() {
        // libdcp writes a scope attribute on some CPLs; an unknown label or a
        // marker with no offset cannot be rewritten faithfully, so it is dropped
        let cpl = r#"<CompositionPlaylist><ReelList>
            <Reel><AssetList><MainMarkers><MarkerList>
              <Marker><Label scope="http://www.smpte-ra.org/schemas/429-7/2006/CPL#standard-markers">LFOC</Label><Offset>99</Offset></Marker>
              <Marker><Label>XXXX</Label><Offset>5</Offset></Marker>
              <Marker><Label>FFMC</Label></Marker>
            </MarkerList></MainMarkers></AssetList></Reel>
            <Reel><AssetList><MainPicture><Id>urn:uuid:x</Id></MainPicture></AssetList></Reel>
        </ReelList></CompositionPlaylist>"#;
        let read = markers_from_cpl(cpl);
        assert_eq!(read.len(), 2, "a reel with no markers still holds its slot");
        assert_eq!(read[0].len(), 1);
        assert_eq!(read[0][0].marker, Marker::Lfoc);
        assert_eq!(read[0][0].frame, 99);
        assert!(read[1].is_empty());
    }

    #[test]
    fn marker_xml_is_the_form_validators_read() {
        let xml = markers_to_xml(&default_markers(100), "  ");
        // a bare <Label>: validators (ours included) match the tag with no
        // attribute, and ST 429-7 supplies the standard-markers scope by default
        assert!(xml.contains("  <MarkerList>\n"), "{xml}");
        assert!(xml.contains("    <Marker>\n"), "{xml}");
        assert!(xml.contains("      <Label>FFOC</Label>\n"), "{xml}");
        assert!(xml.contains("      <Offset>1</Offset>\n"), "{xml}");
        assert!(!xml.contains("Scope"), "no scope attribute: {xml}");
    }

    #[test]
    fn given_markers_replace_the_default_pair() {
        let specs = vec!["FFEC=00:00:10:00".to_string()];
        let entries = markers_for_composition(&specs, 24, 1000).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].marker, Marker::Ffec);
        assert_eq!(entries[0].frame, 240);
        // no specs falls back to FFOC/LFOC
        assert_eq!(markers_for_composition(&[], 24, 1000).unwrap().len(), 2);
        // a bad spec is an error, not a silent default
        assert!(markers_for_composition(&["nope".to_string()], 24, 1000).is_err());
    }

    #[test]
    fn ffoc_is_one_lfoc_is_last_frame() {
        let markers = default_markers(100);
        let ffoc = markers.iter().find(|m| m.marker == Marker::Ffoc).unwrap();
        let lfoc = markers.iter().find(|m| m.marker == Marker::Lfoc).unwrap();
        // libdcp INCORRECT_FFOC fires unless FFOC == 1
        assert_eq!(ffoc.frame, 1);
        assert_eq!(lfoc.frame, 99);
    }

    #[test]
    fn no_markers_for_empty_composition() {
        assert!(default_markers(0).is_empty());
    }

    #[test]
    fn parse_frame_offset_number_and_timecode() {
        assert_eq!(parse_frame_offset("120", 24).unwrap(), 120);
        assert_eq!(parse_frame_offset("00:00:01:00", 24).unwrap(), 24);
        assert_eq!(parse_frame_offset("01:00:00:12", 24).unwrap(), 86412);
        assert!(parse_frame_offset("00:00:00:24", 24).is_err()); // frame >= fps
        assert!(parse_frame_offset("1:2:3", 24).is_err()); // wrong field count
        assert!(parse_frame_offset("abc", 24).is_err());
    }

    #[test]
    fn parse_marker_arg_validates_label_and_bounds() {
        let e = parse_marker_arg("FFEC=00:00:10:00", 24, 1000).unwrap();
        assert_eq!(e.marker, Marker::Ffec);
        assert_eq!(e.frame, 240);
        // case-insensitive label, plain frame value
        assert_eq!(parse_marker_arg("lfoi=500", 24, 1000).unwrap().frame, 500);
        // unknown label
        assert!(parse_marker_arg("XXXX=10", 24, 1000).is_err());
        // missing '='
        assert!(parse_marker_arg("FFOC", 24, 1000).is_err());
        // past composition end
        assert!(parse_marker_arg("LFOC=2000", 24, 1000).is_err());
    }
}
