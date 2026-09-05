use serde::{Deserialize, Serialize};
use std::path::Path;

/// CPL configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CplConfig {
    pub title: String,
    pub content_kind: String,
    pub rating: String,
    /// Certification ratings for the RatingList. Empty writes an empty list.
    #[serde(default)]
    pub ratings: Vec<crate::isdcf_name::Rating>,
    /// LabelText of the ContentVersion element. None writes the title.
    #[serde(default)]
    pub content_version_label: Option<String>,
    pub reels: Vec<CplReel>,
    pub standard: crate::Standard,
    /// Packaged sound layout for the SMPTE CompositionMetadataAsset (ST 429-16).
    /// None for a silent composition or Interop (no metadata asset is written).
    pub main_sound: Option<MainSound>,
    /// RFC 5646 sign-language tag (ISDCF Doc 13). When set, the CompositionMetadata
    /// carries the SignLanguageVideo ExtensionMetadata with this Language Tag.
    #[serde(default)]
    pub sign_language: Option<String>,
    /// ST 429-16 composition identity. Every one is optional in the schema and
    /// the order here is the order the schema requires them in.
    #[serde(default)]
    pub release_territory: Option<String>,
    /// Defaults to 1 when unset, since Bv2.1 requires the element.
    #[serde(default)]
    pub version_number: Option<u32>,
    #[serde(default)]
    pub chain: Option<String>,
    #[serde(default)]
    pub distributor: Option<String>,
    #[serde(default)]
    pub facility: Option<String>,
    /// Mastering luminance, written with the required `units` attribute.
    #[serde(default)]
    pub luminance: Option<Luminance>,
}

/// Mastering luminance for the CompositionMetadataAsset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Luminance {
    pub value: f64,
    pub units: LuminanceUnits,
}

/// The two units ST 429-16 allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LuminanceUnits {
    FootLambert,
    CandelaPerSquareMetre,
}

impl LuminanceUnits {
    fn as_str(self) -> &'static str {
        match self {
            Self::FootLambert => "foot-lambert",
            Self::CandelaPerSquareMetre => "candela-per-square-metre",
        }
    }
}

impl Luminance {
    /// Parse `"<value> <units>"`, the two units ST 429-16 allows. The value must
    /// be positive: the schema type excludes zero.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let (value, units) = spec.trim().split_once(char::is_whitespace).ok_or_else(|| {
            format!("luminance '{spec}' must be a value and a unit, e.g. '14 foot-lambert'")
        })?;
        let value: f64 = value
            .trim()
            .parse()
            .map_err(|_| format!("luminance value '{value}' is not a number"))?;
        if value <= 0.0 {
            return Err(format!("luminance {value} must be greater than zero"));
        }
        let units = match units.trim() {
            "foot-lambert" => LuminanceUnits::FootLambert,
            "candela-per-square-metre" => LuminanceUnits::CandelaPerSquareMetre,
            other => {
                return Err(format!(
                    "luminance unit '{other}' must be foot-lambert or candela-per-square-metre"
                ));
            }
        };
        Ok(Self { value, units })
    }
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
    /// Coded raster of the picture essence: how big the frames are. Declared as
    /// MainPictureStoredArea and as the ScreenAspectRatio, and it must match the
    /// AspectRatio the picture MXF carries. Never a container preset.
    pub picture_width: u32,
    pub picture_height: u32,
    /// Active image rectangle inside the coded raster: what a projector masks to.
    /// Zero means the whole coded raster is active. Never larger than it.
    #[serde(default)]
    pub picture_active_width: u32,
    #[serde(default)]
    pub picture_active_height: u32,
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
    /// Base64 SHA-1 of each track file, repeated from the PKL so a server that
    /// hash-checks against the CPL has something to check.
    pub picture_hash: Option<String>,
    pub sound_hash: Option<String>,
    /// Bare UUID of the timed-text (subtitle) track, when present.
    pub subtitle_id: Option<String>,
    pub subtitle_edit_rate_num: u32,
    pub subtitle_edit_rate_den: u32,
    pub subtitle_duration: u64,
    pub subtitle_entry_point: u64,
    pub subtitle_language: Option<String>,
    #[serde(default)]
    pub subtitle_hash: Option<String>,
    /// KeyId (bare UUID) when the timed-text essence is encrypted.
    #[serde(default)]
    pub subtitle_key_id: Option<String>,
    /// Bare UUID of the closed-caption (ST 429-12) timed-text track, when present.
    /// Distinct from the open subtitle: emitted as ST 429-12 ClosedCaption.
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
    #[serde(default)]
    pub ccap_hash: Option<String>,
    /// KeyId (bare UUID) when the closed-caption essence is encrypted.
    #[serde(default)]
    pub ccap_key_id: Option<String>,
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

impl CplReel {
    /// The active image rectangle, defaulting to the whole coded raster.
    pub fn active_area(&self) -> (u32, u32) {
        if self.picture_active_width > 0 && self.picture_active_height > 0 {
            (self.picture_active_width, self.picture_active_height)
        } else {
            (self.picture_width, self.picture_height)
        }
    }
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

/// The picture geometry a CPL declares for one composition.
///
/// `stored` is the coded raster the encoder actually produced, read back from the
/// codestream: MainPictureStoredArea and ScreenAspectRatio declare it, and it is
/// the ratio the picture MXF descriptor carries. `active` is the image rectangle
/// inside it that a projector masks to, from `--container` / `--container-dims`.
/// Active defaults to the whole coded raster and can never exceed it (ST 429-16,
/// libdcp INVALID_MAIN_PICTURE_ACTIVE_AREA).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureGeometry {
    pub stored_width: u32,
    pub stored_height: u32,
    pub active_width: u32,
    pub active_height: u32,
}

/// Read the coded raster from the first codestream in `j2k_dir` and pair it with
/// the requested container. A CPL declares the essence that was written, so an
/// unreadable codestream is an error here rather than a fall back to a preset.
pub fn picture_geometry(
    j2k_dir: &Path,
    container_width: u32,
    container_height: u32,
) -> Result<PictureGeometry, String> {
    let frame = crate::reel::collect_frames(j2k_dir)
        .into_iter()
        .next()
        .ok_or_else(|| format!("no J2K codestream in {}", j2k_dir.display()))?;
    let (stored_width, stored_height) = crate::pad::read_j2k_dimensions(&frame)?;
    let (active_width, active_height) = if container_width > 0 && container_height > 0 {
        (container_width, container_height)
    } else {
        (stored_width, stored_height)
    };
    check_active_area_fits(stored_width, stored_height, active_width, active_height)?;
    Ok(PictureGeometry {
        stored_width,
        stored_height,
        active_width,
        active_height,
    })
}

/// ST 429-16 and libdcp's INVALID_MAIN_PICTURE_ACTIVE_AREA both require the
/// active area to fit inside the coded raster. The plan knows the raster before
/// the encode, so `preflight` applies this rule there as well as here.
pub fn check_active_area_fits(
    stored_width: u32,
    stored_height: u32,
    active_width: u32,
    active_height: u32,
) -> Result<(), String> {
    if active_width <= stored_width && active_height <= stored_height {
        return Ok(());
    }
    Err(format!(
        "container {active_width}x{active_height} is larger than the \
         {stored_width}x{stored_height} picture the encoder produced: the CPL would \
         declare an active area the essence does not have. Encode at the container \
         size, or pick a container that fits inside the frames"
    ))
}

/// The active area a CPL declares, when it carries a CompositionMetadataAsset.
/// Used by the paths that rebuild a CPL from an existing package, so a masked
/// composition keeps its masking.
pub fn active_area_from_cpl(cpl_xml: &str) -> Option<(u32, u32)> {
    let doc = roxmltree::Document::parse(cpl_xml).ok()?;
    let area = doc
        .descendants()
        .find(|n| n.has_tag_name("MainPictureActiveArea"))?;
    let edge = |name: &str| -> Option<u32> {
        area.descendants()
            .find(|n| n.has_tag_name(name))
            .and_then(|n| n.text())
            .and_then(|t| t.trim().parse().ok())
    };
    Some((edge("Width")?, edge("Height")?))
}

/// The sound layout a CPL declares in its CompositionMetadataAsset. Used by the
/// paths that rebuild a CPL around the source's own sound MXF, so the block they
/// write describes the essence they ship. An Interop CPL has no such block.
pub fn main_sound_from_cpl(cpl_xml: &str) -> Option<MainSound> {
    let doc = roxmltree::Document::parse(cpl_xml).ok()?;
    let element_text = |name: &str| -> Option<String> {
        doc.descendants()
            .find(|n| n.has_tag_name(name))
            .and_then(|n| n.text())
            .map(|t| t.trim().to_string())
    };
    let configuration = element_text("MainSoundConfiguration")?;
    // the element is a rational, "48000 1"
    let rate = element_text("MainSoundSampleRate")?;
    let sample_rate = rate.split_whitespace().next()?.parse().ok()?;
    Some(MainSound {
        configuration,
        sample_rate,
    })
}

const MCA_CHANNEL_TAG_PREFIX: &str = "ch";
const MCA_SOUNDFIELD_TAG_PREFIX: &str = "sg";
const ACCESSIBILITY_CHANNEL_TAGS: [&str; 2] = ["HI", "VIN"];

pub fn main_sound_from_track_file(path: &Path) -> Result<MainSound, String> {
    let file = path.to_string_lossy().to_string();
    let mut reader = asdcplib::pcm::MxfReader::new();
    // an encrypted track still reads here, the descriptor is in the plain header
    reader
        .open_read(&file)
        .map_err(|e| format!("cannot open the sound track {}: {e}", path.display()))?;
    let descriptor = reader.audio_descriptor().map_err(|e| {
        format!(
            "cannot read the audio descriptor of {}: {e}",
            path.display()
        )
    })?;
    let rate = descriptor.audio_sampling_rate;
    if rate.numerator <= 0 || rate.denominator <= 0 {
        return Err(format!(
            "{} declares the sampling rate {} {}",
            path.display(),
            rate.numerator,
            rate.denominator
        ));
    }
    let labels = reader
        .mca_label_subdescriptors()
        .map_err(|e| format!("cannot read the MCA labels of {}: {e}", path.display()))?;
    let configuration = mca_main_sound_configuration(&labels, descriptor.channel_count)
        .unwrap_or_else(|| {
            main_sound_configuration(
                descriptor.channel_count,
                descriptor.channel_count,
                None,
                None,
            )
        });
    Ok(MainSound {
        configuration,
        sample_rate: (rate.numerator / rate.denominator) as u32,
    })
}

fn mca_main_sound_configuration(
    labels: &[asdcplib::pcm::McaLabelSubDescriptor],
    channel_count: u32,
) -> Option<String> {
    use asdcplib::pcm::McaLabelKind;

    let mut slots = vec![SOUNDFIELD_NONE; channel_count as usize];
    let mut labelled_any_channel = false;
    for label in labels
        .iter()
        .filter(|l| l.kind == McaLabelKind::AudioChannel)
    {
        // MCAChannelID is one based
        let Some(channel_id) = label.channel_id.filter(|id| *id > 0) else {
            continue;
        };
        let Some(slot) = slots.get_mut(channel_id as usize - 1) else {
            continue;
        };
        *slot = strip_mca_prefix(&label.tag_symbol, MCA_CHANNEL_TAG_PREFIX);
        labelled_any_channel = true;
    }
    if !labelled_any_channel {
        return None;
    }
    // stereo and mono have no ST 428-12 group label, so the count names the group
    let group = labels
        .iter()
        .find(|l| l.kind == McaLabelKind::SoundfieldGroup)
        .map(|l| strip_mca_prefix(&l.tag_symbol, MCA_SOUNDFIELD_TAG_PREFIX))
        .unwrap_or_else(|| {
            let main_channels = slots
                .iter()
                .filter(|slot| {
                    **slot != SOUNDFIELD_NONE && !ACCESSIBILITY_CHANNEL_TAGS.contains(slot)
                })
                .count();
            soundfield_group(main_channels as u32)
        });
    Some(format!("{group}/{}", slots.join(",")))
}

fn strip_mca_prefix<'a>(tag_symbol: &'a str, prefix: &str) -> &'a str {
    tag_symbol.strip_prefix(prefix).unwrap_or(tag_symbol)
}

/// Generate a Composition Playlist XML via the shared postkit writer.
pub fn generate_cpl(config: &CplConfig, cpl_uuid: &str, output_file: &Path) -> i32 {
    use postkit::packaging::{self, DcpCpl, DcpCplReel, DcpRating};

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
            picture_hash: r.picture_hash.clone(),
            sound_hash: r.sound_hash.clone(),
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
        // Bv2.1 8.1: present, and equal to the content title
        annotation_text: Some(config.title.clone()),
        content_version_label: config.content_version_label.clone(),
        ratings: config
            .ratings
            .iter()
            .map(|rating| DcpRating {
                agency: rating.agency.clone(),
                label: rating.label.clone(),
            })
            .collect(),
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
/// ST 429-12 (2008) timed-text namespace, which is where a SMPTE closed caption
/// sits.
const NS_TIMED_TEXT_429_12: &str = "http://www.smpte-ra.org/schemas/429-12/2008/TT";

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
            segment = remove_main_picture(&segment);
        }
        if !reel.markers.is_empty() {
            segment = insert_main_markers(&segment, reel);
        }
        out.push_str(&segment);
        if let Some(ref sid) = reel.subtitle_id {
            out.push_str(&main_subtitle_block(reel, sid));
        }
        // everything below is in a namespace of its own, and 429-7 admits those
        // through an xs:any that follows MainSubtitle: a foreign element written
        // where MainPicture sat makes the AssetList schema-invalid
        if reel.stereoscopic {
            out.push_str(&stereoscopic_picture_block(reel, standard));
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

/// Scope under which a `ReleaseTerritory` is a UN M.49 numeric code.
const UNM49_SCOPE: &str =
    "http://www.smpte-ra.org/schemas/429-16/2014/CPL-Metadata#scope/release-territory/UNM49";

/// ST 428-12 soundfield group for a single channel, which ISDCF's audio-config
/// registry gives as the DCP naming code 10, "1.0 (Center Channel Mono)".
const SOUNDFIELD_MONO: &str = "M";

/// Written where no ST 428-12 group describes the channel count. ISDCF uses '-'
/// for a MainSoundConfiguration with nothing to declare.
const SOUNDFIELD_NONE: &str = "-";

fn soundfield_group(main_channels: u32) -> &'static str {
    match main_channels {
        1 => SOUNDFIELD_MONO,
        2 => "20",
        6 | 16 => "51",
        8 => "71",
        _ => SOUNDFIELD_NONE,
    }
}

/// Build the ST 429-16 MainSoundConfiguration. The group comes from the channels
/// the content fills, the slot count from the channels the track is packaged
/// with, so the silent fill channels are written '-' rather than counted into
/// the group.
///
/// A layout with no canonical DCP labels keeps its slot count and writes every
/// slot '-': the sound essence carries no MCA labels to name those channels
/// with, and ISDCF's own guidance for channels with nothing to declare is the
/// '-' form. The string is required either way, since Bv2.1 wants the metadata
/// asset that holds it on every SMPTE CPL.
pub fn main_sound_configuration(
    content_channels: u32,
    packaged_channels: u32,
    hi_channel: Option<u32>,
    vi_channel: Option<u32>,
) -> String {
    let extra = hi_channel.is_some() as u32 + vi_channel.is_some() as u32;
    let main_count = content_channels.saturating_sub(extra);
    // canonical DCP channel order per ISDCF / SMPTE RDD 52. 16 is 5.1 because no
    // content is 16 channels wide: a caller that knows only the packaged count is
    // looking at the 5.1 track the wrap has always filled.
    let labels: &[&str] = match main_count {
        2 => &["L", "R"],
        6 | 16 => &["L", "R", "C", "LFE", "Ls", "Rs"],
        8 => &["L", "R", "C", "LFE", "Lss", "Rss", "Lrs", "Rrs"],
        _ => &[],
    };
    let group = soundfield_group(main_count);
    let mut slots = vec![SOUNDFIELD_NONE; packaged_channels as usize];
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
    format!("{group}/{}", slots.join(","))
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
    if let Some(ref territory) = config.release_territory {
        // a numeric value is a UN M.49 code, which readers only accept as one
        // when the scope says so; an alphabetic one is an RFC 5646 region
        // subtag and takes the schema's own default scope.
        let scope = if territory.chars().all(|c| c.is_ascii_digit()) {
            format!(" scope=\"{UNM49_SCOPE}\"")
        } else {
            String::new()
        };
        b.push_str(&format!(
            "          <meta:ReleaseTerritory{scope}>{}</meta:ReleaseTerritory>\n",
            escape_xml(territory)
        ));
    }
    // Bv2.1 8.6.1 requires this, so it is always written; the schema's own
    // default is 1, which is what a first delivery is.
    b.push_str(&format!(
        "          <meta:VersionNumber>{}</meta:VersionNumber>\n",
        config.version_number.unwrap_or(1)
    ));
    for (element, value) in [
        ("Chain", config.chain.as_deref()),
        ("Distributor", config.distributor.as_deref()),
        ("Facility", config.facility.as_deref()),
    ] {
        if let Some(value) = value {
            b.push_str(&format!(
                "          <meta:{element}>{}</meta:{element}>\n",
                escape_xml(value)
            ));
        }
    }
    if let Some(ref luminance) = config.luminance {
        b.push_str(&format!(
            "          <meta:Luminance units=\"{}\">{}</meta:Luminance>\n",
            luminance.units.as_str(),
            luminance.value
        ));
    }
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
    let (active_width, active_height) = reel.active_area();
    b.push_str("          <meta:MainPictureActiveArea>\n");
    b.push_str(&format!(
        "            <meta:Width>{active_width}</meta:Width>\n"
    ));
    b.push_str(&format!(
        "            <meta:Height>{active_height}</meta:Height>\n"
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

/// Drop a reel segment's `<MainPicture>…</MainPicture>` block. A stereoscopic
/// reel declares its picture as the ST 429-10 MainStereoscopicPicture appended
/// further down the AssetList instead.
fn remove_main_picture(segment: &str) -> String {
    const OPEN: &str = "        <MainPicture>\n";
    const CLOSE: &str = "        </MainPicture>\n";
    let (Some(start), Some(close)) = (segment.find(OPEN), segment.find(CLOSE)) else {
        return segment.to_string();
    };
    format!("{}{}", &segment[..start], &segment[close + CLOSE.len()..])
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
    // ST 429-7 PictureTrackFileAssetType, which 429-10 reuses: KeyId then Hash,
    // both before FrameRate
    if let Some(ref key_id) = reel.picture_key_id {
        b.push_str(&format!("          <KeyId>urn:uuid:{key_id}</KeyId>\n"));
    }
    if let Some(ref hash) = reel.picture_hash {
        b.push_str(&format!("          <Hash>{hash}</Hash>\n"));
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
    // ST 429-7 TrackFileAssetType orders KeyId before Hash, and Language after both
    if let Some(ref key_id) = reel.subtitle_key_id {
        b.push_str(&format!("          <KeyId>urn:uuid:{key_id}</KeyId>\n"));
    }
    if let Some(ref hash) = reel.subtitle_hash {
        b.push_str(&format!("          <Hash>{hash}</Hash>\n"));
    }
    if let Some(ref lang) = reel.subtitle_language {
        b.push_str(&format!("          <Language>{lang}</Language>\n"));
    }
    b.push_str("        </MainSubtitle>\n");
    b
}

/// ST 429-12 closed-caption asset. Same timed-text structure as MainSubtitle but
/// a distinct accessibility role, so validators and playback treat it as CCAP.
///
/// prefixed form: 429-7's AssetList only accepts a foreign-namespace element
/// after MainSubtitle, and ST 429-12 declares the element as ClosedCaption with
/// its Language qualified (elementFormDefault="qualified"). The other children
/// come from 429-7's TrackFileAssetType, so they stay in the CPL default
/// namespace.
fn main_closed_caption_block(reel: &CplReel, ccap_id: &str) -> String {
    let mut b = String::new();
    b.push_str(&format!(
        "        <tt:ClosedCaption xmlns:tt=\"{NS_TIMED_TEXT_429_12}\">\n"
    ));
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
    if let Some(ref key_id) = reel.ccap_key_id {
        b.push_str(&format!("          <KeyId>urn:uuid:{key_id}</KeyId>\n"));
    }
    if let Some(ref hash) = reel.ccap_hash {
        b.push_str(&format!("          <Hash>{hash}</Hash>\n"));
    }
    if let Some(ref lang) = reel.ccap_language {
        b.push_str(&format!("          <tt:Language>{lang}</tt:Language>\n"));
    }
    b.push_str("        </tt:ClosedCaption>\n");
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
        assert_eq!(main_sound_configuration(2, 2, None, None), "20/L,R");
        assert_eq!(
            main_sound_configuration(6, 6, None, None),
            "51/L,R,C,LFE,Ls,Rs"
        );
        assert_eq!(
            main_sound_configuration(8, 8, None, None),
            "71/L,R,C,LFE,Lss,Rss,Lrs,Rrs"
        );
        // a 5.1 source padded to a 16-channel MXF carries ten '-' fills
        assert_eq!(
            main_sound_configuration(6, 16, None, None),
            "51/L,R,C,LFE,Ls,Rs,-,-,-,-,-,-,-,-,-,-"
        );
        // HI/VI take their own channel slots after the main layout
        assert_eq!(
            main_sound_configuration(8, 8, Some(6), Some(7)),
            "51/L,R,C,LFE,Ls,Rs,HI,VIN"
        );
    }

    /// Stereo filled to a wide track keeps the stereo group: the fill channels
    /// carry silence, not a 5.1 mix.
    #[test]
    fn filled_channels_do_not_join_the_soundfield_group() {
        assert_eq!(
            main_sound_configuration(2, 16, None, None),
            "20/L,R,-,-,-,-,-,-,-,-,-,-,-,-,-,-"
        );
        assert_eq!(main_sound_configuration(2, 6, None, None), "20/L,R,-,-,-,-");
        assert_eq!(
            main_sound_configuration(6, 8, None, None),
            "51/L,R,C,LFE,Ls,Rs,-,-"
        );
    }

    /// Sound the essence carries no MCA labels for still gets a configuration,
    /// so its composition can carry the metadata asset Bv2.1 requires. The slots
    /// stay '-' rather than borrowing a layout the labels never claimed.
    #[test]
    fn unlabelled_sound_keeps_its_channel_count_and_names_no_channels() {
        assert_eq!(main_sound_configuration(1, 1, None, None), "M/-");
        assert_eq!(
            main_sound_configuration(1, 16, None, None),
            "M/-,-,-,-,-,-,-,-,-,-,-,-,-,-,-,-"
        );
        assert_eq!(main_sound_configuration(3, 3, None, None), "-/-,-,-");
        assert_eq!(main_sound_configuration(5, 5, None, None), "-/-,-,-,-,-");
    }

    /// A mono build's CPL is a SMPTE CPL, so Bv2.1 8.6.1 wants the metadata asset
    /// and its VersionNumber there too.
    #[test]
    fn unlabelled_sound_still_gets_the_composition_metadata_asset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let config = CplConfig {
            title: "Mono".into(),
            content_kind: "feature".into(),
            reels: vec![sound_reel()],
            standard: crate::Standard::Smpte,
            main_sound: Some(MainSound {
                configuration: main_sound_configuration(1, 1, None, None),
                sample_rate: 48000,
            }),
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains("<meta:CompositionMetadataAsset"), "{xml}");
        assert!(
            xml.contains("<meta:VersionNumber>1</meta:VersionNumber>"),
            "{xml}"
        );
        assert!(
            xml.contains("<meta:MainSoundConfiguration>M/-</meta:MainSoundConfiguration>"),
            "{xml}"
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
    fn main_sound_reads_back_from_a_generated_cpl() {
        let dir = tempfile::tempdir().unwrap();
        let with_sound = dir.path().join("CPL_sound.xml");
        let config = CplConfig {
            title: "Read Back".into(),
            content_kind: "feature".into(),
            reels: vec![sound_reel()],
            standard: crate::Standard::Smpte,
            main_sound: Some(MainSound {
                configuration: "51/L,R,C,LFE,Ls,Rs".into(),
                sample_rate: 48000,
            }),
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &with_sound), 0);
        let read_back = main_sound_from_cpl(&std::fs::read_to_string(&with_sound).unwrap())
            .expect("the generated CPL declares its sound layout");
        assert_eq!(read_back.configuration, "51/L,R,C,LFE,Ls,Rs");
        assert_eq!(read_back.sample_rate, 48000);

        let without_sound = dir.path().join("CPL_nosound.xml");
        let bare = CplConfig {
            main_sound: None,
            ..config
        };
        assert_eq!(generate_cpl(&bare, "cpl2", &without_sound), 0);
        let xml = std::fs::read_to_string(&without_sound).unwrap();
        assert!(!xml.contains("MainSoundConfiguration"));
        assert!(main_sound_from_cpl(&xml).is_none());

        let unparsable_rate = "<CompositionPlaylist>\
             <MainSoundConfiguration>51/L,R,C,LFE,Ls,Rs</MainSoundConfiguration>\
             <MainSoundSampleRate>forty-eight</MainSoundSampleRate>\
             </CompositionPlaylist>";
        assert!(main_sound_from_cpl(unparsable_rate).is_none());
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
        assert!(xml.contains(&format!(
            "<tt:ClosedCaption xmlns:tt=\"{NS_TIMED_TEXT_429_12}\">"
        )));
        assert!(xml.contains("<Id>urn:uuid:ccap-uuid-1</Id>"));
        assert!(xml.contains("<tt:Language>en</tt:Language>"));
        // closed caption is distinct from the open subtitle role
        assert!(!xml.contains("<MainSubtitle>"));
        // and it sits inside a reel AssetList
        let cc_pos = xml.find("<tt:ClosedCaption").unwrap();
        let close_pos = xml.find("</AssetList>").unwrap();
        assert!(cc_pos < close_pos);
    }

    /// Some servers hash-check the CPL rather than the PKL, so a timed-text
    /// asset needs its `Hash` there like picture and sound, in 429-7 order:
    /// after Duration and before Language.
    #[test]
    fn timed_text_assets_carry_their_hash_in_the_cpl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let mut reel = sound_reel();
        reel.subtitle_id = Some("sub-uuid-1".into());
        reel.subtitle_edit_rate_num = 24;
        reel.subtitle_edit_rate_den = 1;
        reel.subtitle_duration = 240;
        reel.subtitle_language = Some("en".into());
        reel.subtitle_hash = Some("c3VidGl0bGUtaGFzaA==".into());
        reel.ccap_id = Some("ccap-uuid-1".into());
        reel.ccap_edit_rate_num = 24;
        reel.ccap_edit_rate_den = 1;
        reel.ccap_duration = 240;
        reel.ccap_language = Some("en".into());
        reel.ccap_hash = Some("Y2NhcC1oYXNo".into());
        let config = CplConfig {
            title: "Hash Test".into(),
            content_kind: "feature".into(),
            reels: vec![reel],
            standard: crate::Standard::Smpte,
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();

        let subtitle = xml
            .split("<MainSubtitle>")
            .nth(1)
            .and_then(|s| s.split("</MainSubtitle>").next())
            .expect("a MainSubtitle");
        assert!(
            subtitle.contains("<Hash>c3VidGl0bGUtaGFzaA==</Hash>"),
            "{subtitle}"
        );
        assert!(
            subtitle.find("<Duration>").unwrap() < subtitle.find("<Hash>").unwrap(),
            "Hash follows Duration: {subtitle}"
        );
        assert!(
            subtitle.find("<Hash>").unwrap() < subtitle.find("<Language>").unwrap(),
            "Hash precedes Language: {subtitle}"
        );

        let ccap = xml
            .split("<tt:ClosedCaption")
            .nth(1)
            .and_then(|s| s.split("</tt:ClosedCaption>").next())
            .expect("a ClosedCaption");
        assert!(ccap.contains("<Hash>Y2NhcC1oYXNo</Hash>"), "{ccap}");
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

    #[test]
    fn ratings_and_the_content_version_label_reach_the_xml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let config = CplConfig {
            title: "Rated".into(),
            content_kind: "feature".into(),
            ratings: vec![
                crate::isdcf_name::Rating {
                    agency: "http://www.mpaa.org/2003-ratings".into(),
                    label: "PG-13".into(),
                },
                crate::isdcf_name::Rating {
                    agency: "http://www.bbfc.co.uk/BBFCRatings".into(),
                    label: "15".into(),
                },
            ],
            content_version_label: Some("Final Cut".into()),
            ..Default::default()
        };

        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains("<LabelText>Final Cut</LabelText>"), "{xml}");
        assert!(
            xml.contains("<Agency>http://www.mpaa.org/2003-ratings</Agency>"),
            "{xml}"
        );
        assert!(xml.contains("<Label>PG-13</Label>"), "{xml}");
        assert!(
            xml.find("<Agency>http://www.mpaa.org/2003-ratings</Agency>")
                < xml.find("<Agency>http://www.bbfc.co.uk/BBFCRatings</Agency>"),
            "ratings keep the order they were given: {xml}"
        );
    }

    #[test]
    fn no_content_version_label_writes_the_title() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let config = CplConfig {
            title: "Unlabelled".into(),
            content_kind: "feature".into(),
            ..Default::default()
        };

        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains("<LabelText>Unlabelled</LabelText>"), "{xml}");
        assert!(xml.contains("<RatingList/>"), "{xml}");
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

    /// 429-7 admits the 429-10 picture element through an xs:any that follows
    /// MainSound and MainSubtitle, so writing it where MainPicture sat makes the
    /// AssetList schema-invalid.
    #[test]
    fn stereoscopic_picture_follows_the_429_7_track_elements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let mut reel = stereo_reel();
        reel.sound_id = Some("snd1".into());
        reel.sound_edit_rate_num = 24;
        reel.sound_edit_rate_den = 1;
        reel.sound_duration = 240;
        reel.subtitle_id = Some("sub1".into());
        reel.subtitle_edit_rate_num = 24;
        reel.subtitle_edit_rate_den = 1;
        reel.subtitle_duration = 240;
        let config = CplConfig {
            title: "3D Order".into(),
            content_kind: "feature".into(),
            reels: vec![reel],
            standard: crate::Standard::Smpte,
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();
        let stereo = xml.find("<msp-cpl:MainStereoscopicPicture").unwrap();
        assert!(xml.find("</MainSound>").unwrap() < stereo, "{xml}");
        assert!(xml.find("</MainSubtitle>").unwrap() < stereo, "{xml}");
        assert!(stereo < xml.find("</AssetList>").unwrap(), "{xml}");
    }

    /// A stereoscopic picture is a track file like any other, so the servers that
    /// hash-check against the CPL need its `Hash` there: after Duration/KeyId and
    /// before FrameRate, the 429-7 order 429-10 reuses.
    #[test]
    fn stereoscopic_picture_carries_its_hash_in_the_cpl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CPL.xml");
        let mut reel = stereo_reel();
        reel.picture_hash = Some("cGljdHVyZS1oYXNo".into());
        reel.picture_key_id = Some("11111111-2222-3333-4444-555555555555".into());
        let config = CplConfig {
            title: "3D Hash".into(),
            content_kind: "feature".into(),
            reels: vec![reel],
            ..Default::default()
        };
        assert_eq!(generate_cpl(&config, "cpl1", &path), 0);
        let xml = std::fs::read_to_string(&path).unwrap();

        let picture = xml
            .split("<msp-cpl:MainStereoscopicPicture")
            .nth(1)
            .and_then(|s| s.split("</msp-cpl:MainStereoscopicPicture>").next())
            .expect("a MainStereoscopicPicture");
        assert!(
            picture.contains("<Hash>cGljdHVyZS1oYXNo</Hash>"),
            "{picture}"
        );
        assert!(
            picture.find("<KeyId>").unwrap() < picture.find("<Hash>").unwrap(),
            "Hash follows KeyId: {picture}"
        );
        assert!(
            picture.find("<Hash>").unwrap() < picture.find("<FrameRate>").unwrap(),
            "Hash precedes FrameRate: {picture}"
        );
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

    const ACCESSIBILITY_TRACK_CHANNELS: u32 = 8;
    const STEREO_CHANNELS: u32 = 2;
    const HEARING_IMPAIRED_CHANNEL: u32 = 6;
    const VISUALLY_IMPAIRED_CHANNEL: u32 = 7;
    const SILENCE_FRAMES: u64 = 24;
    const TEST_FRAME_RATE: u32 = 24;
    const DCP_SAMPLE_RATE: u32 = 48_000;

    fn silent_wav(dir: &Path, channels: u32) -> std::path::PathBuf {
        let wav = dir.join("silence.wav");
        crate::audio_fallback::write_silent_wav(&wav, channels, SILENCE_FRAMES, TEST_FRAME_RATE)
            .expect("write silence");
        wav
    }

    #[test]
    fn main_sound_from_track_file_reads_the_mca_labels() {
        let dir = tempfile::tempdir().unwrap();
        let wav = silent_wav(dir.path(), ACCESSIBILITY_TRACK_CHANNELS);
        let labels = crate::mxf_wrap::build_mca_config(
            ACCESSIBILITY_TRACK_CHANNELS,
            ACCESSIBILITY_TRACK_CHANNELS,
            Some(HEARING_IMPAIRED_CHANNEL),
            Some(VISUALLY_IMPAIRED_CHANNEL),
        )
        .expect("5.1 plus HI and VI-N labels");
        let mxf = dir.path().join("labelled.mxf");
        crate::mxf_wrap::wrap_mxf_files(
            vec![wav],
            &mxf,
            crate::mxf_wrap::MxfType::PcmAudio,
            TEST_FRAME_RATE,
            None,
            Some(postkit::mxf_wrap::McaConfig {
                labels,
                spoken_language: None,
            }),
            None,
        )
        .expect("wrap the labelled track");

        let sound = main_sound_from_track_file(&mxf).expect("read the sound track");
        // the channel count alone would call these eight channels 7.1
        assert_eq!(sound.configuration, "51/L,R,C,LFE,Ls,Rs,HI,VIN");
        assert_eq!(sound.sample_rate, DCP_SAMPLE_RATE);
    }

    #[test]
    fn main_sound_from_track_file_names_the_stereo_group_the_essence_omits() {
        let dir = tempfile::tempdir().unwrap();
        let wav = silent_wav(dir.path(), STEREO_CHANNELS);
        let mxf = dir.path().join("stereo.mxf");
        crate::mxf_wrap::wrap_mxf_files(
            vec![wav],
            &mxf,
            crate::mxf_wrap::MxfType::PcmAudio,
            TEST_FRAME_RATE,
            None,
            None,
            None,
        )
        .expect("wrap the stereo track");

        let sound = main_sound_from_track_file(&mxf).expect("read the sound track");
        assert_eq!(sound.configuration, "20/L,R");
    }

    #[test]
    fn main_sound_from_track_file_falls_back_to_the_channel_count() {
        let dir = tempfile::tempdir().unwrap();
        let wav = silent_wav(dir.path(), ACCESSIBILITY_TRACK_CHANNELS);
        let mxf = dir.path().join("unlabelled.mxf");
        let result = postkit::mxf_wrap::mxf_wrap(&postkit::mxf_wrap::MxfWrapOptions {
            input_files: vec![wav],
            output: mxf.clone(),
            essence_type: postkit::mxf_wrap::EssenceType::Pcm,
            standard: postkit::mxf_wrap::MxfStandard::AsDcp,
            fps_num: TEST_FRAME_RATE,
            fps_den: 1,
            partition_size: 0,
            encryption: None,
            mca_config: None,
            resource_ids: vec![],
            hdr: None,
            asset_uuid: None,
            timed_text_duration_frames: None,
        });
        assert!(result.success, "wrap failed: {}", result.error);

        let sound = main_sound_from_track_file(&mxf).expect("read the sound track");
        assert_eq!(sound.configuration, "71/L,R,C,LFE,Lss,Rss,Lrs,Rrs");
        assert_eq!(sound.sample_rate, DCP_SAMPLE_RATE);
    }

    #[test]
    fn main_sound_from_track_file_names_the_file_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent.mxf");
        let error = main_sound_from_track_file(&missing).expect_err("a missing track must fail");
        assert!(error.contains("absent.mxf"), "{error}");
    }
}
