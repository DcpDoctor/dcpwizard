//! ISDCF Digital Cinema Naming Convention content titles, ported from
//! DCP-o-matic's `Film::isdcf_name`.

use crate::cpl::{Luminance, LuminanceUnits};
use crate::{ContentType, Resolution, Standard};
use serde::{Deserialize, Serialize};

const TITLE_LENGTH_LIMIT: usize = 14;
const TITLE_WORD_SEPARATORS: [char; 2] = [' ', '_'];
const STUDIO_LETTERS: usize = 4;
const STUDIO_MINIMUM_LETTERS: usize = 2;
const FACILITY_LETTERS: usize = 3;
const UNSPECIFIED_LANGUAGE: &str = "XX";
const STANDARD_FRAME_RATE: u32 = 24;
const DEFAULT_VERSION_NUMBER: u32 = 1;
/// Flat, the shape the convention assumes when nothing declares a container.
pub const DEFAULT_CONTAINER_SIZE: (u32, u32) = (1998, 1080);
const CANDELA_PER_SQUARE_METRE_PER_FOOT_LAMBERT: f64 = 3.426;
const ASPECT_TOLERANCE: f32 = 0.01;

/// Aspect labels and the reference container sizes they are measured from,
/// taken from DCP-o-matic's ratio list.
const ASPECT_LABELS: &[(u32, u32, &str)] = &[
    (1290, 1080, "119"),
    (1350, 1080, "125"),
    (1440, 1080, "133"),
    (1485, 1080, "137"),
    (1544, 1080, "143"),
    (1620, 1080, "150"),
    (1800, 1080, "166"),
    (1920, 1080, "178"),
    (1998, 1080, "F"),
    (1716, 858, "200"),
    (2048, 926, "221"),
    (2048, 872, "S"),
    (2048, 858, "S"),
    (2048, 1080, "C"),
];

/// RFC 5646 tag to DCNC spelling, vendored from libdcp's `tags/dcnc`.
const DCNC_LANGUAGES: &[(&str, &str)] = &[
    ("hy", "HY"),
    ("sq", "SQ"),
    ("ar", "AR"),
    ("bs", "BS"),
    ("bg", "BG"),
    ("ca", "CA"),
    ("yue", "YUE"),
    ("cmn", "CMN"),
    ("cmn-Hans", "QMS"),
    ("cmn-Hant", "QMT"),
    ("nan", "NAN"),
    ("cmn-TW", "QTM"),
    ("hr", "HR"),
    ("cs", "CS"),
    ("da", "DA"),
    ("nl", "NL"),
    ("en", "EN"),
    ("et", "ET"),
    ("eu", "EU"),
    ("fi", "FI"),
    ("nl-BE", "VLS"),
    ("fr", "FR"),
    ("gl", "GL"),
    ("fr-CA", "QFC"),
    ("de", "DE"),
    ("gsw", "GSW"),
    ("el", "EL"),
    ("he", "HE"),
    ("hi", "HI"),
    ("hu", "HU"),
    ("is", "IS"),
    ("id", "IND"),
    ("it", "IT"),
    ("ja", "JA"),
    ("kk", "KK"),
    ("km", "KM"),
    ("ko", "KO"),
    ("ky", "KG"),
    ("lv", "LV"),
    ("lt", "LT"),
    ("ms", "MSA"),
    ("mr", "MR"),
    ("mn", "MN"),
    ("no", "NO"),
    ("pl", "PL"),
    ("pt-BR", "QBP"),
    ("pt", "PT"),
    ("ro", "RO"),
    ("ru", "RU"),
    ("sr", "SR"),
    ("sk", "SK"),
    ("sl", "SL"),
    ("es-AR", "QSA"),
    ("es", "ES"),
    ("es-419", "LAS"),
    ("es-MX", "QSM"),
    ("sv", "SV"),
    ("ta", "TA"),
    ("te", "TE"),
    ("th", "TH"),
    ("tr", "TR"),
    ("uk", "UK"),
    ("ur", "UR"),
    ("vi", "VI"),
    ("cy", "WEL"),
];

/// Latin-1 Supplement and Latin Extended-A letters that lose their accent, the
/// part of DCP-o-matic's ICU transliteration that reaches the title. Anything
/// else outside the allowed set is dropped rather than folded.
const ACCENTED_LETTERS: &[(char, char)] = &[
    ('À', 'A'),
    ('Á', 'A'),
    ('Â', 'A'),
    ('Ã', 'A'),
    ('Ä', 'A'),
    ('Å', 'A'),
    ('Ç', 'C'),
    ('È', 'E'),
    ('É', 'E'),
    ('Ê', 'E'),
    ('Ë', 'E'),
    ('Ì', 'I'),
    ('Í', 'I'),
    ('Î', 'I'),
    ('Ï', 'I'),
    ('Ñ', 'N'),
    ('Ò', 'O'),
    ('Ó', 'O'),
    ('Ô', 'O'),
    ('Õ', 'O'),
    ('Ö', 'O'),
    ('Ù', 'U'),
    ('Ú', 'U'),
    ('Û', 'U'),
    ('Ü', 'U'),
    ('Ý', 'Y'),
    ('à', 'a'),
    ('á', 'a'),
    ('â', 'a'),
    ('ã', 'a'),
    ('ä', 'a'),
    ('å', 'a'),
    ('ç', 'c'),
    ('è', 'e'),
    ('é', 'e'),
    ('ê', 'e'),
    ('ë', 'e'),
    ('ì', 'i'),
    ('í', 'i'),
    ('î', 'i'),
    ('ï', 'i'),
    ('ñ', 'n'),
    ('ò', 'o'),
    ('ó', 'o'),
    ('ô', 'o'),
    ('õ', 'o'),
    ('ö', 'o'),
    ('ù', 'u'),
    ('ú', 'u'),
    ('û', 'u'),
    ('ü', 'u'),
    ('ý', 'y'),
    ('ÿ', 'y'),
    ('Ā', 'A'),
    ('ā', 'a'),
    ('Ă', 'A'),
    ('ă', 'a'),
    ('Ą', 'A'),
    ('ą', 'a'),
    ('Ć', 'C'),
    ('ć', 'c'),
    ('Ĉ', 'C'),
    ('ĉ', 'c'),
    ('Ċ', 'C'),
    ('ċ', 'c'),
    ('Č', 'C'),
    ('č', 'c'),
    ('Ď', 'D'),
    ('ď', 'd'),
    ('Ē', 'E'),
    ('ē', 'e'),
    ('Ĕ', 'E'),
    ('ĕ', 'e'),
    ('Ė', 'E'),
    ('ė', 'e'),
    ('Ę', 'E'),
    ('ę', 'e'),
    ('Ě', 'E'),
    ('ě', 'e'),
    ('Ĝ', 'G'),
    ('ĝ', 'g'),
    ('Ğ', 'G'),
    ('ğ', 'g'),
    ('Ġ', 'G'),
    ('ġ', 'g'),
    ('Ģ', 'G'),
    ('ģ', 'g'),
    ('Ĥ', 'H'),
    ('ĥ', 'h'),
    ('Ĩ', 'I'),
    ('ĩ', 'i'),
    ('Ī', 'I'),
    ('ī', 'i'),
    ('Ĭ', 'I'),
    ('ĭ', 'i'),
    ('Į', 'I'),
    ('į', 'i'),
    ('İ', 'I'),
    ('Ĵ', 'J'),
    ('ĵ', 'j'),
    ('Ķ', 'K'),
    ('ķ', 'k'),
    ('Ĺ', 'L'),
    ('ĺ', 'l'),
    ('Ļ', 'L'),
    ('ļ', 'l'),
    ('Ľ', 'L'),
    ('ľ', 'l'),
    ('Ł', 'L'),
    ('ł', 'l'),
    ('Ń', 'N'),
    ('ń', 'n'),
    ('Ņ', 'N'),
    ('ņ', 'n'),
    ('Ň', 'N'),
    ('ň', 'n'),
    ('Ō', 'O'),
    ('ō', 'o'),
    ('Ŏ', 'O'),
    ('ŏ', 'o'),
    ('Ő', 'O'),
    ('ő', 'o'),
    ('Ŕ', 'R'),
    ('ŕ', 'r'),
    ('Ŗ', 'R'),
    ('ŗ', 'r'),
    ('Ř', 'R'),
    ('ř', 'r'),
    ('Ś', 'S'),
    ('ś', 's'),
    ('Ŝ', 'S'),
    ('ŝ', 's'),
    ('Ş', 'S'),
    ('ş', 's'),
    ('Š', 'S'),
    ('š', 's'),
    ('Ţ', 'T'),
    ('ţ', 't'),
    ('Ť', 'T'),
    ('ť', 't'),
    ('Ũ', 'U'),
    ('ũ', 'u'),
    ('Ū', 'U'),
    ('ū', 'u'),
    ('Ŭ', 'U'),
    ('ŭ', 'u'),
    ('Ů', 'U'),
    ('ů', 'u'),
    ('Ű', 'U'),
    ('ű', 'u'),
    ('Ų', 'U'),
    ('ų', 'u'),
    ('Ŵ', 'W'),
    ('ŵ', 'w'),
    ('Ŷ', 'Y'),
    ('ŷ', 'y'),
    ('Ÿ', 'Y'),
    ('Ź', 'Z'),
    ('ź', 'z'),
    ('Ż', 'Z'),
    ('ż', 'z'),
    ('Ž', 'Z'),
    ('ž', 'z'),
];

/// A rating from one certification agency, e.g. BBFC / PG.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rating {
    pub agency: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerritoryType {
    #[default]
    Specific,
    InternationalTexted,
    InternationalTextless,
}

/// Subtitles and captions carry the same languages but different name tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextKind {
    #[default]
    Subtitle,
    Caption,
}

/// A soundtrack channel present in the DCP, in the spelling the name's channel
/// digits are counted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundtrackChannel {
    Left,
    Right,
    Centre,
    Lfe,
    LeftSurround,
    RightSurround,
    LeftCentre,
    RightCentre,
    BackSurroundLeft,
    BackSurroundRight,
}

impl SoundtrackChannel {
    fn counts_as_soundtrack(self) -> bool {
        matches!(
            self,
            Self::Left
                | Self::Right
                | Self::Centre
                | Self::LeftSurround
                | Self::RightSurround
                | Self::BackSurroundLeft
                | Self::BackSurroundRight
        )
    }
}

/// Creation date, appended as YYYYMMDD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsdcfDate {
    pub year: u32,
    pub month: u32,
    pub day: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsdcfNameInput {
    pub title: String,
    pub content_type: ContentType,
    /// Version for a SMPTE package. Interop packages use `content_versions`.
    pub version_number: u32,
    pub content_versions: Vec<String>,
    pub temp_version: bool,
    pub pre_release: bool,
    pub red_band: bool,
    pub chain: Option<String>,
    pub three_d: bool,
    pub two_d_version_of_three_d: bool,
    pub luminance: Option<Luminance>,
    pub frame_rate: u32,
    pub container_size: (u32, u32),
    /// Picture area inside the container, when it differs from the container.
    pub active_picture_size: Option<(u32, u32)>,
    pub audio_language: Option<String>,
    pub open_text_languages: Vec<String>,
    pub open_text_kind: TextKind,
    pub open_text_burnt_in: bool,
    pub closed_text_languages: Vec<String>,
    pub closed_text_kind: TextKind,
    pub territory_type: TerritoryType,
    pub release_territory: Option<String>,
    pub ratings: Vec<Rating>,
    pub soundtrack_channels: Vec<SoundtrackChannel>,
    pub has_hearing_impaired: bool,
    pub has_visually_impaired: bool,
    pub has_atmos: bool,
    pub resolution: Resolution,
    pub studio: Option<String>,
    pub date: Option<IsdcfDate>,
    pub facility: Option<String>,
    pub standard: Standard,
    pub version_file: bool,
}

impl Default for IsdcfNameInput {
    fn default() -> Self {
        Self {
            title: String::new(),
            content_type: ContentType::default(),
            version_number: DEFAULT_VERSION_NUMBER,
            content_versions: Vec::new(),
            temp_version: false,
            pre_release: false,
            red_band: false,
            chain: None,
            three_d: false,
            two_d_version_of_three_d: false,
            luminance: None,
            frame_rate: STANDARD_FRAME_RATE,
            container_size: DEFAULT_CONTAINER_SIZE,
            active_picture_size: None,
            audio_language: None,
            open_text_languages: Vec::new(),
            open_text_kind: TextKind::default(),
            open_text_burnt_in: false,
            closed_text_languages: Vec::new(),
            closed_text_kind: TextKind::default(),
            territory_type: TerritoryType::default(),
            release_territory: None,
            ratings: Vec::new(),
            soundtrack_channels: Vec::new(),
            has_hearing_impaired: false,
            has_visually_impaired: false,
            has_atmos: false,
            resolution: Resolution::default(),
            studio: None,
            date: None,
            facility: None,
            standard: Standard::default(),
            version_file: false,
        }
    }
}

/// The DCNC spelling of one RFC 5646 tag, upper case. Tags outside libdcp's
/// table fall back to their primary subtag.
pub fn dcnc_language(tag: &str) -> String {
    for (rfc_5646, dcnc) in DCNC_LANGUAGES {
        if tag.eq_ignore_ascii_case(rfc_5646) {
            return dcnc.to_string();
        }
    }

    let primary_subtag = tag.split('-').next().unwrap_or_default();
    if primary_subtag.is_empty() {
        return UNSPECIFIED_LANGUAGE.to_string();
    }

    primary_subtag.to_uppercase()
}

/// The ISDCF content title for a DCP.
pub fn isdcf_name(input: &IsdcfNameInput) -> String {
    let mut name = mangled_title(&input.title);

    name += &format!(
        "_{}-{}",
        content_type_label(input.content_type),
        version(input)
    );

    if input.temp_version {
        name += "-Temp";
    }
    if input.pre_release {
        name += "-Pre";
    }
    if input.red_band {
        name += "-RedBand";
    }
    if let Some(chain) = input.chain.as_ref().filter(|chain| !chain.is_empty()) {
        name += &format!("-{chain}");
    }
    if input.three_d {
        name += "-3D";
    }
    if input.two_d_version_of_three_d {
        name += "-2D";
    }
    if let Some(luminance) = &input.luminance {
        name += &format!("-{}fl", (foot_lamberts(luminance) * 10.0).round());
    }
    if input.frame_rate != STANDARD_FRAME_RATE {
        name += &format!("-{}", input.frame_rate);
    }

    name += &format!("_{}", aspect_label(input.container_size));
    if let Some(active) = active_aspect(input) {
        name += &format!("-{active}");
    }

    name += &format!("_{}", audio_language(input));
    name += &text_languages(input);
    name += &territory(input);
    name += &audio_channels(input);

    if input.has_atmos {
        name += "-IAB";
    }

    name += &format!("_{}", resolution_label(input.resolution));

    if let Some(studio) = input
        .studio
        .as_ref()
        .filter(|studio| studio.chars().count() >= STUDIO_MINIMUM_LETTERS)
    {
        name += &format!("_{}", first_letters(studio, STUDIO_LETTERS));
    }

    if let Some(date) = input.date {
        name += &format!("_{:04}{:02}{:02}", date.year, date.month, date.day);
    }

    if let Some(facility) = input
        .facility
        .as_ref()
        .filter(|facility| facility.chars().count() >= FACILITY_LETTERS)
    {
        name += &format!("_{}", first_letters(facility, FACILITY_LETTERS));
    }

    name += match input.standard {
        Standard::Interop => "_IOP",
        Standard::Smpte => "_SMPTE",
    };

    if input.three_d {
        name += "-3D";
    }

    name += if input.version_file { "_VF" } else { "_OV" };

    name
}

fn content_type_label(content_type: ContentType) -> &'static str {
    match content_type {
        ContentType::Feature => "FTR",
        ContentType::Short => "SHR",
        ContentType::Trailer => "TLR",
        ContentType::Test => "TST",
        ContentType::Transitional => "XSN",
        ContentType::Rating => "RTG",
        ContentType::Teaser => "TSR",
        ContentType::Policy => "POL",
        ContentType::PublicServiceAnnouncement => "PSA",
        ContentType::Advertisement => "ADV",
        ContentType::Episode => "EPS",
    }
}

fn resolution_label(resolution: Resolution) -> &'static str {
    match resolution {
        Resolution::TwoK => "2K",
        Resolution::FourK => "4K",
    }
}

fn version(input: &IsdcfNameInput) -> String {
    if input.standard == Standard::Smpte {
        return input.version_number.to_string();
    }

    let numeric = input
        .content_versions
        .first()
        .filter(|content_version| {
            !content_version.is_empty() && content_version.chars().all(|c| c.is_ascii_digit())
        })
        .cloned();

    numeric.unwrap_or_else(|| DEFAULT_VERSION_NUMBER.to_string())
}

fn foot_lamberts(luminance: &Luminance) -> f64 {
    match luminance.units {
        LuminanceUnits::FootLambert => luminance.value,
        LuminanceUnits::CandelaPerSquareMetre => {
            luminance.value / CANDELA_PER_SQUARE_METRE_PER_FOOT_LAMBERT
        }
    }
}

fn ratio(size: (u32, u32)) -> f32 {
    if size.1 == 0 {
        return 0.0;
    }

    size.0 as f32 / size.1 as f32
}

fn hundredths_of_ratio(size: (u32, u32)) -> i64 {
    (ratio(size) * 100.0).round() as i64
}

fn aspect_label(size: (u32, u32)) -> &'static str {
    let wanted = ratio(size);

    let exact = ASPECT_LABELS
        .iter()
        .find(|(width, height, _)| (ratio((*width, *height)) - wanted).abs() <= ASPECT_TOLERANCE);
    if let Some((_, _, label)) = exact {
        return label;
    }

    let mut nearest = ASPECT_LABELS[0];
    for candidate in ASPECT_LABELS {
        let distance = (ratio((candidate.0, candidate.1)) - wanted).abs();
        if distance < (ratio((nearest.0, nearest.1)) - wanted).abs() {
            nearest = *candidate;
        }
    }

    nearest.2
}

/// The interior aspect, shown only when it differs from the container. The
/// convention leaves it off trailers.
fn active_aspect(input: &IsdcfNameInput) -> Option<i64> {
    if input.content_type == ContentType::Trailer {
        return None;
    }

    let active = hundredths_of_ratio(input.active_picture_size?);
    if active == hundredths_of_ratio(input.container_size) {
        return None;
    }

    Some(active)
}

fn audio_language(input: &IsdcfNameInput) -> String {
    input
        .audio_language
        .as_ref()
        .map(|tag| dcnc_language(tag))
        .unwrap_or_else(|| UNSPECIFIED_LANGUAGE.to_string())
}

fn text_languages(input: &IsdcfNameInput) -> String {
    if let Some(tag) = input.open_text_languages.first() {
        let spelling = dcnc_language(tag);
        let spelling = if input.open_text_burnt_in {
            spelling.to_lowercase()
        } else {
            spelling
        };
        let caption = match input.open_text_kind {
            TextKind::Caption => "-OCAP",
            TextKind::Subtitle => "",
        };
        return format!("-{spelling}{caption}");
    }

    if let Some(tag) = input.closed_text_languages.first() {
        let caption = match input.closed_text_kind {
            TextKind::Caption => "-CCAP",
            TextKind::Subtitle => "",
        };
        return format!("-{}{caption}", dcnc_language(tag));
    }

    format!("-{UNSPECIFIED_LANGUAGE}")
}

fn territory(input: &IsdcfNameInput) -> String {
    match input.territory_type {
        TerritoryType::InternationalTexted => return "_INT-TD".to_string(),
        TerritoryType::InternationalTextless => return "_INT-TL".to_string(),
        TerritoryType::Specific => {}
    }

    let Some(territory) = &input.release_territory else {
        return String::new();
    };

    let mut result = format!("_{}", territory.to_uppercase());
    if let Some(rating) = input.ratings.first() {
        let label: String = rating
            .label
            .chars()
            .filter(|c| *c != '+' && *c != '-')
            .collect();
        result += &format!("-{label}");
    }

    result
}

fn audio_channels(input: &IsdcfNameInput) -> String {
    let mut distinct: Vec<SoundtrackChannel> = Vec::new();
    for channel in &input.soundtrack_channels {
        if !distinct.contains(channel) {
            distinct.push(*channel);
        }
    }

    let soundtrack = distinct
        .iter()
        .filter(|channel| channel.counts_as_soundtrack())
        .count();
    let low_frequency = distinct
        .iter()
        .filter(|channel| **channel == SoundtrackChannel::Lfe)
        .count();

    let mut result = String::new();
    if soundtrack == 0 && low_frequency == 0 {
        result += "_MOS";
    } else if soundtrack > 0 {
        result += &format!("_{soundtrack}{low_frequency}");
    }

    if input.has_hearing_impaired {
        result += "-HI";
    }
    if input.has_visually_impaired {
        result += "-VI";
    }

    result
}

fn first_letters(text: &str, letters: usize) -> String {
    text.chars()
        .take(letters)
        .collect::<String>()
        .to_uppercase()
}

fn mangled_title(title: &str) -> String {
    let mut words = String::new();

    for word in title.split(TITLE_WORD_SEPARATORS) {
        let mut letters: Vec<char> = word.chars().collect();
        let Some(first) = letters.first_mut() else {
            continue;
        };
        *first = first.to_ascii_uppercase();

        let capitals = letters.iter().filter(|c| c.is_ascii_uppercase()).count();
        if capitals == letters.len() {
            for letter in letters.iter_mut().skip(1) {
                *letter = letter.to_ascii_lowercase();
            }
        }

        words.extend(letters);
    }

    words
        .chars()
        .map(unaccented)
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(TITLE_LENGTH_LIMIT)
        .collect()
}

fn unaccented(letter: char) -> char {
    ACCENTED_LETTERS
        .iter()
        .find(|(accented, _)| *accented == letter)
        .map(|(_, plain)| *plain)
        .unwrap_or(letter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> IsdcfNameInput {
        IsdcfNameInput {
            title: "My Nice Film".to_string(),
            content_type: ContentType::Feature,
            container_size: (1998, 1080),
            date: Some(IsdcfDate {
                year: 2014,
                month: 7,
                day: 4,
            }),
            audio_language: Some("en-US".to_string()),
            content_versions: vec!["1".to_string()],
            release_territory: Some("GB".to_string()),
            ratings: vec![Rating {
                agency: "BBFC".to_string(),
                label: "PG".to_string(),
            }],
            studio: Some("ST".to_string()),
            facility: Some("FAC".to_string()),
            standard: Standard::Interop,
            soundtrack_channels: vec![SoundtrackChannel::Centre],
            ..Default::default()
        }
    }

    /// The long-name film DCP-o-matic's test builds up from the basic one.
    fn long_name() -> IsdcfNameInput {
        IsdcfNameInput {
            title: "My Nice Film With A Very Long Name".to_string(),
            content_type: ContentType::Trailer,
            container_size: (2048, 858),
            resolution: Resolution::FourK,
            open_text_languages: vec!["fr-FR".to_string()],
            open_text_burnt_in: true,
            version_number: 2,
            release_territory: Some("US".to_string()),
            ratings: vec![Rating {
                agency: "MPA".to_string(),
                label: "R".to_string(),
            }],
            studio: Some("di".to_string()),
            facility: Some("ppfacility".to_string()),
            audio_language: Some("de-DE".to_string()),
            standard: Standard::Smpte,
            soundtrack_channels: vec![],
            ..base()
        }
    }

    #[test]
    fn basic_name() {
        assert_eq!(
            isdcf_name(&base()),
            "MyNiceFilm_FTR-1_F_EN-XX_GB-PG_10_2K_ST_20140704_FAC_IOP_OV"
        );
    }

    #[test]
    fn no_audio_language_writes_xx() {
        let input = IsdcfNameInput {
            audio_language: None,
            ..base()
        };
        assert_eq!(
            isdcf_name(&input),
            "MyNiceFilm_FTR-1_F_XX-XX_GB-PG_10_2K_ST_20140704_FAC_IOP_OV"
        );
    }

    #[test]
    fn long_name_is_truncated() {
        assert_eq!(
            isdcf_name(&long_name()),
            "MyNiceFilmWith_TLR-2_S_DE-fr_US-R_MOS_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn open_captions_are_marked() {
        let input = IsdcfNameInput {
            open_text_kind: TextKind::Caption,
            ..long_name()
        };
        assert_eq!(
            isdcf_name(&input),
            "MyNiceFilmWith_TLR-2_S_DE-fr-OCAP_US-R_MOS_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn rating_punctuation_is_stripped() {
        let input = IsdcfNameInput {
            ratings: vec![Rating {
                agency: "RARS".to_string(),
                label: "6+".to_string(),
            }],
            ..long_name()
        };
        assert_eq!(
            isdcf_name(&input),
            "MyNiceFilmWith_TLR-2_S_DE-fr_US-6_MOS_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn no_rating_writes_nothing() {
        let input = IsdcfNameInput {
            ratings: vec![],
            ..long_name()
        };
        assert_eq!(
            isdcf_name(&input),
            "MyNiceFilmWith_TLR-2_S_DE-fr_US_MOS_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn interior_aspect_is_hidden_for_trailers() {
        let input = IsdcfNameInput {
            container_size: (1998, 1080),
            active_picture_size: Some((1436, 1080)),
            ..long_name()
        };
        assert_eq!(
            isdcf_name(&input),
            "MyNiceFilmWith_TLR-2_F_DE-fr_US-R_MOS_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn interior_aspect_is_shown_for_other_content() {
        let input = IsdcfNameInput {
            content_type: ContentType::Transitional,
            container_size: (1998, 1080),
            active_picture_size: Some((1436, 1080)),
            ..long_name()
        };
        assert_eq!(
            isdcf_name(&input),
            "MyNiceFilmWith_XSN-2_F-133_DE-fr_US-R_MOS_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn interior_aspect_is_always_numeric() {
        let sizes = [
            ((1998, 836), "239"),
            ((1998, 1052), "190"),
            ((1998, 908), "220"),
            ((1998, 1025), "195"),
        ];

        for (size, expected) in sizes {
            let input = IsdcfNameInput {
                content_type: ContentType::Transitional,
                container_size: (1998, 1080),
                active_picture_size: Some(size),
                ..long_name()
            };
            assert_eq!(
                isdcf_name(&input),
                format!(
                    "MyNiceFilmWith_XSN-2_F-{expected}_DE-fr_US-R_MOS_4K_DI_20140704_PPF_SMPTE_OV"
                )
            );
        }
    }

    fn transitional() -> IsdcfNameInput {
        IsdcfNameInput {
            content_type: ContentType::Transitional,
            container_size: (1998, 1080),
            active_picture_size: Some((1436, 1080)),
            ..long_name()
        }
    }

    #[test]
    fn three_d_is_marked_twice() {
        let input = IsdcfNameInput {
            three_d: true,
            ..transitional()
        };
        assert_eq!(
            isdcf_name(&input),
            "MyNiceFilmWith_XSN-2-3D_F-133_DE-fr_US-R_MOS_4K_DI_20140704_PPF_SMPTE-3D_OV"
        );
    }

    #[test]
    fn content_type_modifiers() {
        let input = IsdcfNameInput {
            temp_version: true,
            pre_release: true,
            red_band: true,
            two_d_version_of_three_d: true,
            chain: Some("MyChain".to_string()),
            luminance: Some(Luminance {
                value: 4.5,
                units: LuminanceUnits::FootLambert,
            }),
            frame_rate: 48,
            ..transitional()
        };
        assert_eq!(
            isdcf_name(&input),
            "MyNiceFilmWith_XSN-2-Temp-Pre-RedBand-MyChain-2D-45fl-48_F-133_DE-fr_US-R_MOS_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn title_already_in_camel_case() {
        let input = IsdcfNameInput {
            title: "IKnowCamels".to_string(),
            ..transitional()
        };
        assert_eq!(
            isdcf_name(&input),
            "IKnowCamels_XSN-2_F-133_DE-fr_US-R_MOS_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn title_in_capitals() {
        for title in ["LIKE SHOUTING", "LIKE_SHOUTING"] {
            let input = IsdcfNameInput {
                title: title.to_string(),
                ..transitional()
            };
            assert_eq!(
                isdcf_name(&input),
                "LikeShouting_XSN-2_F-133_DE-fr_US-R_MOS_4K_DI_20140704_PPF_SMPTE_OV"
            );
        }
    }

    #[test]
    fn title_with_hyphens_is_left_alone() {
        let input = IsdcfNameInput {
            title: "LIKE-SHOUTING".to_string(),
            ..transitional()
        };
        assert_eq!(
            isdcf_name(&input),
            "LIKE-SHOUTING_XSN-2_F-133_DE-fr_US-R_MOS_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    fn shouting(channels: Vec<SoundtrackChannel>) -> IsdcfNameInput {
        IsdcfNameInput {
            title: "LIKE_SHOUTING".to_string(),
            soundtrack_channels: channels,
            ..transitional()
        }
    }

    #[test]
    fn audio_channel_markup() {
        use SoundtrackChannel::*;

        let cases = [
            (vec![Centre], "10"),
            (vec![Centre, Left], "20"),
            (vec![Centre, Left, Right], "30"),
            (vec![Centre, Left, Right, Lfe], "31"),
            (vec![Centre, Left, Right, Lfe, LeftSurround], "41"),
            (
                vec![Centre, Left, Right, Lfe, LeftSurround, RightSurround],
                "51",
            ),
            (
                vec![
                    Centre,
                    Left,
                    Right,
                    Lfe,
                    LeftSurround,
                    RightSurround,
                    BackSurroundLeft,
                    BackSurroundRight,
                ],
                "71",
            ),
        ];

        for (channels, expected) in cases {
            assert_eq!(
                isdcf_name(&shouting(channels)),
                format!(
                    "LikeShouting_XSN-2_F-133_DE-fr_US-R_{expected}_4K_DI_20140704_PPF_SMPTE_OV"
                )
            );
        }
    }

    #[test]
    fn accessibility_channels() {
        use SoundtrackChannel::*;

        let surround = vec![Centre, Left, Right, Lfe, LeftSurround, RightSurround];
        let input = IsdcfNameInput {
            has_hearing_impaired: true,
            ..shouting(surround.clone())
        };
        assert_eq!(
            isdcf_name(&input),
            "LikeShouting_XSN-2_F-133_DE-fr_US-R_51-HI_4K_DI_20140704_PPF_SMPTE_OV"
        );

        let input = IsdcfNameInput {
            has_hearing_impaired: true,
            has_visually_impaired: true,
            ..shouting(surround)
        };
        assert_eq!(
            isdcf_name(&input),
            "LikeShouting_XSN-2_F-133_DE-fr_US-R_51-HI-VI_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    #[test]
    fn dcnc_spelling_beats_the_language_subtag() {
        use SoundtrackChannel::*;

        let input = IsdcfNameInput {
            audio_language: Some("pt-BR".to_string()),
            has_hearing_impaired: true,
            has_visually_impaired: true,
            ..shouting(vec![
                Centre,
                Left,
                Right,
                Lfe,
                LeftSurround,
                RightSurround,
                BackSurroundLeft,
                BackSurroundRight,
            ])
        };
        assert_eq!(
            isdcf_name(&input),
            "LikeShouting_XSN-2_F-133_QBP-fr_US-R_71-HI-VI_4K_DI_20140704_PPF_SMPTE_OV"
        );
    }

    fn hello() -> IsdcfNameInput {
        IsdcfNameInput {
            title: "Hello".to_string(),
            content_type: ContentType::Test,
            container_size: (1998, 1080),
            date: Some(IsdcfDate {
                year: 2023,
                month: 1,
                day: 18,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn atmos_is_marked() {
        let input = IsdcfNameInput {
            has_atmos: true,
            ..hello()
        };
        assert_eq!(
            isdcf_name(&input),
            "Hello_TST-1_F_XX-XX_MOS-IAB_2K_20230118_SMPTE_OV"
        );
    }

    #[test]
    fn closed_captions_are_marked() {
        let input = IsdcfNameInput {
            closed_text_languages: vec!["de-DE".to_string()],
            closed_text_kind: TextKind::Caption,
            ..hello()
        };
        assert_eq!(
            isdcf_name(&input),
            "Hello_TST-1_F_XX-DE-CCAP_MOS_2K_20230118_SMPTE_OV"
        );
    }

    #[test]
    fn closed_subtitles_are_not_marked_as_captions() {
        let input = IsdcfNameInput {
            closed_text_languages: vec!["de-DE".to_string()],
            closed_text_kind: TextKind::Subtitle,
            ..hello()
        };
        assert_eq!(
            isdcf_name(&input),
            "Hello_TST-1_F_XX-DE_MOS_2K_20230118_SMPTE_OV"
        );
    }

    #[test]
    fn accents_are_removed_from_the_title() {
        let input = IsdcfNameInput {
            title: "BezüglichMeineKatze".to_string(),
            ..hello()
        };
        assert_eq!(
            isdcf_name(&input),
            "BezuglichMeine_TST-1_F_XX-XX_MOS_2K_20230118_SMPTE_OV"
        );
    }

    #[test]
    fn burnt_in_text_is_lower_case_and_open_text_upper() {
        let burnt = IsdcfNameInput {
            open_text_languages: vec!["pt-BR".to_string()],
            open_text_burnt_in: true,
            ..hello()
        };
        assert_eq!(
            isdcf_name(&burnt),
            "Hello_TST-1_F_XX-qbp_MOS_2K_20230118_SMPTE_OV"
        );

        let open = IsdcfNameInput {
            open_text_burnt_in: false,
            ..burnt
        };
        assert_eq!(
            isdcf_name(&open),
            "Hello_TST-1_F_XX-QBP_MOS_2K_20230118_SMPTE_OV"
        );
    }

    #[test]
    fn international_territories() {
        let texted = IsdcfNameInput {
            territory_type: TerritoryType::InternationalTexted,
            release_territory: Some("GB".to_string()),
            ..hello()
        };
        assert_eq!(
            isdcf_name(&texted),
            "Hello_TST-1_F_XX-XX_INT-TD_MOS_2K_20230118_SMPTE_OV"
        );

        let textless = IsdcfNameInput {
            territory_type: TerritoryType::InternationalTextless,
            ..texted
        };
        assert_eq!(
            isdcf_name(&textless),
            "Hello_TST-1_F_XX-XX_INT-TL_MOS_2K_20230118_SMPTE_OV"
        );
    }

    #[test]
    fn version_file_is_marked() {
        let input = IsdcfNameInput {
            version_file: true,
            ..hello()
        };
        assert_eq!(
            isdcf_name(&input),
            "Hello_TST-1_F_XX-XX_MOS_2K_20230118_SMPTE_VF"
        );
    }

    #[test]
    fn interop_version_comes_from_the_content_version() {
        let numeric = IsdcfNameInput {
            standard: Standard::Interop,
            content_versions: vec!["7".to_string()],
            version_number: 3,
            ..hello()
        };
        assert_eq!(
            isdcf_name(&numeric),
            "Hello_TST-7_F_XX-XX_MOS_2K_20230118_IOP_OV"
        );

        let not_numeric = IsdcfNameInput {
            content_versions: vec!["director's cut".to_string()],
            ..numeric
        };
        assert_eq!(
            isdcf_name(&not_numeric),
            "Hello_TST-1_F_XX-XX_MOS_2K_20230118_IOP_OV"
        );
    }

    #[test]
    fn language_falls_back_to_the_primary_subtag() {
        assert_eq!(dcnc_language("pt-BR"), "QBP");
        assert_eq!(dcnc_language("cmn-Hans"), "QMS");
        assert_eq!(dcnc_language("en-US"), "EN");
        assert_eq!(dcnc_language("fr-FR"), "FR");
        assert_eq!(dcnc_language("sw-KE"), "SW");
        assert_eq!(dcnc_language("mi"), "MI");
        assert_eq!(dcnc_language(""), "XX");
    }

    #[test]
    fn standard_container_aspect_labels() {
        assert_eq!(aspect_label((2048, 858)), "S");
        assert_eq!(aspect_label((1998, 1080)), "F");
        assert_eq!(aspect_label((2048, 1080)), "C");
        assert_eq!(aspect_label((4096, 1716)), "S");
        assert_eq!(aspect_label((3996, 2160)), "F");
        assert_eq!(aspect_label((4096, 2160)), "C");
    }
}
