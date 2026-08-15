use serde::{Deserialize, Serialize};
use std::path::Path;

/// CPL configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CplConfig {
    pub title: String,
    pub content_kind: String,
    pub rating: String,
    pub reels: Vec<CplReel>,
    pub standard: crate::Standard,
    /// Packaged sound layout for the SMPTE CompositionMetadataAsset (ST 429-16).
    /// None for a silent composition or Interop (no metadata asset is written).
    pub main_sound: Option<MainSound>,
    /// RFC 5646 sign-language tag (ISDCF Doc 13). When set, the CompositionMetadata
    /// carries the SignLanguageVideo ExtensionMetadata with this Language Tag.
    #[serde(default)]
    pub sign_language: Option<String>,
}

/// Sound layout for the SMPTE CompositionMetadataAsset (ST 429-16): the
/// MainSoundConfiguration string plus the sample rate for MainSoundSampleRate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainSound {
    /// e.g. "51/L,R,C,LFE,Ls,Rs" with '-' padding for silent fill channels.
    pub configuration: String,
    /// 48000 or 96000.
    pub sample_rate: u32,
}

/// A single reel in the CPL.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CplReel {
    pub reel_id: String,
    pub picture_id: String,
    pub picture_width: u32,
    pub picture_height: u32,
    pub picture_edit_rate_num: u32,
    pub picture_edit_rate_den: u32,
    pub picture_duration: u64,
    pub picture_entry_point: u64,
    /// KeyId (bare UUID) when the picture essence is encrypted.
    pub picture_key_id: Option<String>,
    pub sound_id: Option<String>,
    pub sound_edit_rate_num: u32,
    pub sound_edit_rate_den: u32,
    pub sound_duration: u64,
    pub sound_entry_point: u64,
    /// KeyId (bare UUID) when the sound essence is encrypted.
    pub sound_key_id: Option<String>,
    /// Bare UUID of the timed-text (subtitle) track, when present.
    pub subtitle_id: Option<String>,
    pub subtitle_edit_rate_num: u32,
    pub subtitle_edit_rate_den: u32,
    pub subtitle_duration: u64,
    pub subtitle_entry_point: u64,
    pub subtitle_language: Option<String>,
    /// Bare UUID of the closed-caption (ST 429-12) timed-text track, when present.
    /// Distinct from the open subtitle: emitted as MainClosedCaption.
    #[serde(default)]
    pub ccap_id: Option<String>,
    #[serde(default)]
    pub ccap_edit_rate_num: u32,
    #[serde(default)]
    pub ccap_edit_rate_den: u32,
    #[serde(default)]
    pub ccap_duration: u64,
    #[serde(default)]
    pub ccap_entry_point: u64,
    #[serde(default)]
    pub ccap_language: Option<String>,
    /// Picture is a stereoscopic (ST 429-10) essence: emit MainStereoscopicPicture
    /// with FrameRate doubled (two frames per edit unit) instead of MainPicture.
    pub stereoscopic: bool,
    /// Auxiliary data (ST 429-18) track, e.g. Dolby Atmos. When present, an
    /// AuxData element is added to the reel's AssetList.
    pub aux_data: Option<AuxData>,
    /// ST 429-7 markers for this reel, emitted as its MainMarkers asset. Bv2.1
    /// expects one in reel 1 carrying at least FFOC/LFOC.
    #[serde(default)]
    pub markers: Vec<crate::markers::MarkerEntry>,
}

/// Auxiliary-data (ST 429-18) track for a CPL reel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuxData {
    pub id: String,
    pub edit_rate_num: u32,
    pub edit_rate_den: u32,
    pub duration: u64,
    pub entry_point: u64,
    pub key_id: Option<String>,
    /// SMPTE data-essence UL (e.g. Dolby Atmos IAB bitstream).
    pub data_type: String,
}

/// Generate a Composition Playlist XML via the shared postkit writer.
pub fn generate_cpl(config: &CplConfig, cpl_uuid: &str, output_file: &Path) -> i32 {
    use postkit::packaging::{self, DcpCpl, DcpCplReel};

    let namespace = match config.standard {
        crate::Standard::Smpte => packaging::ns::CPL_SMPTE,
        crate::Standard::Interop => packaging::ns::CPL_INTEROP,
    };

    let reels = config
        .reels
        .iter()
        .map(|r| DcpCplReel {
            reel_id: r.reel_id.clone(),
            picture_id: r.picture_id.clone(),
            picture_width: r.picture_width,
            picture_height: r.picture_height,
            picture_edit_rate_num: r.picture_edit_rate_num,
            picture_edit_rate_den: r.picture_edit_rate_den,
            picture_duration: r.picture_duration,
            picture_entry_point: r.picture_entry_point,
            picture_key_id: r.picture_key_id.clone(),
            sound_id: r.sound_id.clone(),
            sound_edit_rate_num: r.sound_edit_rate_num,
            sound_edit_rate_den: r.sound_edit_rate_den,
            sound_duration: r.sound_duration,
            sound_entry_point: r.sound_entry_point,
            sound_key_id: r.sound_key_id.clone(),
        })
        .collect();

    let cpl = DcpCpl {
        uuid: cpl_uuid.to_string(),
        namespace: namespace.to_string(),
        title: config.title.clone(),
        content_kind: config.content_kind.clone(),
        issuer: "DCP Wizard".into(),
        creator: "DCP Wizard".into(),
        issue_date: time_now_iso(),
        reels,
    };

    // SMPTE Bv2.1 requires a CompositionMetadataAsset (ST 429-16) in the first
    // reel; it carries MainSoundConfiguration so validators know the real channel
    // layout. Interop has no such asset.
    let metadata_block = if config.standard == crate::Standard::Smpte {
        match (config.main_sound.as_ref(), config.reels.first()) {
            (Some(ms), Some(first)) => Some(composition_metadata_block(config, first, ms)),
            _ => None,
        }
    } else {
        None
    };

    // postkit's DcpCplReel only writes MainPicture/MainSound, so splice the extra
    // per-reel assets (markers, subtitle, aux data, metadata) and rewrite the
    // picture element for stereoscopic reels. Reel order is preserved.
    let mut xml = cpl.to_xml();
    let needs_splice = metadata_block.is_some()
        || config.reels.iter().any(|r| {
            r.subtitle_id.is_some()
                || r.ccap_id.is_some()
                || r.aux_data.is_some()
                || r.stereoscopic
                || !r.markers.is_empty()
        });
    if needs_splice {
        xml = splice_reel_extras(
            &xml,
            &config.reels,
            config.standard,
            metadata_block.as_deref(),
        );
    }

    match std::fs::write(output_file, xml) {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("Failed to write CPL: {e}");
            -1
        }
    }
}

/// ST 429-10 (2008) stereoscopic picture namespace.
const NS_STEREO_429_10: &str =
    "http://www.smpte-ra.org/schemas/429-10/2008/Main-Stereo-Picture-CPL";
/// Dolby auxiliary-data (ST 429-18) namespace used by real Atmos DCPs.
const NS_AUX_DATA: &str = "http://www.dolby.com/schemas/2012/AD";

/// Rewrite each reel's picture element and add the extra AssetList entries
/// (markers, subtitle, aux data, first-reel metadata) that postkit's writer does
/// not emit. Walks the reel segments in order between `</AssetList>` markers.
fn splice_reel_extras(
    xml: &str,
    reels: &[CplReel],
    standard: crate::Standard,
    metadata_first_reel: Option<&str>,
) -> String {
    const MARKER: &str = "      </AssetList>\n";
    let mut out = String::with_capacity(xml.len() + 512);
    let mut idx = 0;
    for (i, reel) in reels.iter().enumerate() {
        let Some(pos) = xml[idx..].find(MARKER) else {
            break;
        };
        let end = idx + pos;
        let mut segment = xml[idx..end].to_string();
        if reel.stereoscopic {
            segment = rewrite_stereoscopic(&segment, reel, standard);
        }
        if !reel.markers.is_empty() {
            segment = insert_main_markers(&segment, reel);
        }
        out.push_str(&segment);
        if let Some(ref sid) = reel.subtitle_id {
            out.push_str(&main_subtitle_block(reel, sid));
        }
        if let Some(ref cid) = reel.ccap_id {
            out.push_str(&main_closed_caption_block(reel, cid));
        }
        if let Some(ref aux) = reel.aux_data {
            out.push_str(&aux_data_block(aux));
        }
        if i == 0
            && let Some(block) = metadata_first_reel
        {
            out.push_str(block);
        }
        out.push_str(MARKER);
        idx = end + MARKER.len();
    }
    out.push_str(&xml[idx..]);
    out
}

/// ST 429-16 CompositionMetadataAsset namespace.
const NS_CPL_META: &str = "http://www.smpte-ra.org/schemas/429-16/2014/CPL-Metadata";

/// Build the ST 429-16 MainSoundConfiguration from the packaged sound MXF channel
/// count plus optional HI/VI accessibility channels. Channels past the labeled
/// soundfield are silent fill, written as '-'. Returns None for a layout with no
/// canonical DCP label. `channel_count` is the wrapped MXF channel count (a 5.1
/// source is padded to 16, so its config carries ten '-' placeholders).
pub fn main_sound_configuration(
    channel_count: u32,
    hi_channel: Option<u32>,
    vi_channel: Option<u32>,
) -> Option<String> {
    let extra = hi_channel.is_some() as u32 + vi_channel.is_some() as u32;
    let main_count = channel_count.saturating_sub(extra);
    // canonical DCP channel order per ISDCF / SMPTE RDD 52
    let (group, labels): (&str, &[&str]) = match main_count {
        2 => ("20", &["L", "R"]),
        6 | 16 => ("51", &["L", "R", "C", "LFE", "Ls", "Rs"]),
        8 => ("71", &["L", "R", "C", "LFE", "Lss", "Rss", "Lrs", "Rrs"]),
        _ => return None,
    };
    let mut slots = vec!["-"; channel_count as usize];
    for (i, l) in labels.iter().enumerate() {
        if let Some(slot) = slots.get_mut(i) {
            *slot = l;
        }
    }
    if let Some(i) = hi_channel
        && let Some(slot) = slots.get_mut(i as usize)
    {
        *slot = "HI";
    }
    if let Some(i) = vi_channel
        && let Some(slot) = slots.get_mut(i as usize)
    {
        *slot = "VIN";
    }
    Some(format!("{group}/{}", slots.join(",")))
}

/// SMPTE Bv2.1 CompositionMetadataAsset for the first reel. Stored and active
/// picture areas are the full container; sound layout and sample rate come from
/// the packaged audio. Base-type children (Id/EditRate/IntrinsicDuration) sit in
/// the CPL's default 429-7 namespace, the rest in the 429-16 meta namespace.
fn composition_metadata_block(config: &CplConfig, reel: &CplReel, sound: &MainSound) -> String {
    use postkit::packaging::escape_xml;
    let id = uuid::Uuid::new_v4();
    let mut b = String::new();
    b.push_str(&format!(
        "        <meta:CompositionMetadataAsset xmlns:meta=\"{NS_CPL_META}\">\n"
    ));
    b.push_str(&format!("          <Id>urn:uuid:{id}</Id>\n"));
    b.push_str(&format!(
        "          <EditRate>{} {}</EditRate>\n",
        reel.picture_edit_rate_num, reel.picture_edit_rate_den
    ));
    b.push_str(&format!(
        "          <IntrinsicDuration>{}</IntrinsicDuration>\n",
        reel.picture_duration
    ));
    b.push_str(&format!(
        "          <meta:FullContentTitleText>{}</meta:FullContentTitleText>\n",
        escape_xml(&config.title)
    ));
    b.push_str(&format!(
        "          <meta:MainSoundConfiguration>{}</meta:MainSoundConfiguration>\n",
        sound.configuration
    ));
    b.push_str(&format!(
        "          <meta:MainSoundSampleRate>{} 1</meta:MainSoundSampleRate>\n",
        sound.sample_rate
    ));
    b.push_str("          <meta:MainPictureStoredArea>\n");
    b.push_str(&format!(
        "            <meta:Width>{}</meta:Width>\n",
        reel.picture_width
    ));
    b.push_str(&format!(
        "            <meta:Height>{}</meta:Height>\n",
        reel.picture_height
    ));
    b.push_str("          </meta:MainPictureStoredArea>\n");
    b.push_str("          <meta:MainPictureActiveArea>\n");
    b.push_str(&format!(
        "            <meta:Width>{}</meta:Width>\n",
        reel.picture_width
    ));
    b.push_str(&format!(
        "            <meta:Height>{}</meta:Height>\n",
        reel.picture_height
    ));
    b.push_str("          </meta:MainPictureActiveArea>\n");
    if let Some(ref lang) = reel.subtitle_language {
        b.push_str(&format!(
            "          <meta:MainSubtitleLanguageList>{}</meta:MainSubtitleLanguageList>\n",
            escape_xml(lang)
        ));
    }
    // Bv2.1 marker, matching the ISDCF SMPTE-Bv21 test content
    b.push_str("          <meta:ExtensionMetadataList>\n");
    b.push_str(
        "            <meta:ExtensionMetadata scope=\"http://www.smpte-ra.org/schemas/429-16/2013/Ext-Meta#scope\">\n",
    );
    b.push_str("              <meta:Name>SMPTE</meta:Name>\n");
    b.push_str("              <meta:PropertyList>\n");
    b.push_str("                <meta:Property>\n");
    b.push_str("                  <meta:Name>Version</meta:Name>\n");
    b.push_str("                  <meta:Value>Bv2.1</meta:Value>\n");
    b.push_str("                </meta:Property>\n");
    b.push_str("              </meta:PropertyList>\n");
    b.push_str("            </meta:ExtensionMetadata>\n");
    // ISDCF Doc 13 sign-language video: the Language Tag identifies the packed
    // VP9 program carried on channel 15 of the sound track.
    if let Some(lang) = config.sign_language.as_deref() {
        b.push_str(
            "            <meta:ExtensionMetadata scope=\"http://isdcf.com/2017/10/SignLanguageVideo\">\n",
        );
        b.push_str("              <meta:Name>Sign Language Video</meta:Name>\n");
        b.push_str("              <meta:PropertyList>\n");
        b.push_str("                <meta:Property>\n");
        b.push_str("                  <meta:Name>Language Tag</meta:Name>\n");
        b.push_str(&format!(
            "                  <meta:Value>{}</meta:Value>\n",
            escape_xml(lang)
        ));
        b.push_str("                </meta:Property>\n");
        b.push_str("              </meta:PropertyList>\n");
        b.push_str("            </meta:ExtensionMetadata>\n");
    }
    b.push_str("          </meta:ExtensionMetadataList>\n");
    b.push_str("        </meta:CompositionMetadataAsset>\n");
    b
}

/// Replace a reel segment's `<MainPicture>…</MainPicture>` block with a ST 429-10
/// MainStereoscopicPicture (FrameRate = 2x edit rate for interleaved L/R).
fn rewrite_stereoscopic(segment: &str, reel: &CplReel, standard: crate::Standard) -> String {
    const OPEN: &str = "        <MainPicture>\n";
    const CLOSE: &str = "        </MainPicture>\n";
    let (Some(start), Some(close)) = (segment.find(OPEN), segment.find(CLOSE)) else {
        return segment.to_string();
    };
    let block_end = close + CLOSE.len();
    let mut out = String::with_capacity(segment.len() + 128);
    out.push_str(&segment[..start]);
    out.push_str(&stereoscopic_picture_block(reel, standard));
    out.push_str(&segment[block_end..]);
    out
}

fn stereoscopic_picture_block(reel: &CplReel, standard: crate::Standard) -> String {
    // prefixed form: the element sits in the 429-10 namespace while its children
    // (Id, EditRate, ...) inherit the CPL's 429-7 default namespace. This is the
    // form that validates against the 429-10 XSD (whose base type is 429-7's
    // PictureTrackFileAssetType) and matches real production 3D DCPs; the
    // default-xmlns form used by some samples fails strict schema validation.
    let mut b = String::new();
    b.push_str(&format!(
        "        <msp-cpl:MainStereoscopicPicture xmlns:msp-cpl=\"{NS_STEREO_429_10}\">\n"
    ));
    b.push_str(&format!(
        "          <Id>urn:uuid:{}</Id>\n",
        reel.picture_id
    ));
    b.push_str(&format!(
        "          <EditRate>{} {}</EditRate>\n",
        reel.picture_edit_rate_num, reel.picture_edit_rate_den
    ));
    b.push_str(&format!(
        "          <IntrinsicDuration>{}</IntrinsicDuration>\n",
        reel.picture_duration
    ));
    b.push_str(&format!(
        "          <EntryPoint>{}</EntryPoint>\n",
        reel.picture_entry_point
    ));
    b.push_str(&format!(
        "          <Duration>{}</Duration>\n",
        reel.picture_duration
    ));
    if let Some(ref key_id) = reel.picture_key_id {
        b.push_str(&format!("          <KeyId>urn:uuid:{key_id}</KeyId>\n"));
    }
    // stereoscopic essence carries two frames (left+right) per edit unit
    b.push_str(&format!(
        "          <FrameRate>{} {}</FrameRate>\n",
        reel.picture_edit_rate_num * 2,
        reel.picture_edit_rate_den
    ));
    match standard {
        crate::Standard::Interop => b.push_str(&format!(
            "          <ScreenAspectRatio>{}</ScreenAspectRatio>\n",
            interop_aspect(reel.picture_width, reel.picture_height)
        )),
        crate::Standard::Smpte => b.push_str(&format!(
            "          <ScreenAspectRatio>{} {}</ScreenAspectRatio>\n",
            reel.picture_width, reel.picture_height
        )),
    }
    b.push_str("        </msp-cpl:MainStereoscopicPicture>\n");
    b
}

/// Interop ScreenAspectRatio is a decimal ratio rounded to 2 dp.
fn interop_aspect(w: u32, h: u32) -> String {
    if h == 0 {
        return "0".to_string();
    }
    format!("{:.2}", w as f64 / h as f64)
}

fn aux_data_block(aux: &AuxData) -> String {
    let mut b = String::new();
    b.push_str(&format!(
        "        <axd:AuxData xmlns:axd=\"{NS_AUX_DATA}\">\n"
    ));
    b.push_str(&format!("          <Id>urn:uuid:{}</Id>\n", aux.id));
    b.push_str(&format!(
        "          <EditRate>{} {}</EditRate>\n",
        aux.edit_rate_num, aux.edit_rate_den
    ));
    b.push_str(&format!(
        "          <IntrinsicDuration>{}</IntrinsicDuration>\n",
        aux.duration
    ));
    b.push_str(&format!(
        "          <EntryPoint>{}</EntryPoint>\n",
        aux.entry_point
    ));
    b.push_str(&format!(
        "          <Duration>{}</Duration>\n",
        aux.duration
    ));
    if let Some(ref key_id) = aux.key_id {
        b.push_str(&format!("          <KeyId>urn:uuid:{key_id}</KeyId>\n"));
    }
    b.push_str(&format!(
        "          <axd:DataType>{}</axd:DataType>\n",
        aux.data_type
    ));
    b.push_str("        </axd:AuxData>\n");
    b
}

/// Give a composition the Bv2.1 marker pair: FFOC at frame 1 of the first reel,
/// LFOC one before the end of the last reel. Each offset is relative to the reel
/// that carries it, which is how validators read them.
pub fn apply_default_markers(reels: &mut [CplReel]) {
    use crate::markers::{Marker, MarkerEntry};
    if let Some(first) = reels.first_mut() {
        first.markers.push(MarkerEntry::new(Marker::Ffoc, 1));
    }
    if let Some(last) = reels.last_mut() {
        let end = last.picture_duration.saturating_sub(1);
        last.markers.push(MarkerEntry::new(Marker::Lfoc, end));
    }
}

/// Put the reel's MainMarkers asset at the head of its AssetList. ST 429-7
/// orders the list (MainMarkers, MainPicture, MainSound, MainSubtitle), so it
/// cannot go with the other spliced assets at the end.
fn insert_main_markers(segment: &str, reel: &CplReel) -> String {
    const ASSET_LIST_OPEN: &str = "      <AssetList>\n";
    let block = main_markers_block(reel);
    match segment.find(ASSET_LIST_OPEN) {
        Some(pos) => {
            let head = pos + ASSET_LIST_OPEN.len();
            format!("{}{block}{}", &segment[..head], &segment[head..])
        }
        None => format!("{segment}{block}"),
    }
}

/// ST 429-7 MainMarkers asset. It is an inline track with no essence file, so
/// nothing references it from the PKL or ASSETMAP. Its EditRate matches the
/// picture's, or a marker Offset would count in different units than the reel.
fn main_markers_block(reel: &CplReel) -> String {
    let id = uuid::Uuid::new_v4();
    let mut b = String::new();
    b.push_str("        <MainMarkers>\n");
    b.push_str(&format!("          <Id>urn:uuid:{id}</Id>\n"));
    b.push_str(&format!(
        "          <EditRate>{} {}</EditRate>\n",
        reel.picture_edit_rate_num, reel.picture_edit_rate_den
    ));
    b.push_str(&format!(
        "          <IntrinsicDuration>{}</IntrinsicDuration>\n",
        reel.picture_duration
    ));
    b.push_str(&crate::markers::markers_to_xml(&reel.markers, "          "));
    b.push_str("        </MainMarkers>\n");
    b
}

fn main_subtitle_block(reel: &CplReel, subtitle_id: &str) -> String {
    let mut b = String::new();
    b.push_str("        <MainSubtitle>\n");
    b.push_str(&format!("          <Id>urn:uuid:{subtitle_id}</Id>\n"));
    b.push_str(&format!(
        "          <EditRate>{} {}</EditRate>\n",
        reel.subtitle_edit_rate_num, reel.subtitle_edit_rate_den
    ));
    b.push_str(&format!(
        "          <IntrinsicDuration>{}</IntrinsicDuration>\n",
        reel.subtitle_duration
    ));
    b.push_str(&format!(
        "          <EntryPoint>{}</EntryPoint>\n",
        reel.subtitle_entry_point
    ));
    b.push_str(&format!(
        "          <Duration>{}</Duration>\n",
        reel.subtitle_duration
    ));
    if let Some(ref lang) = reel.subtitle_language {
        b.push_str(&format!("          <Language>{lang}</Language>\n"));
    }
    b.push_str("        </MainSubtitle>\n");
    b
}

/// ST 429-12 closed-caption asset. Same timed-text structure as MainSubtitle but
/// a distinct accessibility role, so validators and playback treat it as CCAP.
fn main_closed_caption_block(reel: &CplReel, ccap_id: &str) -> String {
    let mut b = String::new();
    b.push_str("        <MainClosedCaption>\n");
    b.push_str(&format!("          <Id>urn:uuid:{ccap_id}</Id>\n"));
    b.push_str(&format!(
        "          <EditRate>{} {}</EditRate>\n",
        reel.ccap_edit_rate_num, reel.ccap_edit_rate_den
    ));
    b.push_str(&format!(
        "          <IntrinsicDuration>{}</IntrinsicDuration>\n",
        reel.ccap_duration
    ));
    b.push_str(&format!(
        "          <EntryPoint>{}</EntryPoint>\n",
        reel.ccap_entry_point
    ));
    b.push_str(&format!(
        "          <Duration>{}</Duration>\n",
        reel.ccap_duration
    ));
    if let Some(ref lang) = reel.ccap_language {
        b.push_str(&format!("          <Language>{lang}</Language>\n"));
    }
    b.push_str("        </MainClosedCaption>\n");
    b
}

fn time_now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_sound_configuration_by_channel_count() {
        assert_eq!(
            main_sound_configuration(2, None, None).as_deref(),
            Some("20/L,R")
        );
        assert_eq!(
            main_sound_configuration(6, None, None).as_deref(),
            Some("51/L,R,C,LFE,Ls,Rs")
        );
        assert_eq!(
            main_sound_configuration(8, None, None).as_deref(),
            Some("71/L,R,C,LFE,Lss,Rss,Lrs,Rrs")
        );
        // a 5.1 source padded to a 16-channel MXF carries ten '-' fills
        assert_eq!(
            main_sound_configuration(16, None, None).as_deref(),
            Some("51/L,R,C,LFE,Ls,Rs,-,-,-,-,-,-,-,-,-,-")
        );
        // HI/VI take their own channel slots after the main layout
        assert_eq!(
            main_sound_configuration(8, Some(6), Some(7)).as_deref(),
            Some("51/L,R,C,LFE,Ls,Rs,HI,VIN")
        );
    }

    fn sound_reel() -> CplReel {
        CplReel {
            reel_id: "r1".into(),
            picture_id: "pic1".into(),
            picture_width: 2048,
            picture_height: 858,
            picture_edit_rate_num: 24,
            picture_edit_rate_den: 1,
            picture_duration: 240,
            sound_id: Some("snd1".into()),
            sound_edit_rate_num: 24,
            sound_edit_rate_den: 1,
            sound_duration: 240,
            ..Default::default()
        }
    }

    #[test]
    fn smpte_cpl_writes_composition_metadata_asset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let config = CplConfig {
            title: "Meta Test".into(),
            content_kind: "feature".into(),
            reels: vec![sound_reel()],
            standard: crate::Standard::Smpte,
            main_sound: Some(MainSound {
                configuration: "51/L,R,C,LFE,Ls,Rs,-,-,-,-,-,-,-,-,-,-".into(),
                sample_rate: 48000,
            }),
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains(
            "<meta:CompositionMetadataAsset xmlns:meta=\"http://www.smpte-ra.org/schemas/429-16/2014/CPL-Metadata\">"
        ));
        assert!(xml.contains(
            "<meta:MainSoundConfiguration>51/L,R,C,LFE,Ls,Rs,-,-,-,-,-,-,-,-,-,-</meta:MainSoundConfiguration>"
        ));
        assert!(xml.contains("<meta:MainSoundSampleRate>48000 1</meta:MainSoundSampleRate>"));
        assert!(xml.contains("<meta:FullContentTitleText>Meta Test</meta:FullContentTitleText>"));
        assert!(xml.contains("<meta:Width>2048</meta:Width>"));
        assert!(xml.contains("<meta:ExtensionMetadata"));
        // it sits inside the first reel's AssetList
        let meta_pos = xml.find("<meta:CompositionMetadataAsset").unwrap();
        let close_pos = xml.find("</AssetList>").unwrap();
        assert!(meta_pos < close_pos);
    }

    #[test]
    fn cpl_emits_closed_caption_asset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let mut reel = sound_reel();
        reel.ccap_id = Some("ccap-uuid-1".into());
        reel.ccap_edit_rate_num = 24;
        reel.ccap_edit_rate_den = 1;
        reel.ccap_duration = 240;
        reel.ccap_language = Some("en".into());
        let config = CplConfig {
            title: "CCAP Test".into(),
            content_kind: "feature".into(),
            reels: vec![reel],
            standard: crate::Standard::Smpte,
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains("<MainClosedCaption>"));
        assert!(xml.contains("<Id>urn:uuid:ccap-uuid-1</Id>"));
        assert!(xml.contains("<Language>en</Language>"));
        // closed caption is distinct from the open subtitle role
        assert!(!xml.contains("<MainSubtitle>"));
        // and it sits inside a reel AssetList
        let cc_pos = xml.find("<MainClosedCaption>").unwrap();
        let close_pos = xml.find("</AssetList>").unwrap();
        assert!(cc_pos < close_pos);
    }

    #[test]
    fn interop_cpl_has_no_metadata_asset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let config = CplConfig {
            title: "Interop".into(),
            content_kind: "feature".into(),
            reels: vec![sound_reel()],
            standard: crate::Standard::Interop,
            main_sound: Some(MainSound {
                configuration: "51/L,R,C,LFE,Ls,Rs".into(),
                sample_rate: 48000,
            }),
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(!xml.contains("CompositionMetadataAsset"));
    }

    #[test]
    fn generate_cpl_uses_supplied_identity_and_standard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL_expected.xml");
        let config = CplConfig {
            title: "Interop Test".into(),
            content_kind: "test".into(),
            standard: crate::Standard::Interop,
            ..Default::default()
        };

        assert_eq!(generate_cpl(&config, "expected", &path), 0);
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<Id>urn:uuid:expected</Id>"));
        assert!(xml.contains("<ContentTitleText>Interop Test</ContentTitleText>"));
        assert!(xml.contains("PROTO-ASDCP-CPL-20040511"));
    }

    fn stereo_reel() -> CplReel {
        CplReel {
            reel_id: "r1".into(),
            picture_id: "pic1".into(),
            picture_width: 2048,
            picture_height: 858,
            picture_edit_rate_num: 24,
            picture_edit_rate_den: 1,
            picture_duration: 240,
            stereoscopic: true,
            ..Default::default()
        }
    }

    #[test]
    fn stereoscopic_reel_emits_main_stereoscopic_picture() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let config = CplConfig {
            title: "3D".into(),
            content_kind: "feature".into(),
            reels: vec![stereo_reel()],
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(!xml.contains("<MainPicture>"), "picture replaced by stereo");
        assert!(xml.contains(
            "<msp-cpl:MainStereoscopicPicture xmlns:msp-cpl=\"http://www.smpte-ra.org/schemas/429-10/2008/Main-Stereo-Picture-CPL\">"
        ));
        // edit rate is the composition rate; frame rate is doubled (L+R interleaved)
        assert!(xml.contains("<EditRate>24 1</EditRate>"));
        assert!(xml.contains("<FrameRate>48 1</FrameRate>"));
        assert!(xml.contains("</msp-cpl:MainStereoscopicPicture>"));
    }

    #[test]
    fn aux_data_reel_emits_axd_auxdata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let mut reel = CplReel {
            reel_id: "r1".into(),
            picture_id: "pic1".into(),
            picture_width: 2048,
            picture_height: 858,
            picture_edit_rate_num: 24,
            picture_edit_rate_den: 1,
            picture_duration: 240,
            ..Default::default()
        };
        reel.aux_data = Some(AuxData {
            id: "aux1".into(),
            edit_rate_num: 24,
            edit_rate_den: 1,
            duration: 240,
            entry_point: 0,
            key_id: None,
            data_type: "urn:smpte:ul:060e2b34.04010105.0e090604.00000000".into(),
        });
        let config = CplConfig {
            title: "Atmos".into(),
            content_kind: "feature".into(),
            reels: vec![reel],
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains("<axd:AuxData xmlns:axd=\"http://www.dolby.com/schemas/2012/AD\">"));
        assert!(xml.contains("<Id>urn:uuid:aux1</Id>"));
        assert!(xml.contains(
            "<axd:DataType>urn:smpte:ul:060e2b34.04010105.0e090604.00000000</axd:DataType>"
        ));
        // aux data sits after MainPicture/MainSound, before </AssetList>
        let aux_pos = xml.find("<axd:AuxData").unwrap();
        let close_pos = xml.find("</AssetList>").unwrap();
        assert!(aux_pos < close_pos);
    }
}
