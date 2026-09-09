use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

#[derive(Copy, Clone, Debug, ValueEnum)]
enum AccessibilityStandardArg {
    Cvaa,
    Eaa,
    Aoda,
    Ofcom,
}

impl From<AccessibilityStandardArg> for postkit::accessibility::AccessibilityStandard {
    fn from(arg: AccessibilityStandardArg) -> Self {
        match arg {
            AccessibilityStandardArg::Cvaa => Self::Cvaa,
            AccessibilityStandardArg::Eaa => Self::Eaa,
            AccessibilityStandardArg::Aoda => Self::Aoda,
            AccessibilityStandardArg::Ofcom => Self::Ofcom,
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum HdrSourceArg {
    Hdr10,
    Hlg,
    PqP3d65,
    DolbyVision,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum HdrTargetArg {
    Hdr10,
    Hlg,
    Sdr,
}

impl From<HdrTargetArg> for postkit::dolby_vision::HdrType {
    fn from(arg: HdrTargetArg) -> Self {
        match arg {
            HdrTargetArg::Hdr10 => Self::Hdr10,
            HdrTargetArg::Hlg => Self::Hlg,
            HdrTargetArg::Sdr => Self::Sdr,
        }
    }
}

impl From<HdrSourceArg> for dcpwizard_core::hdr::HdrSourceFormat {
    fn from(arg: HdrSourceArg) -> Self {
        match arg {
            HdrSourceArg::Hdr10 => Self::Hdr10,
            HdrSourceArg::Hlg => Self::Hlg,
            HdrSourceArg::PqP3d65 => Self::PqP3D65,
            HdrSourceArg::DolbyVision => Self::DolbyVision,
        }
    }
}

/// ST 429-16 composition identity, boxed into the Create variant.
#[derive(Args)]
struct CreateCompositionMetadata {
    /// Content type: FTR, SHR, TLR, TST, XSN, RTG, TSR, POL, PSA, ADV
    #[arg(long)]
    content_type: Option<String>,
    /// UN M.49 region code, or an RFC 5646 region subtag, the composition is
    /// released for (e.g. 826, GB). A numeric value declares the M.49 scope.
    #[arg(long)]
    release_territory: Option<String>,
    /// Composition version number. Defaults to 1, which Bv2.1 requires present.
    #[arg(long)]
    version_number: Option<u32>,
    /// Exhibition chain the composition was mastered for.
    #[arg(long)]
    chain: Option<String>,
    /// Distributor name for the composition metadata.
    #[arg(long)]
    distributor: Option<String>,
    /// Mastering facility name for the composition metadata.
    #[arg(long)]
    facility: Option<String>,
    /// Mastering luminance, e.g. "14 foot-lambert" or "48 candela-per-square-metre".
    #[arg(long)]
    luminance: Option<String>,
}

/// ISDCF naming plus the package metadata the name is built from, boxed into
/// the Create variant. Every flag but `--isdcf-name` is metadata in its own
/// right, so none of them needs the naming flag and it needs none of them.
#[derive(Args)]
struct CreateIsdcfNaming {
    /// Name the DCP by the ISDCF convention: --title is the human title the
    /// content title is built from, and the built name replaces it.
    #[arg(long)]
    isdcf_name: bool,
    /// RFC 5646 language the main soundtrack is spoken in (e.g. en, fr-CA)
    #[arg(long, value_name = "RFC5646")]
    audio_lang: Option<String>,
    /// Certification rating as AGENCY=LABEL (repeatable), e.g.
    /// "http://www.mpaa.org/2003-ratings=PG-13". Written into the CPL's RatingList.
    #[arg(long = "rating", value_name = "AGENCY=LABEL")]
    ratings: Vec<String>,
    /// Studio code for the name (first four letters are used)
    #[arg(long)]
    studio: Option<String>,
    /// Content version text (repeatable). The first is the CPL's ContentVersion
    /// LabelText, and an Interop name takes its version number from it.
    #[arg(long = "content-version", value_name = "TEXT")]
    content_versions: Vec<String>,
    /// Mark the name a temporary version
    #[arg(long)]
    temp_version: bool,
    /// Mark the name a pre-release
    #[arg(long)]
    pre_release: bool,
    /// Mark the name a red band (restricted) version
    #[arg(long)]
    red_band: bool,
    /// Mark the name the 2D version of a 3D release
    #[arg(long)]
    two_d_version_of_three_d: bool,
    /// Release territory kind for the name
    #[arg(
        long,
        default_value = "specific",
        value_parser = ["specific", "international-texted", "international-textless"]
    )]
    territory_type: String,
    /// Creation date in the name (default: today, UTC)
    #[arg(long, value_name = "YYYY-MM-DD")]
    isdcf_date: Option<String>,
    /// Name the package VF instead of OV
    #[arg(long)]
    version_file: bool,
}

/// The naming flags resolved: the name's own options, and the metadata that
/// lands in the package whether or not a name is built from it.
struct CreateNaming {
    options: dcpwizard_core::isdcf_title::IsdcfNamingOptions,
    ratings: Vec<dcpwizard_core::isdcf_name::Rating>,
    content_versions: Vec<String>,
    audio_language: Option<String>,
    replaces_title: bool,
}

/// An agency is a URI, so it carries colons and only the first '=' separates it
/// from the label.
/// The edit rate a job runs at when neither the flags nor the source name one.
const DEFAULT_FRAME_RATE: u32 = 24;
const RATING_SEPARATOR: char = '=';
const ISDCF_DATE_PARTS: usize = 3;

struct MasteringDisplay {
    red: (u16, u16),
    green: (u16, u16),
    blue: (u16, u16),
    white_point: (u16, u16),
    max_luminance: u32,
    min_luminance: u32,
}

// ST 2086 counts chromaticity in 0.00002 steps and luminance in 0.0001 cd/m²
const P3D65_MASTERING_DISPLAY: MasteringDisplay = MasteringDisplay {
    red: (34000, 16000),
    green: (13250, 34500),
    blue: (7500, 3000),
    white_point: (15635, 16450),
    max_luminance: 10_000_000,
    min_luminance: 1,
};

impl CreateIsdcfNaming {
    fn resolve(&self) -> Result<CreateNaming, String> {
        let territory_type = match self.territory_type.as_str() {
            "international-texted" => {
                dcpwizard_core::isdcf_name::TerritoryType::InternationalTexted
            }
            "international-textless" => {
                dcpwizard_core::isdcf_name::TerritoryType::InternationalTextless
            }
            _ => dcpwizard_core::isdcf_name::TerritoryType::Specific,
        };
        let mut ratings = Vec::new();
        for spec in &self.ratings {
            let (agency, label) = spec.split_once(RATING_SEPARATOR).ok_or_else(|| {
                format!(
                    "rating '{spec}' must be AGENCY=LABEL, e.g. \
                     http://www.mpaa.org/2003-ratings=PG-13"
                )
            })?;
            ratings.push(dcpwizard_core::isdcf_name::Rating {
                agency: agency.to_string(),
                label: label.to_string(),
            });
        }
        let date = match self.isdcf_date.as_deref() {
            Some(spec) => Some(parse_isdcf_date(spec)?),
            None => None,
        };
        Ok(CreateNaming {
            options: dcpwizard_core::isdcf_title::IsdcfNamingOptions {
                studio: self.studio.clone(),
                temp_version: self.temp_version,
                pre_release: self.pre_release,
                red_band: self.red_band,
                two_d_version_of_three_d: self.two_d_version_of_three_d,
                territory_type,
                date,
                version_file: self.version_file,
            },
            ratings,
            content_versions: self.content_versions.clone(),
            audio_language: self.audio_lang.clone(),
            replaces_title: self.isdcf_name,
        })
    }
}

fn parse_isdcf_date(spec: &str) -> Result<dcpwizard_core::isdcf_name::IsdcfDate, String> {
    let parts: Vec<&str> = spec.split('-').collect();
    let bad = || format!("date '{spec}' must be YYYY-MM-DD");
    if parts.len() != ISDCF_DATE_PARTS {
        return Err(bad());
    }
    let year: u32 = parts[0].parse().map_err(|_| bad())?;
    let month: u32 = parts[1].parse().map_err(|_| bad())?;
    let day: u32 = parts[2].parse().map_err(|_| bad())?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(bad());
    }
    Ok(dcpwizard_core::isdcf_name::IsdcfDate { year, month, day })
}

/// clap value parser for `--audio-channels`: a DCP sound layout, as a count.
fn parse_packaged_channels(value: &str) -> Result<u32, String> {
    let count: u32 = value
        .parse()
        .map_err(|_| format!("'{value}' is not a channel count"))?;
    dcpwizard_core::mxf_wrap::check_packaged_channel_count(count)?;
    Ok(count)
}

/// Replace the title with the ISDCF content title, when asked for. The channel
/// count comes from the packaged WAV, so this runs after the audio is prepared.
fn apply_isdcf_name(
    config: &mut dcpwizard_core::dcp::DcpConfig,
    naming: &CreateNaming,
    burnt_in_subtitle: bool,
) {
    if !naming.replaces_title {
        return;
    }
    let channel_count = match config.audio_path.as_deref() {
        Some(path) => match dcpwizard_core::mxf_wrap::wav_channels(path) {
            Ok(count) => count as usize,
            Err(e) => {
                tracing::error!("{e}");
                std::process::exit(1);
            }
        },
        None => 0,
    };
    let sound = dcpwizard_core::isdcf_title::soundtrack_summary(
        channel_count,
        config.hi_channel,
        config.vi_channel,
    );
    let name = dcpwizard_core::isdcf_title::isdcf_title(
        config,
        &naming.options,
        &sound,
        burnt_in_subtitle,
    );
    tracing::info!("ISDCF name: {name}");
    config.title = name;
}

/// W5 create-time audio + encode QoL options, boxed into the Create variant.
#[derive(Args)]
struct CreateAudioQol {
    /// Normalize audio to a loudness target before wrapping (dom#1382):
    /// leqm=<db> (ISO 21727 Leq(m)) or lufs=<value> (EBU R128 integrated).
    #[arg(long)]
    loudness_target: Option<String>,
    /// True-peak ceiling in dBTP for --loudness-target (default -1.0). The
    /// gain is refused loud if it would breach this.
    #[arg(long)]
    true_peak_ceiling: Option<f64>,
    /// Upmix stereo audio to 5.1 before wrapping (dom#921/#1080): variant
    /// a (band-split) or b (passthrough + delayed surrounds).
    #[arg(long, value_parser = ["a", "b"])]
    upmix: Option<String>,
    /// Fill the packaged sound track with silent channels up to this count
    /// (2, 6, 8 or 16). Without it a 5.1 source is widened to 16 and every
    /// other source is packaged at its own width.
    #[arg(long, value_parser = parse_packaged_channels)]
    audio_channels: Option<u32>,
    /// Route the --audio channels to DCP lanes: comma-separated IN:OUT or
    /// IN:OUT@GAIN, where IN is a 1-based source channel, OUT a lane name
    /// (L, R, C, LFE, Ls, Rs, Lc, Rc, BsL, BsR, HI, VI) or number, and GAIN is
    /// decibels. Runs before every other audio step.
    #[arg(long)]
    audio_map: Option<String>,
    /// Audio gain in dB, applied after any upmix and loudness pass.
    #[arg(long)]
    audio_gain: Option<f64>,
    /// Shift the sound against the picture by this many milliseconds: positive
    /// arrives later, negative earlier. The running time is unchanged, so the
    /// shift is made up with silence at the other end.
    #[arg(long, value_name = "MILLISECONDS", allow_negative_numbers = true)]
    audio_delay: Option<i64>,
    /// Fade the audio up from silence over this many seconds.
    #[arg(long)]
    audio_fade_in: Option<f64>,
    /// Fade the audio down to silence over this many seconds.
    #[arg(long)]
    audio_fade_out: Option<f64>,
    /// Fade the picture up from black over this many seconds.
    #[arg(long)]
    video_fade_in: Option<f64>,
    /// Fade the picture down to black over this many seconds.
    #[arg(long)]
    video_fade_out: Option<f64>,
    /// Wait until this wall-clock time before encoding (dom#2359): HH:MM,
    /// an RFC 3339 timestamp, or a +offset (+30m, +2h).
    #[arg(long)]
    start_at: Option<String>,
    /// Resume an interrupted encode, reusing J2K frames already on disk
    /// (dom#344). Requires the same source and settings as the first run.
    #[arg(long)]
    resume: bool,
    /// Power the machine off after a successful encode (dom#1394). Fails
    /// loud up front if no shutdown command is available.
    #[arg(long)]
    shutdown_when_done: bool,
}

/// W6 subtitle placement / RTL / wrap / font options, boxed into the Create
/// variant so it stays under the clippy large-variant threshold.
#[derive(Args)]
struct CreateSubtitleOpts {
    /// Subtitle horizontal alignment: left, center, or right (default center)
    #[arg(long, value_parser = ["left", "center", "right"])]
    subtitle_halign: Option<String>,
    /// Subtitle vertical anchor: top, center, or bottom (default bottom)
    #[arg(long, value_parser = ["top", "center", "bottom"])]
    subtitle_valign: Option<String>,
    /// Subtitle vertical position: percent from the valign edge (default 8)
    #[arg(long)]
    subtitle_vposition: Option<f64>,
    /// 3D subtitle depth: SMPTE Zposition emitted on every cue (stereoscopic)
    #[arg(long)]
    subtitle_zposition: Option<f64>,
    /// RTL subtitle reordering: auto, on, or off (default auto)
    #[arg(long, default_value = "auto", value_parser = ["auto", "on", "off"])]
    subtitle_rtl: String,
    /// Auto-wrap subtitle lines longer than this many characters
    #[arg(long)]
    subtitle_wrap: Option<usize>,
    /// TTF/OTF font to embed in the subtitle track (subset to used glyphs)
    #[arg(long)]
    subtitle_font: Option<String>,
    /// Subtitle file rendered into the picture during the encode (SRT, ASS,
    /// PAC, MKS, FCPXML, or Interop DCSubtitle). Burnt-in text is part of the
    /// image and registers no timed-text track.
    #[arg(long)]
    burn_subtitle: Option<String>,
    /// TTF/OTF font to draw --burn-subtitle with (default: a system font)
    #[arg(long)]
    burn_subtitle_font: Option<String>,
    /// Embed the whole font instead of subsetting it to the used glyphs
    #[arg(long)]
    subtitle_no_subset: bool,
    /// Timed-text point size for the --subtitle track (default 42). Not --ccap,
    /// which keeps the default appearance
    #[arg(long)]
    subtitle_font_size: Option<u32>,
    /// Timed-text colour for the --subtitle track as RRGGBB or RRGGBBAA
    /// (default FFFFFF)
    #[arg(long)]
    subtitle_colour: Option<String>,
    /// Timed-text effect for the --subtitle track: none, outline or shadow
    /// (default shadow). An outline is written as the ST 428-7 "border"
    #[arg(long, value_parser = ["none", "outline", "shadow"])]
    subtitle_effect: Option<String>,
    /// Timed-text effect colour for the --subtitle track as RRGGBB or RRGGBBAA
    /// (default 000000)
    #[arg(long)]
    subtitle_effect_colour: Option<String>,
    /// Timed-text fade up for the --subtitle track in milliseconds, rounded to
    /// whole frames (default a twelfth of a second)
    #[arg(long)]
    subtitle_fade_up: Option<u64>,
    /// Timed-text fade down for the --subtitle track in milliseconds, rounded to
    /// whole frames (default a twelfth of a second)
    #[arg(long)]
    subtitle_fade_down: Option<u64>,
    /// Closed-caption (ST 429-12) input, wrapped with a ClosedCaption role
    /// (accessibility track, distinct from --subtitle). SRT/styled or SMPTE DCST.
    #[arg(long, conflicts_with = "versions")]
    ccap: Option<String>,
    /// Closed-caption language code (e.g. "en", "fr")
    #[arg(long, default_value = "en")]
    ccap_language: String,
    #[command(flatten)]
    burn_appearance: CreateBurnAppearance,
}

impl CreateSubtitleOpts {
    /// The appearance flags the caller actually gave, so a refusal can name
    /// them.
    fn named_appearance(&self) -> Vec<&'static str> {
        [
            ("--subtitle-font-size", self.subtitle_font_size.is_some()),
            ("--subtitle-colour", self.subtitle_colour.is_some()),
            ("--subtitle-effect", self.subtitle_effect.is_some()),
            (
                "--subtitle-effect-colour",
                self.subtitle_effect_colour.is_some(),
            ),
            ("--subtitle-fade-up", self.subtitle_fade_up.is_some()),
            ("--subtitle-fade-down", self.subtitle_fade_down.is_some()),
        ]
        .into_iter()
        .filter(|(_, given)| *given)
        .map(|(name, _)| name)
        .collect()
    }

    /// How the packaged track looks, refusing a bad value under the flag's own
    /// name.
    fn appearance(&self) -> Result<dcpwizard_core::subtitle::TimedTextAppearance, String> {
        dcpwizard_core::subtitle::TimedTextAppearance::from_flags(
            self.subtitle_font_size,
            self.subtitle_colour.as_deref(),
            self.subtitle_effect.as_deref(),
            self.subtitle_effect_colour.as_deref(),
            self.subtitle_fade_up,
            self.subtitle_fade_down,
        )
    }
}

/// How `--burn-subtitle` draws, flattened into the subtitle group so the Create
/// variant stays under the clippy large-variant threshold. Every flag is laid
/// over postkit's burn defaults, so an unnamed one keeps the value it has always
/// had.
#[derive(Args)]
struct CreateBurnAppearance {
    #[arg(long, help = burn_font_size_help())]
    burn_font_size: Option<f32>,
    /// Burn-in text colour as RRGGBB or RRGGBBAA (default FFFFFF)
    #[arg(long)]
    burn_colour: Option<String>,
    /// What is drawn under burnt-in text: none, outline or shadow (default
    /// shadow)
    #[arg(long, value_parser = ["none", "outline", "shadow"])]
    burn_effect: Option<String>,
    /// Colour of that outline or shadow as RRGGBB or RRGGBBAA (default 000000)
    #[arg(long)]
    burn_effect_colour: Option<String>,
    #[arg(long, help = burn_outline_width_help())]
    burn_outline_width: Option<f32>,
    #[arg(long, help = burn_line_height_help())]
    burn_line_height: Option<f32>,
    #[arg(long, help = burn_margin_help())]
    burn_margin: Option<f32>,
    /// Horizontal stretch of the burnt-in text (default 1.0)
    #[arg(long)]
    burn_x_scale: Option<f32>,
    /// Vertical stretch of the burnt-in text (default 1.0)
    #[arg(long)]
    burn_y_scale: Option<f32>,
    /// How long a burnt-in cue takes to ramp up from transparent, in
    /// milliseconds (default 0)
    #[arg(long)]
    burn_fade_up: Option<u64>,
    /// How long a burnt-in cue takes to ramp down to transparent, in
    /// milliseconds (default 0)
    #[arg(long)]
    burn_fade_down: Option<u64>,
}

/// Turns postkit's ratios into the percentages the burn appearance flags take.
const RATIO_TO_PERCENT: f32 = 100.0;

/// The visible mark `create` burns into the picture, boxed into the Create
/// variant the way the other flag groups are.
#[derive(Args)]
struct CreateWatermark {
    /// Text burnt visibly into every picture frame (distributor ID, serial).
    /// Part of the image, so it registers no track of its own
    #[arg(long)]
    watermark: Option<String>,
    #[arg(long, help = watermark_font_size_help())]
    watermark_font_size: Option<f32>,
    /// Watermark colour as RRGGBB or RRGGBBAA (default FFFFFF)
    #[arg(long)]
    watermark_colour: Option<String>,
    /// Watermark placement: top, center or bottom (default bottom)
    #[arg(long, value_parser = ["top", "center", "bottom"])]
    watermark_position: Option<String>,
}

impl CreateWatermark {
    /// The appearance flags the caller actually gave, so a refusal can name
    /// them.
    fn named_appearance(&self) -> Vec<&'static str> {
        [
            ("--watermark-font-size", self.watermark_font_size.is_some()),
            ("--watermark-colour", self.watermark_colour.is_some()),
            ("--watermark-position", self.watermark_position.is_some()),
        ]
        .into_iter()
        .filter(|(_, given)| *given)
        .map(|(name, _)| name)
        .collect()
    }
}

/// What the appearance flags are called where they were given, so a refusal
/// names the flag the caller typed.
struct WatermarkFlags {
    font_size: &'static str,
    colour: &'static str,
    position: &'static str,
}

/// The mark's appearance flags under `create`, where they sit beside the
/// picture's own.
const CREATE_WATERMARK_FLAGS: WatermarkFlags = WatermarkFlags {
    font_size: "--watermark-font-size",
    colour: "--watermark-colour",
    position: "--watermark-position",
};

/// The same flags under the `watermark` command, where the mark is all there is.
const WATERMARK_COMMAND_FLAGS: WatermarkFlags = WatermarkFlags {
    font_size: "--font-size",
    colour: "--colour",
    position: "--position",
};

/// How the mark is drawn, refusing a bad value under the flag's own name.
/// `None` when no mark was asked for.
fn watermark_options(
    text: Option<&str>,
    font_size_percent: Option<f32>,
    colour: Option<&str>,
    position: Option<&str>,
    flags: &WatermarkFlags,
) -> Result<Option<dcpwizard_core::watermark::WatermarkOptions>, String> {
    let Some(text) = text else {
        return Ok(None);
    };
    let base = dcpwizard_core::watermark::WatermarkOptions::default();
    if let Some(percent) = font_size_percent {
        postkit::subtitle_raster::check_font_size_percent(percent)
            .map_err(|e| format!("{}: {e}", flags.font_size))?;
    }
    let colour = match colour {
        Some(text) => dcpwizard_core::subtitle::parse_colour_flag(flags.colour, text)?,
        None => base.colour,
    };
    let position = match position {
        Some(text) => dcpwizard_core::watermark::parse_position_flag(flags.position, text)?,
        None => base.position,
    };
    Ok(Some(dcpwizard_core::watermark::WatermarkOptions {
        text: text.to_string(),
        font_size_percent: font_size_percent.unwrap_or(base.font_size_percent),
        colour,
        position,
    }))
}

/// The mark is one cue with no fade covering every frame of any length of
/// picture, so the rate its timings are read against never changes what it
/// draws.
const WATERMARK_BURN_FPS: f64 = 24.0;

fn watermark_font_size_help() -> String {
    format!(
        "Watermark text height as a percent of the frame height (default {:.1})",
        dcpwizard_core::watermark::DEFAULT_FONT_SIZE_PERCENT
    )
}

fn burn_font_size_help() -> String {
    format!(
        "Burn-in text height as a percent of the frame height (default {:.1})",
        postkit::subtitle_raster::DEFAULT_FONT_SIZE_RATIO * RATIO_TO_PERCENT
    )
}

fn burn_outline_width_help() -> String {
    format!(
        "Burn-in outline thickness as a percent of the text height (default {:.1})",
        postkit::subtitle_raster::DEFAULT_OUTLINE_WIDTH_RATIO * RATIO_TO_PERCENT
    )
}

fn burn_line_height_help() -> String {
    format!(
        "Burn-in line box height as a multiple of the text height (default {:.2})",
        postkit::subtitle_raster::DEFAULT_LINE_HEIGHT_RATIO
    )
}

fn burn_margin_help() -> String {
    format!(
        "Burn-in distance from the anchored edge as a percent of the frame height (default {:.1})",
        postkit::subtitle_raster::DEFAULT_MARGIN_RATIO * RATIO_TO_PERCENT
    )
}

impl CreateBurnAppearance {
    /// The flags the caller actually gave, so a refusal can name them.
    fn named(&self) -> Vec<&'static str> {
        [
            ("--burn-font-size", self.burn_font_size.is_some()),
            ("--burn-colour", self.burn_colour.is_some()),
            ("--burn-effect", self.burn_effect.is_some()),
            ("--burn-effect-colour", self.burn_effect_colour.is_some()),
            ("--burn-outline-width", self.burn_outline_width.is_some()),
            ("--burn-line-height", self.burn_line_height.is_some()),
            ("--burn-margin", self.burn_margin.is_some()),
            ("--burn-x-scale", self.burn_x_scale.is_some()),
            ("--burn-y-scale", self.burn_y_scale.is_some()),
            ("--burn-fade-up", self.burn_fade_up.is_some()),
            ("--burn-fade-down", self.burn_fade_down.is_some()),
        ]
        .into_iter()
        .filter(|(_, given)| *given)
        .map(|(name, _)| name)
        .collect()
    }

    /// The appearance to lay over postkit's burn defaults, refusing a bad value
    /// under the flag's own name.
    fn overrides(&self) -> Result<postkit::subtitle_raster::BurnStyleOverrides, String> {
        use dcpwizard_core::subtitle::{parse_colour_flag, parse_effect_flag};
        Ok(postkit::subtitle_raster::BurnStyleOverrides {
            font_size_percent: self.burn_font_size,
            colour: match self.burn_colour.as_deref() {
                Some(text) => Some(parse_colour_flag("--burn-colour", text)?),
                None => None,
            },
            effect: match self.burn_effect.as_deref() {
                Some(text) => Some(parse_effect_flag("--burn-effect", text)?),
                None => None,
            },
            effect_colour: match self.burn_effect_colour.as_deref() {
                Some(text) => Some(parse_colour_flag("--burn-effect-colour", text)?),
                None => None,
            },
            outline_width_percent: self.burn_outline_width,
            line_height_ratio: self.burn_line_height,
            margin_percent: self.burn_margin,
            x_scale: self.burn_x_scale,
            y_scale: self.burn_y_scale,
            fade_up_ms: self.burn_fade_up,
            fade_down_ms: self.burn_fade_down,
        })
    }
}

/// Source-shaping options: what colour the source carries, how much of it to
/// keep, and how long to hold a still. Boxed into the Create variant.
#[derive(Args)]
struct CreateSourceOpts {
    /// Colour space the source carries: rec709 (default), xyz, p3, p3d65, rec2020 or
    /// logc, all of which the encode lands on X'Y'Z' itself. aces and acescg are
    /// scene-referred and refused: convert those first with `dcpwizard colour
    /// --target xyz --lut <LUT>` and then pass xyz.
    #[arg(long, default_value = "rec709")]
    source_colourspace: String,
    /// Trim this much off the head of the source before encoding. Duration with
    /// a unit: frames (48f) or seconds (2s). Trim runs before padding, so
    /// `--trim-start 2s --pad-head 1s` drops two seconds and then prepends one.
    #[arg(long)]
    trim_start: Option<String>,
    /// Trim this much off the tail of the source. Same syntax as --trim-start.
    #[arg(long)]
    trim_end: Option<String>,
    /// Hold a single-image --video for this long. Same syntax as --trim-start.
    /// Required for a still input, and refused for anything else.
    #[arg(long)]
    still_length: Option<String>,
}

/// Black frames and silence extending the head and tail of the programme, and
/// the colour they are filled with. Boxed into the Create variant.
#[derive(Args)]
struct CreatePadding {
    /// Pad the head with black frames + silence. Duration with a unit:
    /// frames (48f) or seconds (2s). Shifts subtitles by the same offset.
    #[arg(long)]
    pad_head: Option<String>,
    /// Pad the tail with black frames + silence. Same syntax as --pad-head.
    #[arg(long)]
    pad_tail: Option<String>,
    /// Background/pad colour as hex sRGB (RRGGBB or #RRGGBB). Default: black.
    /// Applied to head/tail pad frames via the DCDM colour transform.
    #[arg(long)]
    pad_color: Option<String>,
}

/// Where a finished build is delivered on top of being written to --output.
/// Boxed into the Create variant.
#[derive(Args)]
struct CreateDelivery {
    /// Upload the finished package to the theatre management system after a
    /// successful build. Fails loud up front if the config cannot be read.
    #[arg(long)]
    upload_to_tms: bool,
    /// TMS config TOML holding the protocol, host, path, user and password
    /// (default: <config dir>/dcpwizard/tms.toml)
    #[arg(long)]
    tms_config: Option<String>,
}

/// Library items joined onto the build as reels of their own, and the folder
/// they are resolved against. Boxed into the Create variant.
#[derive(Args)]
struct CreateLibraryItems {
    /// Join a library item onto the head of the build as its own reel, by name
    /// (repeatable, in the order given). See `library list`.
    #[arg(long = "head-item", value_name = "NAME", conflicts_with = "versions")]
    head_items: Vec<String>,
    /// Join a library item onto the tail of the build as its own reel, by name
    /// (repeatable, in the order given).
    #[arg(long = "tail-item", value_name = "NAME", conflicts_with = "versions")]
    tail_items: Vec<String>,
    /// Library folder to resolve --head-item / --tail-item against
    /// (default: XDG data dir)
    #[arg(long)]
    library_dir: Option<String>,
}

/// Source picture processing: what happens to the decoded frames before they
/// are compressed. Boxed into the Create variant.
#[derive(Args)]
struct CreatePictureOpts {
    /// Cut this many pixels off the source's left edge, before any rotation
    #[arg(long, default_value_t = 0)]
    crop_left: u32,
    /// Cut this many pixels off the source's right edge, before any rotation
    #[arg(long, default_value_t = 0)]
    crop_right: u32,
    /// Cut this many pixels off the source's top edge, before any rotation
    #[arg(long, default_value_t = 0)]
    crop_top: u32,
    /// Cut this many pixels off the source's bottom edge, before any rotation
    #[arg(long, default_value_t = 0)]
    crop_bottom: u32,
    /// Measure the source's black borders and cut them off
    #[arg(long)]
    auto_crop: bool,
    /// Black level --auto-crop reads as border, 0 to 1 (default 0.1)
    #[arg(long)]
    auto_crop_threshold: Option<f32>,
    /// Cut the source to the container's aspect so the picture fills the frame
    /// instead of being letterboxed. Needs --container or --twok/--fourk.
    #[arg(long)]
    fill_crop: bool,
    /// Turn the source's fields into progressive frames
    #[arg(long)]
    deinterlace: bool,
    /// Run the source through a denoiser
    #[arg(long)]
    denoise: bool,
    /// Rotate the picture clockwise by this many degrees, after the crop
    #[arg(long, value_parser = ["90", "180", "270"])]
    rotate: Option<String>,
    /// Flip the picture, after any rotation
    #[arg(long, value_parser = ["horizontal", "vertical", "both"])]
    flip: Option<String>,
}

impl CreatePictureOpts {
    fn resolve(&self) -> Result<dcpwizard_core::source_picture::SourcePictureOptions, String> {
        use dcpwizard_core::source_picture::SourcePictureOptions;
        use postkit::picture_processing::{
            DEFAULT_AUTO_CROP_THRESHOLD, parse_flip, parse_rotation,
        };
        let (flip_horizontal, flip_vertical) =
            parse_flip(self.flip.as_deref().unwrap_or_default())?;
        Ok(SourcePictureOptions {
            crop: postkit::picture_processing::Crop {
                left: self.crop_left,
                right: self.crop_right,
                top: self.crop_top,
                bottom: self.crop_bottom,
            },
            auto_crop: self.auto_crop,
            auto_crop_threshold: self
                .auto_crop_threshold
                .unwrap_or(DEFAULT_AUTO_CROP_THRESHOLD),
            fill_crop: self.fill_crop,
            deinterlace: self.deinterlace,
            denoise: self.denoise,
            rotation: parse_rotation(self.rotate.as_deref().unwrap_or_default())?,
            flip_horizontal,
            flip_vertical,
        })
    }
}

/// The KDM choices beyond the certificates and the window, shared by `kdm`,
/// `kdm-batch` and `kdm-rewrap`.
#[derive(Args)]
struct KdmOptionArgs {
    /// KDM formulation: the dci- ones add a ContentAuthenticator;
    /// multiple-modified-transitional-1 and dci-specific list the --device-cert
    /// devices, the other two trust any device. Absent derives it from whether
    /// --device-cert was given.
    #[arg(long)]
    formulation: Option<postkit::certificate::KdmFormulation>,
    /// Disable forensic marking of the picture essence, as press and festival
    /// screenings are usually ordered
    #[arg(short = 'p', long)]
    disable_forensic_marking_picture: bool,
    /// Disable forensic marking of the audio essence, optionally only above a
    /// given channel (e.g. 12) so the HI/VI tracks below it keep theirs
    #[arg(short = 'a', long, num_args = 0..=1, value_name = "CHANNEL")]
    disable_forensic_marking_audio: Option<Option<u32>>,
}

impl From<KdmOptionArgs> for dcpwizard_core::kdm::KdmOptions {
    fn from(args: KdmOptionArgs) -> Self {
        use postkit::certificate::{AudioForensicMarking, PictureForensicMarking};
        dcpwizard_core::kdm::KdmOptions {
            formulation: args.formulation,
            picture_forensic_marking: if args.disable_forensic_marking_picture {
                PictureForensicMarking::Disabled
            } else {
                PictureForensicMarking::Enabled
            },
            // absent leaves marking on, bare disables every channel, a number
            // disables the channels above it
            audio_forensic_marking: match args.disable_forensic_marking_audio {
                None => AudioForensicMarking::Enabled,
                Some(None) => AudioForensicMarking::Disabled,
                Some(Some(channel)) => AudioForensicMarking::DisabledAboveChannel(channel),
            },
        }
    }
}

/// CPL/PKL signing identity. Boxed into the Create variant so it stays under
/// the clippy large-variant threshold. Absent leaves the package unsigned.
#[derive(Args)]
struct SignerOpts {
    /// Signer leaf certificate file. With --signer-key, signs the CPL and PKL.
    #[arg(long, requires = "signer_key")]
    signer_cert: Option<String>,
    /// Signer private key file, required with --signer-cert
    #[arg(long, requires = "signer_cert")]
    signer_key: Option<String>,
    /// Signer CA certificate above the leaf (repeatable: intermediate(s) then root)
    #[arg(long, requires = "signer_cert")]
    signer_chain: Vec<String>,
}

/// The signer, or None when no --signer-cert was given. clap's
/// `requires` guarantees the key is present whenever the certificate is.
fn package_signer(opts: &SignerOpts) -> Option<dcpwizard_core::package_signature::PackageSigner> {
    let cert = opts.signer_cert.as_ref()?;
    let key = opts
        .signer_key
        .as_ref()
        .expect("clap requires --signer-key alongside --signer-cert");
    Some(dcpwizard_core::package_signature::PackageSigner {
        signer_cert: PathBuf::from(cert),
        signer_key: PathBuf::from(key),
        signer_chain: opts.signer_chain.iter().map(PathBuf::from).collect(),
    })
}

#[derive(Parser)]
#[command(
    name = "dcpwizard",
    version,
    about = "DCP Wizard — Digital Cinema Package creator"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Encode and decode on grok's accelerator plugin
    #[arg(long, global = true)]
    gpu: bool,

    #[arg(long, global = true, conflicts_with = "gpu")]
    no_gpu: bool,

    #[arg(long, global = true, help = "Grok accelerator plugin license")]
    license: Option<String>,

    #[arg(long, global = true, help = "Grok license registration URL")]
    registration_url: Option<String>,
}

#[derive(Subcommand)]
enum PreferencesCommand {
    #[command(about = "Print every preference as JSON")]
    Show,
    #[command(about = "Print the preferences file path")]
    Path,
    #[command(about = "Restore every preference to its default")]
    Reset,
    #[command(about = "Set one preference")]
    Set {
        #[arg(help = "JSON field name, with camelCase, kebab-case, or snake_case accepted")]
        name: String,
        #[arg(help = "New value")]
        value: String,
    },
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Show or change saved preferences")]
    Preferences {
        #[command(subcommand)]
        action: PreferencesCommand,
    },
    /// Create a new DCP
    Create {
        /// DCP title
        #[arg(short, long)]
        title: String,
        /// Video file (mp4/mov/mkv) or J2K/image sequence directory
        #[arg(long)]
        video: String,
        /// Audio WAV file
        #[arg(long)]
        audio: Option<String>,
        /// Six-channel WAV order: dcp (L,R,C,LFE,Ls,Rs) or lrc-ls-rs-lfe
        #[arg(long, default_value = "dcp")]
        audio_input_order: String,
        /// HDR-to-DCI 3D LUT. Required for HDR source video unless generic tone mapping is enabled.
        #[arg(long)]
        hdr_to_dci_lut: Option<String>,
        /// Allow generic FFmpeg HDR tone mapping. It is not a delivery transform.
        #[arg(long, conflicts_with = "hdr_dci")]
        allow_generic_hdr_tonemap: bool,
        /// Author a DCI HDR Addendum DCP (ST 2084 PQ stamped on the picture MXF
        /// and the CPL). The master's own colour tags name the grade; pass
        /// --hdr-source, --hdr-to-dci-lut or --hdr-already-pq to override that.
        /// Not supported with 3D or reel splitting.
        #[arg(long)]
        hdr_dci: bool,
        /// HDR grade the master carries, when its colour tags do not say.
        #[arg(
            long,
            value_enum,
            requires = "hdr_dci",
            conflicts_with_all = ["hdr_to_dci_lut", "hdr_already_pq"]
        )]
        hdr_source: Option<HdrSourceArg>,
        /// Peak luminance of the grade in cd/m², where the DCI HDR roll-off
        /// starts. Defaults to the master's MaxCLL or mastering display maximum.
        #[arg(long, requires = "hdr_dci")]
        hdr_peak_nits: Option<f32>,
        /// Acknowledge the source is already ST 2084 PQ (DCI HDR), so --hdr-dci
        /// needs no LUT conversion.
        #[arg(long, requires = "hdr_dci", conflicts_with = "hdr_to_dci_lut")]
        hdr_already_pq: bool,
        /// Sign-language video (ISDCF Doc 13): encoded to VP9 and packed onto
        /// channel 15 of the sound track. Requires --sign-language-lang.
        #[arg(long, requires = "sign_language_lang")]
        sign_language_video: Option<String>,
        /// RFC 5646 sign-language tag for --sign-language-video (e.g. sgn-ase).
        #[arg(long)]
        sign_language_lang: Option<String>,

        /// SRT file to convert, or supplied SMPTE subtitle XML to package unchanged
        #[arg(long, conflicts_with = "versions")]
        subtitle: Option<String>,
        /// Multi-version manifest (JSON array): one CPL per entry over shared essence
        #[arg(long)]
        versions: Option<String>,
        /// Subtitle language code (e.g. "en", "fr")
        #[arg(long, default_value = "en")]
        subtitle_language: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// DCP standard (smpte|interop)
        #[arg(long, default_value = "smpte")]
        standard: String,
        /// Delivery profile
        #[arg(long)]
        profile: Option<String>,
        /// Encrypt the DCP
        #[arg(long)]
        encrypt: bool,
        /// Where to write the content keys (required with --encrypt). Holds the
        /// plaintext AES keys: point it outside the DCP and keep it secret.
        #[arg(long, required_if_eq("encrypt", "true"))]
        key_out: Option<String>,
        /// DCP frame rate (auto-detected from source if not specified)
        #[arg(long)]
        frame_rate: Option<u32>,
        /// Force 2K resolution
        #[arg(long)]
        twok: bool,
        /// Force 4K resolution
        #[arg(long)]
        fourk: bool,
        /// Picture container: 2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, or 4k-full
        #[arg(long)]
        container: Option<String>,
        /// Number of encoding threads (default: auto-detect CPU count)
        #[arg(short = 'j', long)]
        threads: Option<u32>,
        /// J2K bandwidth in Mbit/s (default: 250 for 2K, 500 for 4K)
        #[arg(long)]
        video_bit_rate: Option<u32>,
        /// PSNR target in dB (20 to 80) the encoder allocates to instead of the bandwidth's byte target, which becomes a per-frame ceiling
        #[arg(long)]
        quality_psnr: Option<f64>,
        /// Split into reels of at most N minutes each (default: single reel)
        #[arg(long)]
        reel_length: Option<u32>,
        /// Right-eye video/J2K for a stereoscopic 3D DCP (main input is left eye)
        #[arg(long)]
        right_eye: Option<String>,
        /// Dolby Atmos / DCData bitstream to wrap as a ST 429-18 auxiliary track
        #[arg(long)]
        atmos: Option<String>,
        /// Sound channel index (0-based) carrying the Hearing Impaired (HI) track
        #[arg(long)]
        hi_channel: Option<u32>,
        /// Sound channel index (0-based) carrying the Visually Impaired (VI-N) track
        #[arg(long)]
        vi_channel: Option<u32>,
        #[command(flatten)]
        padding: Box<CreatePadding>,
        /// Custom container dimensions WxH (e.g. 1920x1080). Must be even and fit
        /// within the 2K (2048x1080) or 4K (4096x2160) container. Overrides --container.
        #[arg(long, conflicts_with = "container")]
        container_dims: Option<String>,
        /// Split into reels at these timecodes (comma-separated HH:MM:SS or
        /// HH:MM:SS:FF). Each reel must be >= 1s. Conflicts with --reel-length.
        #[arg(long, conflicts_with = "reel_length")]
        split_at: Option<String>,
        /// Split into reels at the source video's chapter marks (ffprobe).
        /// Conflicts with --reel-length and --split-at.
        #[arg(long, conflicts_with_all = ["reel_length", "split_at"])]
        split_chapters: bool,
        /// Force the ffmpeg decode range: full or legal. Corrects wrong or missing
        /// source range metadata (video input only).
        #[arg(long, value_parser = ["full", "legal"])]
        input_range: Option<String>,
        /// Place a CPL marker: LABEL=timecode (repeatable). LABEL is one of FFOC,
        /// LFOC, FFTC, LFTC, FFOI, LFOI, FFEC, LFEC, FFMC, LFMC; the timecode is
        /// a frame number or HH:MM:SS:FF. Given markers replace the default
        /// FFOC/LFOC pair. Not supported with reel splitting.
        #[arg(long = "marker", value_name = "LABEL=TIMECODE")]
        markers: Vec<String>,
        // boxed so the Create variant stays small (clippy large_enum_variant).
        #[command(flatten)]
        composition_metadata: Box<CreateCompositionMetadata>,
        #[command(flatten)]
        audio_qol: Box<CreateAudioQol>,
        #[command(flatten)]
        source_opts: Box<CreateSourceOpts>,
        #[command(flatten)]
        picture_opts: Box<CreatePictureOpts>,
        #[command(flatten)]
        subtitle_qol: Box<CreateSubtitleOpts>,
        #[command(flatten)]
        watermark_opts: Box<CreateWatermark>,
        #[command(flatten)]
        isdcf_naming: Box<CreateIsdcfNaming>,
        #[command(flatten)]
        signer_opts: Box<SignerOpts>,

        #[command(flatten)]
        library_items: Box<CreateLibraryItems>,
        #[command(flatten)]
        delivery: Box<CreateDelivery>,

        /// Run the pre-build check and stop: every refusal and every hint,
        /// without encoding or writing anything under --output.
        #[arg(long)]
        check: bool,
    },
    /// Rebuild ASSETMAP and PKL to cover every asset file present (metadata-only
    /// repackaging; no re-wrap or re-encode). For re-ingesting exported OV/VF
    /// folders whose ASSETMAP/PKL omit hardlinked assets.
    IngestPackage {
        /// DCP package directory to repackage in place
        dir: String,
        #[command(flatten)]
        signer_opts: SignerOpts,
    },
    /// Combine several complete DCPs into one delivery volume with a merged
    /// ASSETMAP/VOLINDEX (and, by default, a single merged PKL). CPLs and essence
    /// are copied byte-identical, so signatures/hashes stay valid.
    Combine {
        /// Input DCP directories (two or more)
        #[arg(required = true, num_args = 1..)]
        inputs: Vec<String>,
        /// Output volume directory
        #[arg(short, long)]
        output: String,
        /// Keep each input's PKL as its own file instead of one merged PKL
        #[arg(long)]
        separate_pkls: bool,
        /// Order CPL entries alphabetically by content title
        #[arg(long)]
        sort: bool,
        /// AnnotationText for the merged PKL/ASSETMAP (default: derived from titles)
        #[arg(long)]
        annotation: Option<String>,
        #[command(flatten)]
        signer_opts: SignerOpts,
    },
    /// Create a supplemental Version File (VF) DCP against an Original Version
    CreateVf {
        /// Original Version (OV) DCP directory
        #[arg(long)]
        ov: String,
        /// Output VF directory
        #[arg(short, long)]
        output: String,
        /// VF title (defaults to "<OV title>_VF")
        #[arg(short, long, default_value = "")]
        title: String,
        /// Replace a reel's picture essence: --replace-picture REEL=PATH (repeatable)
        #[arg(long = "replace-picture", value_name = "REEL=PATH")]
        replace_picture: Vec<String>,
        /// Replace a reel's sound essence: --replace-sound REEL=PATH (repeatable)
        #[arg(long = "replace-sound", value_name = "REEL=PATH")]
        replace_sound: Vec<String>,
        /// Replace a reel's subtitle: --replace-subtitle REEL=PATH (SRT or SMPTE XML)
        #[arg(long = "replace-subtitle", value_name = "REEL=PATH")]
        replace_subtitle: Vec<String>,
        /// Add a subtitle to a reel that has none: --add-subtitle REEL=PATH
        #[arg(long = "add-subtitle", value_name = "REEL=PATH")]
        add_subtitle: Vec<String>,
        /// Replace a reel's closed caption: --replace-ccap REEL=PATH (SRT or SMPTE XML)
        #[arg(long = "replace-ccap", value_name = "REEL=PATH")]
        replace_ccap: Vec<String>,
        /// Add a closed caption to a reel that has none: --add-ccap REEL=PATH
        #[arg(long = "add-ccap", value_name = "REEL=PATH")]
        add_ccap: Vec<String>,
        /// Language code for wrapped subtitle tracks
        #[arg(long, default_value = "en")]
        subtitle_language: String,
        /// TTF/OTF font to embed in the wrapped timed text (default: a system
        /// sans face)
        #[arg(long)]
        subtitle_font: Option<String>,
        #[command(flatten)]
        signer_opts: SignerOpts,
    },
    /// Assemble a new OV composition from existing DCPs: one new CPL whose reels
    /// are the inputs' reels in order. Essence is copied byte-identical and
    /// referenced by its existing UUIDs. Inputs must share standard/rate/
    /// resolution and must not be encrypted.
    Assemble {
        /// Input DCP directories (two or more), in program order
        #[arg(long = "input", required = true, num_args = 1..)]
        input: Vec<String>,
        /// Output OV directory
        #[arg(short, long)]
        output: String,
        /// Title for the assembled composition
        #[arg(short, long, default_value = "")]
        title: String,
        #[command(flatten)]
        signer_opts: SignerOpts,
    },
    /// Edit a DCP's CPL metadata (title/annotation/content-kind/issuer) without
    /// re-wrapping essence. Assigns a new CPL id and refreshes PKL/ASSETMAP.
    /// Refuses encrypted DCPs (the KDM binds the CPL id).
    Edit {
        /// DCP directory to edit
        #[arg(long)]
        input: String,
        /// Write the edited DCP here (copied first); omit to edit in place
        #[arg(short, long)]
        output: Option<String>,
        /// New content title
        #[arg(long)]
        title: Option<String>,
        /// New CPL AnnotationText
        #[arg(long)]
        annotation: Option<String>,
        /// New content kind (abbrev FTR/TLR/... or a raw kind string)
        #[arg(long)]
        content_kind: Option<String>,
        /// New Issuer
        #[arg(long)]
        issuer: Option<String>,
    },
    /// Build a multi-composition DCP: one CPL per manifest entry, each with its
    /// own j2k_dir/audio/subtitle, over one shared PKL/ASSETMAP. Contrast
    /// `create --versions` (multiple CPLs over shared essence).
    CreateMulti {
        /// Compositions manifest (JSON array): each entry names title, j2k_dir,
        /// and optional audio/subtitle/subtitle_language/kind
        #[arg(long)]
        compositions: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// DCP standard (smpte|interop)
        #[arg(long, default_value = "smpte")]
        standard: String,
        /// DCP frame rate
        #[arg(long, default_value = "24")]
        frame_rate: u32,
        /// Force 4K resolution (default 2K)
        #[arg(long)]
        fourk: bool,
        /// Picture container: 2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, 4k-full
        #[arg(long)]
        container: Option<String>,
        /// Custom container dimensions WxH (e.g. 1920x1080). Must be even and fit
        /// within the 2K (2048x1080) or 4K (4096x2160) container. Overrides --container.
        #[arg(long, conflicts_with = "container")]
        container_dims: Option<String>,
        /// Default subtitle language code for entries that omit one
        #[arg(long, default_value = "en")]
        subtitle_language: String,
        /// Default content type abbrev (FTR, TLR, ...) for entries that omit kind
        #[arg(long)]
        content_type: Option<String>,
        /// Encrypt the DCP
        #[arg(long)]
        encrypt: bool,
        /// Where to write the content keys (required with --encrypt)
        #[arg(long, required_if_eq("encrypt", "true"))]
        key_out: Option<String>,
        #[command(flatten)]
        signer_opts: SignerOpts,
    },
    /// Encode an image sequence to JPEG 2000 codestreams in <OUTPUT>/j2k
    Encode {
        /// Input image directory
        #[arg(short, long)]
        input: String,
        /// Output directory, the codestreams land in its j2k subdirectory
        #[arg(short, long)]
        output: String,
        /// Target bitrate (Mbps)
        #[arg(long, default_value = "250")]
        bandwidth: u32,
        /// Frame rate the bitrate is budgeted at
        #[arg(long, default_value = "24")]
        fps: u32,
    },
    /// Full pipeline: video → J2K → DCP (streaming, no intermediate files)
    Pipeline {
        /// Input video file (or image/J2K directory)
        #[arg(short, long)]
        input: String,
        /// DCP title
        #[arg(short, long)]
        title: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// Audio WAV file
        #[arg(long)]
        audio: Option<String>,
        /// Compression ratio (default: 10)
        #[arg(long, default_value = "10")]
        ratio: f64,
        /// Frame rate (default: 24)
        #[arg(long, default_value = "24")]
        fps: u32,
        /// Force the ffmpeg decode range: full or legal. Corrects wrong or missing
        /// source range metadata.
        #[arg(long, value_parser = ["full", "legal"])]
        input_range: Option<String>,
        /// Split into reels at the source video's chapter marks (ffprobe).
        #[arg(long)]
        split_chapters: bool,
    },
    /// Transcode video to image sequence
    Transcode {
        /// Input video file
        #[arg(short, long)]
        input: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// Image format: tiff (default), dpx, exr, png
        #[arg(long, default_value = "tiff")]
        format: String,
        /// Bit depth: 16 (default), 10, or 8
        #[arg(long, default_value = "16")]
        bit_depth: u32,
    },
    /// Re-encode an existing DCP's picture essence to a lower bandwidth
    TranscodeDcp {
        /// Input DCP directory
        #[arg(short, long)]
        input: String,
        /// Output DCP directory (must differ from input)
        #[arg(short, long)]
        output: String,
        /// Target picture bandwidth in Mbit/s
        #[arg(long)]
        video_bit_rate: u32,
        /// Optional target width (with --height, rescales the picture)
        #[arg(long)]
        width: Option<u32>,
        /// Optional target height (with --width, rescales the picture)
        #[arg(long)]
        height: Option<u32>,
        /// KDM XML to decrypt an encrypted source (needs --recipient-key)
        #[arg(long)]
        kdm: Option<String>,
        /// Recipient RSA private key (PEM) matching --kdm
        #[arg(long)]
        recipient_key: Option<String>,
        /// dcpwizard KEYS.json, an alternative key source to --kdm
        #[arg(long)]
        keys: Option<String>,
    },
    /// Decrypt an encrypted DCP into a cleartext DCP with the same structure
    Decrypt {
        /// Input (encrypted) DCP directory
        #[arg(short, long)]
        input: String,
        /// Output DCP directory (must differ from input)
        #[arg(short, long)]
        output: String,
        /// KDM XML (needs --recipient-key)
        #[arg(long)]
        kdm: Option<String>,
        /// Recipient RSA private key (PEM) matching --kdm
        #[arg(long)]
        recipient_key: Option<String>,
        /// dcpwizard KEYS.json, an alternative key source to --kdm
        #[arg(long)]
        keys: Option<String>,
    },
    /// Verify an existing DCP
    Verify {
        /// DCP directory
        dcp_dir: String,
        /// Skip asset hash verification
        #[arg(long)]
        no_hash_check: bool,
        /// Skip picture bitstream checks (faster)
        #[arg(long)]
        no_picture_check: bool,
        /// Require strict SMPTE Bv2.1 compliance
        #[arg(long)]
        strict: bool,
        /// Write report to file (.txt or .html)
        #[arg(short, long)]
        output: Option<String>,
        /// Quiet mode (exit code only, no output)
        #[arg(short, long)]
        quiet: bool,
    },
    /// Show DCP metadata
    Info {
        /// DCP directory
        dcp_dir: String,
    },
    /// Generate KDM for encrypted DCP
    Kdm {
        /// CPL ID
        #[arg(long)]
        cpl_id: String,
        /// Content title
        #[arg(long)]
        content_title: String,
        /// Recipient certificate file
        #[arg(long)]
        cert: String,
        /// Signer leaf certificate file
        #[arg(long)]
        signer_cert: String,
        /// Signer private key file
        #[arg(long)]
        signer_key: String,
        /// Signer CA certificate above the leaf (repeatable: intermediate(s) then root)
        #[arg(long)]
        signer_chain: Vec<String>,
        /// Output KDM file
        #[arg(short, long)]
        output: String,
        /// Valid from (ISO 8601 or "now"). Overrides a --template start.
        #[arg(short = 'f', long)]
        valid_from: Option<String>,
        /// Valid to (ISO 8601 or relative duration). Overrides a --template end.
        #[arg(short = 't', long)]
        valid_to: Option<String>,
        /// Named validity template to expand the window from (kdm-template)
        #[arg(long)]
        template: Option<String>,
        /// Validity templates file (default: XDG data dir)
        #[arg(long)]
        templates_file: Option<String>,
        /// KDM history log file (default: XDG data dir); every KDM is recorded
        #[arg(long)]
        history_file: Option<String>,
        /// Email the KDM to this address (repeatable). Requires --smtp-config
        #[arg(long = "email-to")]
        email_to: Vec<String>,
        /// SMTP config TOML for sending the KDM by email
        #[arg(long)]
        smtp_config: Option<String>,
        /// DCP keys file (KEYS.json from `create --encrypt`) whose content keys
        /// this KDM should carry. Required to unlock an encrypted DCP.
        #[arg(long)]
        keys: Option<String>,
        /// KDM format: smpte (default) or interop (legacy, needs real-gear validation)
        #[arg(long, default_value = "smpte")]
        format: String,
        /// AnnotationText override (default: "<title> KDM for <recipient>")
        #[arg(long)]
        annotation: Option<String>,
        /// Playback device certificate this KDM is restricted to (repeatable).
        /// Naming any device replaces the DCI assume-trust entry, so the KDM
        /// then plays only on the devices listed here.
        #[arg(long = "device-cert")]
        device_cert: Vec<String>,
        #[command(flatten)]
        kdm_options: KdmOptionArgs,
    },
    /// Re-wrap a DKDM to a new recipient
    KdmRewrap {
        /// Source DKDM file
        #[arg(long)]
        dkdm: String,
        /// DKDM recipient's private key (decrypts the source key blocks)
        #[arg(long)]
        dkdm_key: String,
        /// New recipient certificate file
        #[arg(long)]
        cert: String,
        /// Signer leaf certificate file
        #[arg(long)]
        signer_cert: String,
        /// Signer private key file
        #[arg(long)]
        signer_key: String,
        /// Signer CA certificate above the leaf (repeatable: intermediate(s) then root)
        #[arg(long)]
        signer_chain: Vec<String>,
        /// Valid from: ISO 8601 or "now"; empty preserves the DKDM window
        #[arg(short = 'f', long, default_value = "")]
        valid_from: String,
        /// Valid to: ISO 8601 or relative duration; empty preserves the DKDM window
        #[arg(short = 't', long, default_value = "")]
        valid_to: String,
        /// Output KDM file
        #[arg(short, long)]
        output: String,
        /// Playback device certificate the re-wrapped KDM is restricted to
        /// (repeatable). The source DKDM's device list is never carried over,
        /// because it names the DKDM recipient's devices, not the new one's.
        #[arg(long = "device-cert")]
        device_cert: Vec<String>,
        #[command(flatten)]
        kdm_options: KdmOptionArgs,
    },
    /// Copy DCP to drive
    Copy {
        /// DCP directory
        #[arg(long)]
        src: String,
        /// Destination drive/directory
        #[arg(long)]
        dst: String,
    },
    /// Format a delivery drive as ext2/ext3 (cinema hard-drive delivery).
    /// Refuses any mounted target; requires --yes.
    FormatDrive {
        /// Target block device (or regular file with --image)
        target: String,
        /// Filesystem: ext2 or ext3
        #[arg(long, default_value = "ext2")]
        fs: String,
        /// Volume label
        #[arg(long)]
        label: Option<String>,
        /// Confirm the erase (required)
        #[arg(long)]
        yes: bool,
        /// Format a regular file instead of a block device (tests/loopback)
        #[arg(long)]
        image: bool,
    },
    /// Report a drive's filesystem type and label without modifying it.
    CheckDrive {
        /// Target block device or image file
        target: String,
    },
    /// Measure audio loudness
    Loudness {
        /// Audio file: a WAV or a PCM MXF
        audio_file: String,
    },
    /// Equal-power crossfade join of two WAVs (dom#374)
    Crossfade {
        /// First (leading) WAV
        #[arg(long)]
        a: String,
        /// Second (trailing) WAV
        #[arg(long)]
        b: String,
        /// Output WAV
        #[arg(short, long)]
        output: String,
        /// Overlap length in seconds
        #[arg(long, default_value = "1.0")]
        overlap: f64,
    },
    /// Decode a mid-side channel pair to L/R in a WAV (dom#3020)
    MidSideDecode {
        /// Input WAV
        #[arg(short, long)]
        input: String,
        /// Output WAV
        #[arg(short, long)]
        output: String,
        /// Mid channel index (0-based); becomes left
        #[arg(long, default_value = "0")]
        mid: usize,
        /// Side channel index (0-based); becomes right
        #[arg(long, default_value = "1")]
        side: usize,
    },
    /// Generate QC report
    Report {
        /// DCP directory
        #[arg(long)]
        dcp: String,
        /// Output HTML file
        #[arg(short, long)]
        output: String,
        /// Decode every picture track to find black and frozen runs. Hours for
        /// a feature, since the J2K decode is ffmpeg's software one
        #[arg(long)]
        scan_picture: bool,
    },
    /// Start REST API server
    Serve {
        /// Listen address (host:port)
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        bind: String,
        /// Require this key in X-Api-Key or Authorization: Bearer on every
        /// request but /health. Without it the API is open to anyone who can
        /// reach the bind address.
        #[arg(long)]
        api_key: Option<String>,
    },
    /// Build a DCP from every master that lands in a watched folder
    Watch {
        /// Directory to watch for masters
        dir: String,
        /// Directory each package is written to, under the master's file stem
        #[arg(short, long)]
        output: String,
        /// POST a JSON notification to this URL when a package is built or fails
        #[arg(long)]
        webhook_url: Option<String>,
        /// Seconds between polls
        #[arg(
            long,
            default_value_t = dcpwizard_core::watch::DEFAULT_POLL_INTERVAL_SECONDS,
            value_parser = clap::value_parser!(u64).range(
                dcpwizard_core::watch::MINIMUM_POLL_INTERVAL_SECONDS..
            )
        )]
        interval: u64,
        /// Flags passed to `create` for every package, after a `--` separator
        #[arg(last = true)]
        create_arguments: Vec<String>,
    },
    /// Export a DCP picture MXF to a delivery format via ffmpeg
    Export {
        /// Input picture MXF
        #[arg(long)]
        input: String,
        /// Output file (or directory for image-sequence)
        #[arg(short, long)]
        output: String,
        /// Format: prores, h264, h265, dnxhr, image-sequence
        #[arg(long, default_value = "h264")]
        format: String,
        /// Quality CRF for h264/h265 (lower is better; default 18)
        #[arg(long, default_value = "18")]
        crf: u32,
        /// Optional sound MXF to mux into the output
        #[arg(long)]
        audio: Option<String>,
    },
    /// Generate shell completion
    Completion {
        /// Shell (bash|zsh|fish)
        #[arg(default_value = "bash")]
        shell: String,
    },
    /// Start job queue daemon
    Daemon,
    /// Manage job queue
    Batch {
        #[command(subcommand)]
        action: BatchAction,
    },
    /// Convert SRT subtitles to DCP XML (SMPTE Timed Text)
    SubtitleConvert {
        /// Input SRT file
        #[arg(short, long)]
        input: String,
        /// Output XML file
        #[arg(short, long)]
        output: String,
        /// Language code (e.g. "en", "fr", "de")
        #[arg(short, long, default_value = "en")]
        language: String,
        /// Frame rate for timecode conversion (24, 25, 30, 48)
        #[arg(long, default_value = "24")]
        fps: u32,
        /// Bottom-line position as a percentage up from the bottom of the screen
        #[arg(long, default_value = "8.0")]
        vposition: f64,
    },
    /// Extract timed text from a DCP or subtitle asset to SRT or plain text
    SubtitleExtract {
        /// Input DCP directory, or a subtitle asset (XML or timed-text MXF)
        #[arg(short, long)]
        input: String,
        /// Output file; .srt keeps timing, .txt is text only
        #[arg(short, long)]
        output: String,
    },
    /// Edit a standalone subtitle file: list cues, shift timing, or change a
    /// cue's text/timing, writing SRT back out (dom#828, dom#2071). It edits
    /// source subtitle files, never subtitles inside a finished DCP.
    SubtitleEdit {
        /// Input subtitle file (SRT/ASS/PAC/MKS/FCPXML/interop XML)
        #[arg(short, long)]
        input: String,
        /// Output SRT path (required for edits; omit with --list)
        #[arg(short, long)]
        output: Option<String>,
        /// List cues and exit without writing output
        #[arg(long)]
        list: bool,
        /// Shift every cue by this many milliseconds (may be negative)
        #[arg(long, allow_hyphen_values = true)]
        shift_ms: Option<i64>,
        /// 1-based cue index to edit with --text / --set-start-ms / --set-end-ms
        #[arg(long)]
        index: Option<usize>,
        /// New text for the --index cue
        #[arg(long)]
        text: Option<String>,
        /// New start time (ms) for the --index cue (with --set-end-ms)
        #[arg(long)]
        set_start_ms: Option<u64>,
        /// New end time (ms) for the --index cue (with --set-start-ms)
        #[arg(long)]
        set_end_ms: Option<u64>,
        /// Timecode rate for frame-based inputs (interop/PAC), default 24
        #[arg(long, default_value_t = 24)]
        fps: u32,
    },
    /// Burn subtitles into a video file, for a review copy rather than a
    /// package: a DCP burn goes through `create --burn-subtitle`
    #[command(alias = "burn-in")]
    Burnin {
        /// Input video file
        #[arg(short, long)]
        input: String,
        /// Subtitle file (SRT or ASS)
        #[arg(short, long)]
        subtitles: String,
        /// Output video file
        #[arg(short, long)]
        output: String,
        /// Font size for burn-in (default: 24)
        #[arg(long, default_value = "24")]
        font_size: u32,
        /// Text colour as RRGGBB hex; unset keeps the subtitle file's own
        #[arg(long)]
        colour: Option<String>,
        /// Placement: top, center or bottom (default: bottom)
        #[arg(long, default_value = "bottom")]
        position: String,
        /// Video encoder for the output, e.g. libx264 or prores_ks; unset leaves
        /// the choice to ffmpeg's guess from the output name
        #[arg(long)]
        video_codec: Option<String>,
        /// Constant rate factor, 0 being lossless x264/x265; unset keeps the
        /// encoder's default
        #[arg(long)]
        crf: Option<u32>,
    },
    /// Convert video to a target DCI container (scale/crop/letterbox)
    Convert {
        /// Input video file
        #[arg(short, long)]
        input: String,
        /// Output video file
        #[arg(short, long)]
        output: String,
        /// Target container: 2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, 4k-full
        #[arg(short, long)]
        target: String,
        /// Method: letterbox, crop, or scale
        #[arg(short, long, default_value = "letterbox")]
        method: String,
    },

    /// Create DCDM (Digital Cinema Distribution Master) X'Y'Z' sequence
    Dcdm {
        /// Input image sequence directory
        #[arg(short, long)]
        input: String,

        /// Output DCDM TIFF directory
        #[arg(short, long)]
        output: String,

        /// Source colour space (rec709, p3, p3d65, rec2020, xyz, logc)
        #[arg(short, long, default_value = "rec709")]
        colour_space: String,

        /// Optional 3D LUT for colour transform
        #[arg(long)]
        lut: Option<String>,
    },

    /// Convert colour space of images/video
    Colour {
        /// Input file or directory
        #[arg(short, long)]
        input: String,

        /// Output file or directory
        #[arg(short, long)]
        output: String,

        /// Source colour space (rec709, p3, p3d65, xyz, rec2020, aces, acescg, logc)
        #[arg(short, long)]
        source: String,

        /// Target colour space (rec709, p3, rec2020, xyz for DCDM, p3-d65 mastering)
        #[arg(short, long)]
        target: String,

        /// Optional 3D LUT file for custom transform
        #[arg(long)]
        lut: Option<String>,
    },

    /// Conform an EDL/xmeml timeline: parse, or (with --media-dir) resolve every
    /// reel to media and write a reel/asset plan + conform manifest
    Conform {
        /// Input timeline file (EDL, AAF, FCP XML, OTIO)
        #[arg(short, long)]
        input: String,

        /// Media directory: resolve each reel to a file here and assemble a plan
        #[arg(long)]
        media_dir: Option<String>,

        /// Output directory for the reel plan + conform manifest (with --media-dir)
        #[arg(short, long)]
        output: Option<String>,

        /// Output the parsed timeline as JSON (parse-only mode)
        #[arg(long)]
        json: bool,

        #[command(flatten)]
        signer_opts: SignerOpts,
    },

    /// Extract a frame from video/MXF as image
    #[command(name = "frame-extract")]
    FrameExtract {
        /// Input video/MXF file
        #[arg(short, long)]
        input: String,

        /// Frame number to extract
        #[arg(short, long, default_value = "0")]
        frame: u32,

        /// Output image file (png, jpg, tiff)
        #[arg(short, long)]
        output: String,

        /// Content key for encrypted picture essence, 32 hex chars. Other users
        /// of the machine can read it from the process list: prefer --keys-json
        #[arg(long, conflicts_with = "keys_json")]
        key: Option<String>,

        /// dcpwizard KEYS.json holding the content key for encrypted essence
        #[arg(long)]
        keys_json: Option<String>,
    },

    /// Inject Dolby Vision RPU into HEVC stream
    #[command(name = "dv-inject")]
    DvInject {
        /// Input HEVC file
        #[arg(short, long)]
        input: String,

        /// RPU file (.bin)
        #[arg(short, long)]
        rpu: String,

        /// Output file
        #[arg(short, long)]
        output: String,
    },

    /// Inject HDR10 static metadata
    #[command(name = "hdr10-inject")]
    Hdr10Inject {
        /// Input video file
        #[arg(short, long)]
        input: String,

        /// Output video file
        #[arg(short, long)]
        output: String,

        /// Max content light level (MaxCLL)
        #[arg(long, default_value = "1000")]
        max_cll: u16,

        /// Max frame average light level (MaxFALL)
        #[arg(long, default_value = "400")]
        max_fall: u16,
    },

    /// Tone map a video between HDR10, HLG and SDR
    #[command(name = "hdr-convert")]
    HdrConvert {
        /// Input video file
        #[arg(short, long)]
        input: String,

        /// Output video file
        #[arg(short, long)]
        output: String,

        /// Grade the output carries
        #[arg(short, long, value_enum)]
        target: HdrTargetArg,
    },

    /// Burn a visible watermark into an existing DCP's picture essence
    Watermark {
        /// Input DCP directory
        #[arg(short, long)]
        input: String,

        /// Output DCP directory (must differ from input)
        #[arg(short, long)]
        output: String,

        /// Watermark payload (distributor ID, serial, etc.) rendered visibly
        #[arg(short, long)]
        payload: String,

        #[arg(long, help = watermark_font_size_help())]
        font_size: Option<f32>,

        /// Text colour as RRGGBB or RRGGBBAA (default FFFFFF)
        #[arg(long)]
        colour: Option<String>,

        /// Placement: top, center or bottom (default bottom)
        #[arg(long, value_parser = ["top", "center", "bottom"])]
        position: Option<String>,

        /// TTF/OTF font to draw the mark with (default: a system font)
        #[arg(long)]
        font: Option<String>,

        /// Target picture bandwidth in Mbit/s for the marked picture; unset
        /// re-encodes at the source picture's own average bandwidth
        #[arg(long)]
        video_bit_rate: Option<u32>,

        /// KDM XML to decrypt an encrypted source (needs --recipient-key)
        #[arg(long)]
        kdm: Option<String>,

        /// Recipient RSA private key (PEM) matching --kdm
        #[arg(long)]
        recipient_key: Option<String>,

        /// dcpwizard KEYS.json, an alternative key source to --kdm
        #[arg(long)]
        keys: Option<String>,
    },

    /// Generate or inspect X.509 certificates for DCP encryption
    #[command(alias = "cert")]
    Certificate {
        #[command(subcommand)]
        action: CertAction,
    },

    /// Generate KDMs for multiple recipients in one pass
    #[command(name = "kdm-batch")]
    KdmBatch {
        /// CPL ID
        #[arg(long)]
        cpl_id: String,
        /// Content title
        #[arg(long)]
        content_title: String,
        /// Recipient certificate file (repeatable, one KDM generated per cert)
        #[arg(long = "cert")]
        certs: Vec<String>,
        /// Directory of recipient certificates; every *.pem/*.crt/*.cer in it
        /// gets a KDM. Combined with any --cert values.
        #[arg(long)]
        cert_dir: Option<String>,
        /// Cinema in the db: generate a KDM for every screen (repeatable)
        #[arg(long = "cinema")]
        cinemas: Vec<String>,
        /// Single screen selector "cinema/screen" from the db (repeatable)
        #[arg(long = "screen")]
        screens: Vec<String>,
        /// Cinema db file (default: XDG data dir)
        #[arg(long)]
        db: Option<String>,
        /// Signer leaf certificate file
        #[arg(long)]
        signer_cert: String,
        /// Signer private key file
        #[arg(long)]
        signer_key: String,
        /// Signer CA certificate above the leaf (repeatable)
        #[arg(long)]
        signer_chain: Vec<String>,
        /// Output directory for generated KDMs
        #[arg(short, long)]
        output_dir: String,
        /// Valid from (ISO 8601 or "now"). Overrides a --template start.
        #[arg(short = 'f', long)]
        valid_from: Option<String>,
        /// Valid to (ISO 8601 or relative duration). Overrides a --template end.
        #[arg(short = 't', long)]
        valid_to: Option<String>,
        /// Named validity template to expand the window from
        #[arg(long)]
        template: Option<String>,
        /// Validity templates file (default: XDG data dir)
        #[arg(long)]
        templates_file: Option<String>,
        /// KDM history log file (default: XDG data dir); every KDM is recorded
        #[arg(long)]
        history_file: Option<String>,
        /// Email each cinema its KDMs zipped (one email per cinema). Extra
        /// address(es) to add to every email (repeatable)
        #[arg(long = "email-to")]
        email_to: Vec<String>,
        /// SMTP config TOML for sending KDMs by email
        #[arg(long)]
        smtp_config: Option<String>,
        /// Ignore cinema contact emails; send only to --email-to (dom#2515)
        #[arg(long)]
        email_only_additional: bool,
        /// DCP keys file (KEYS.json from `create --encrypt`) whose content keys
        /// every generated KDM should carry.
        #[arg(long)]
        keys: Option<String>,
        /// KDM format: smpte (default) or interop (legacy, needs real-gear validation)
        #[arg(long, default_value = "smpte")]
        format: String,
        #[command(flatten)]
        kdm_options: KdmOptionArgs,
    },

    /// Manage the cinema/screen database
    Cinema {
        /// Cinema db file (default: XDG data dir)
        #[arg(long, global = true)]
        db: Option<String>,
        #[command(subcommand)]
        action: CinemaAction,
    },

    /// Show the KDM generation history log (dom#1014)
    #[command(name = "kdm-history")]
    KdmHistory {
        /// History log file (default: XDG data dir)
        #[arg(long)]
        history_file: Option<String>,
        /// Filter by content title substring
        #[arg(long)]
        title: Option<String>,
        /// Filter by recipient subject or serial substring
        #[arg(long)]
        recipient: Option<String>,
        /// Only records at or after this date/prefix (e.g. "2026-07")
        #[arg(long)]
        since: Option<String>,
        /// Only records at or before this date/prefix
        #[arg(long)]
        until: Option<String>,
    },

    /// Manage named KDM validity templates (dom#2424)
    #[command(name = "kdm-template")]
    KdmTemplate {
        /// Templates file (default: XDG data dir)
        #[arg(long, global = true)]
        templates_file: Option<String>,
        #[command(subcommand)]
        action: TemplateAction,
    },

    /// Manage the library of head idents, tail idents, rating cards and
    /// anti-piracy clips that `create --head-item` / `--tail-item` join on
    Library {
        /// Library folder (default: XDG data dir)
        #[arg(long, global = true)]
        library_dir: Option<String>,
        #[command(subcommand)]
        action: LibraryAction,
    },

    /// Download a projector/server certificate by vendor + serial (dom#2705)
    #[command(name = "cert-fetch")]
    CertFetch {
        /// Vendor: dolby/doremi, qube (anonymous); christie, gdc, barco
        /// (need --user/--password). Others must be obtained from the vendor.
        #[arg(long)]
        vendor: String,
        /// Server serial number
        #[arg(long)]
        serial: String,
        /// Device type (qube only, e.g. QXPD)
        #[arg(long = "type")]
        device_type: Option<String>,
        /// Vendor account user (christie/gdc/barco)
        #[arg(long)]
        user: Option<String>,
        /// Vendor account password (christie/gdc/barco); never logged
        #[arg(long)]
        password: Option<String>,
        /// Output PEM file for the downloaded certificate
        #[arg(short, long)]
        output: String,
    },

    /// Package a trailer (ratings card + countdown leader + content)
    Trailer {
        /// Trailer content video file
        #[arg(short, long)]
        content: String,
        /// Output directory
        #[arg(short, long)]
        output: String,
        /// Trailer title (rendered on the ratings card)
        #[arg(long, default_value = "")]
        title: String,
        /// Rating text (e.g. "PG-13", "15")
        #[arg(long, default_value = "")]
        rating: String,
        /// Rating system: mpaa, bbfc, fsk, custom
        #[arg(long, default_value = "mpaa")]
        rating_system: String,
        /// Band colour: green, red, yellow
        #[arg(long, default_value = "green")]
        band: String,
        /// Countdown leader length in seconds
        #[arg(long, default_value = "8")]
        countdown: u32,
        /// Frame rate
        #[arg(long, default_value = "24")]
        fps: u32,
    },

    /// Generate DCP markers for a composition
    Markers {
        /// Composition length in frames
        #[arg(short, long)]
        frames: u64,
        /// Place a marker: LABEL=timecode (repeatable). LABEL is one of FFOC,
        /// LFOC, FFTC, LFTC, FFOI, LFOI, FFEC, LFEC, FFMC, LFMC; timecode is a
        /// frame number or HH:MM:SS:FF. Given markers replace the FFOC/LFOC
        /// default set.
        #[arg(long = "marker")]
        markers: Vec<String>,
        /// Frame rate for HH:MM:SS:FF timecodes (default 24)
        #[arg(long, default_value = "24")]
        fps: u32,
        /// Emit an XML MarkerList instead of a plain list
        #[arg(long)]
        xml: bool,
    },

    /// Check accessibility compliance of a DCP
    Accessibility {
        /// DCP directory
        dcp_dir: String,
        /// Accessibility standard to check against
        #[arg(short, long, value_enum, default_value_t = AccessibilityStandardArg::Cvaa)]
        standard: AccessibilityStandardArg,
    },

    /// Send a webhook notification (HTTP POST via curl)
    Webhook {
        /// Target URL
        #[arg(short, long)]
        url: String,
        /// Event type
        #[arg(long, default_value = "ping")]
        event: String,
        /// Job ID
        #[arg(long, default_value = "")]
        job_id: String,
        /// Shared secret (sent as X-Webhook-Secret)
        #[arg(long, default_value = "")]
        secret: String,
        /// JSON payload (defaults to a test ping body)
        #[arg(long, default_value = "")]
        payload: String,
    },

    /// Upload a finished DCP to a theatre management system over ftp or sftp
    Tms {
        /// Package directory to upload
        package: String,
        /// TMS config TOML holding the protocol, host, path, user and password
        /// (default: <config dir>/dcpwizard/tms.toml)
        #[arg(long)]
        tms_config: Option<String>,
    },

    /// Content version / delivery history tracker (SQLite)
    Version {
        #[command(subcommand)]
        action: VersionAction,
    },

    /// OV/VF version dashboard and distribution tracking
    Dashboard {
        #[command(subcommand)]
        action: DashboardAction,
    },
}

#[derive(Subcommand)]
enum VersionAction {
    /// Record a delivery
    Record {
        /// Tracker database file
        #[arg(long, default_value = "deliveries.db")]
        db: String,
        /// Package UUID
        #[arg(long)]
        package_uuid: String,
        /// Title
        #[arg(long, default_value = "")]
        title: String,
        /// Version label (e.g. OV, VF)
        #[arg(long, default_value = "")]
        version: String,
        /// Destination
        #[arg(long, default_value = "")]
        destination: String,
        /// Delivery method (e.g. hard_drive, satellite)
        #[arg(long, default_value = "")]
        method: String,
        /// Mark as verified
        #[arg(long)]
        verified: bool,
    },
    /// List recorded deliveries
    List {
        /// Tracker database file
        #[arg(long, default_value = "deliveries.db")]
        db: String,
        /// Filter by package UUID
        #[arg(long)]
        package_uuid: Option<String>,
        /// Filter by destination
        #[arg(long)]
        destination: Option<String>,
    },
    /// Export delivery history (format by extension: .json or .csv)
    Export {
        /// Tracker database file
        #[arg(long, default_value = "deliveries.db")]
        db: String,
        /// Output file (.json or .csv)
        #[arg(short, long)]
        output: String,
    },
}

#[derive(Subcommand)]
enum DashboardAction {
    /// Register a DCP version (OV or VF)
    Register {
        /// Version UUID
        #[arg(long)]
        uuid: String,
        /// Title
        #[arg(long)]
        title: String,
        /// Version type: OV or VF
        #[arg(long, default_value = "OV")]
        version_type: String,
        /// Territory (ISO 3166-1 alpha-2)
        #[arg(long, default_value = "")]
        territory: String,
        /// Language (RFC 5646)
        #[arg(long, default_value = "")]
        language: String,
        /// Standard: SMPTE or Interop
        #[arg(long, default_value = "SMPTE")]
        standard: String,
        /// DCP path
        #[arg(long, default_value = "")]
        dcp_path: String,
        /// Status: draft, released, archived
        #[arg(long, default_value = "draft")]
        status: String,
        /// KDM recipient theatre (repeatable)
        #[arg(long = "kdm-recipient")]
        kdm_recipients: Vec<String>,
    },
    /// List registered versions
    List {
        /// Filter by territory
        #[arg(long)]
        territory: Option<String>,
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
    },
    /// Update a version's status
    Status {
        /// Version UUID
        #[arg(long)]
        uuid: String,
        /// New status
        #[arg(long)]
        status: String,
    },
    /// Export the distribution matrix as CSV
    Matrix {
        /// Output CSV file
        #[arg(short, long)]
        output: String,
    },
    /// Start the dashboard HTTP server
    Serve {
        /// Listen port
        #[arg(short, long, default_value = "9090")]
        port: u32,
        /// Bind address
        #[arg(short, long, default_value = "127.0.0.1")]
        bind: String,
    },
}

#[derive(Subcommand)]
enum CertAction {
    /// Generate a full certificate chain (root → intermediate → signer)
    Chain {
        /// Organization name for the certificates
        #[arg(long)]
        organization: String,
        /// Output directory for generated certificates
        #[arg(short, long)]
        output: String,
    },
    /// Generate a single certificate
    Generate {
        /// Certificate type: root, intermediate, leaf, signer
        #[arg(short = 't', long, default_value = "signer")]
        cert_type: String,
        /// Common Name (CN)
        #[arg(long)]
        cn: String,
        /// Organization
        #[arg(long, default_value = "")]
        organization: String,
        /// Output certificate file
        #[arg(long)]
        output_cert: String,
        /// Output private key file
        #[arg(long)]
        output_key: String,
        /// Issuer certificate (required for non-root)
        #[arg(long)]
        issuer_cert: Option<String>,
        /// Issuer private key (required for non-root)
        #[arg(long)]
        issuer_key: Option<String>,
        /// Key size in bits
        #[arg(long, default_value = "2048")]
        key_bits: u32,
        /// Validity in days
        #[arg(long, default_value = "3650")]
        validity_days: u32,
    },
    /// Inspect a certificate file and show its details
    Inspect {
        /// Path to PEM certificate file
        cert_file: String,
    },
}

#[derive(Subcommand)]
enum CinemaAction {
    /// Add a cinema
    Add {
        /// Cinema name
        #[arg(long)]
        name: String,
        /// Contact email (repeatable)
        #[arg(long = "email")]
        emails: Vec<String>,
        /// Free-text notes
        #[arg(long, default_value = "")]
        notes: String,
    },
    /// List cinemas and their screens
    List,
    /// Remove a cinema
    Remove {
        #[arg(long)]
        name: String,
    },
    /// Add a screen with its recipient certificate
    AddScreen {
        /// Cinema name
        #[arg(long)]
        cinema: String,
        /// Screen name
        #[arg(long)]
        name: String,
        /// Recipient certificate (PEM/CRT file)
        #[arg(long)]
        cert: String,
        /// Embed the certificate PEM in the db instead of storing the path
        #[arg(long)]
        inline: bool,
    },
    /// Remove a screen from a cinema
    RemoveScreen {
        #[arg(long)]
        cinema: String,
        #[arg(long)]
        name: String,
    },
    /// Search cinemas/screens by name or certificate serial/thumbprint (dom#2707)
    Search {
        /// Query substring
        query: String,
    },
    /// Import a facility from an FLM-x file (dom#239)
    ImportFlm {
        /// FLM XML file
        file: String,
    },
}

#[derive(Subcommand)]
enum TemplateAction {
    /// Add a validity template
    Add {
        /// Template name
        #[arg(long)]
        name: String,
        /// Offset from now to the start (e.g. "0 days", "2 days")
        #[arg(long, default_value = "")]
        start_offset: String,
        /// Window length (e.g. "1 week", "180 days")
        #[arg(long)]
        duration: String,
        /// UTC offset for emitted timestamps (e.g. "+02:00"); empty = UTC
        #[arg(long, default_value = "")]
        tz_offset: String,
    },
    /// List validity templates
    List,
    /// Remove a validity template
    Remove {
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand)]
enum LibraryAction {
    /// Copy media into the library
    Add {
        /// Video or image file to import
        #[arg(long)]
        file: String,
        /// Name to file it under, and the name `create` addresses it by
        #[arg(long)]
        name: String,
        /// head-ident, tail-ident, rating-card or anti-piracy
        #[arg(long)]
        kind: String,
        /// How long a still image is held, in seconds. A video carries its own
        /// length and takes no duration.
        #[arg(long)]
        duration: Option<f64>,
    },
    /// List what the library holds
    List,
    /// Drop an item and the media copied in with it
    Remove {
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand)]
enum BatchAction {
    /// List all jobs
    List,
    /// Submit a new job
    Add {
        /// Job type (create-dcp|verify-dcp|export-dcp|import-video|encode-j2k|wrap-mxf|copy-to-drive)
        #[arg(short = 'T', long)]
        r#type: String,
        /// Job parameters (JSON string)
        #[arg(short, long)]
        params: String,
    },
    /// Cancel a job
    Cancel {
        /// Job ID to cancel
        id: String,
    },
}

/// Where the time inside an encode went, as one line. postkit renders the four
/// phase clocks off `PipelineProgress`, and `create` encodes through the grok
/// pipeline, which reports the same clocks on `EncodeProgress`.
fn print_encode_progress(progress: &postkit::pipeline::PipelineProgress) {
    let percent = if progress.total_frames > 0 {
        (progress.frame as f64 / progress.total_frames as f64) * 100.0
    } else {
        0.0
    };
    eprint!(
        "\r[encode] {}/{} frames ({:.0}%) {:.1} fps   ",
        progress.frame, progress.total_frames, percent, progress.fps
    );
}

fn encode_phase_breakdown(progress: &postkit::grok_encoder::EncodeProgress) -> String {
    postkit::pipeline::PipelineProgress {
        stage: "encode".to_string(),
        message: String::new(),
        frame: progress.frames_encoded,
        total_frames: progress.total_frames,
        fps: progress.fps,
        elapsed_secs: progress.elapsed_secs,
        percent: 0.0,
        decode_wait_secs: progress.decode_wait_secs,
        prepare_secs: progress.prepare_secs,
        encode_secs: progress.encode_secs,
        write_secs: progress.write_secs,
    }
    .phase_breakdown()
}

fn parse_colour_space(s: &str) -> postkit::colour::ColourSpace {
    postkit::colour::parse_colour_space(s).unwrap_or_else(|| {
        tracing::warn!("Unknown colour space '{s}', defaulting to Rec709");
        postkit::colour::ColourSpace::Rec709
    })
}

/// Map a `colour --target` string to a dcdm-module target (X'Y'Z' DCDM or P3-D65
/// mastering). Returns None for ffmpeg colorspace targets (rec709/p3/rec2020).
fn parse_dcdm_target(s: &str) -> Option<postkit::dcdm::DcdmTarget> {
    match s.to_lowercase().as_str() {
        "xyz" | "ciexyz" => Some(postkit::dcdm::DcdmTarget::Xyz),
        "p3-d65" | "p3d65" => Some(postkit::dcdm::DcdmTarget::P3D65),
        _ => None,
    }
}

// ── create-time helpers (container dims, reel splits, input range) ───────────

/// Resolve container dimensions from a preset name or a custom WxH.
///
/// `dims` (e.g. "1920x1080") wins over a `preset`; both absent yields (0,0)
/// meaning "use the full-container resolution default". Custom dims are validated
/// against the 2K/4K bounds.
fn resolve_container(
    preset: Option<&str>,
    dims: Option<&str>,
    is_4k: bool,
) -> Result<(u32, u32), String> {
    if let Some(spec) = dims {
        let (w, h) = spec
            .split_once(['x', 'X'])
            .ok_or_else(|| format!("--container-dims '{spec}' must be WxH (e.g. 1920x1080)"))?;
        let w: u32 = w
            .trim()
            .parse()
            .map_err(|_| format!("invalid width in --container-dims '{spec}'"))?;
        let h: u32 = h
            .trim()
            .parse()
            .map_err(|_| format!("invalid height in --container-dims '{spec}'"))?;
        dcpwizard_core::dcp::validate_container_dims(w, h, is_4k)?;
        return Ok((w, h));
    }
    match preset {
        Some("2k-scope") => Ok((2048, 858)),
        Some("2k-flat") => Ok((1998, 1080)),
        Some("2k-full") => Ok((2048, 1080)),
        Some("4k-scope") => Ok((4096, 1716)),
        Some("4k-flat") => Ok((3996, 2160)),
        Some("4k-full") => Ok((4096, 2160)),
        Some(value) => Err(format!("Unknown container: {value}")),
        None => Ok((0, 0)),
    }
}

/// ffprobe chapter boundaries for `video` at `fps`, or a loud error.
fn video_chapter_boundaries(video: &Path, fps: u32) -> Result<Vec<u64>, String> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_chapters"])
        .arg(video)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    if !out.status.success() {
        return Err("ffprobe failed to read chapters".into());
    }
    let json = String::from_utf8_lossy(&out.stdout);
    dcpwizard_core::reel::parse_chapter_starts(&json, fps)
}

/// Resolve reel-split boundaries from --split-at timecodes or --split-chapters.
fn resolve_reel_splits(
    split_at: Option<&str>,
    split_chapters: bool,
    chapter_video: Option<&Path>,
    fps: u32,
) -> Result<Vec<u64>, String> {
    if let Some(spec) = split_at {
        let mut frames = Vec::new();
        for tc in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            frames.push(dcpwizard_core::reel::parse_timecode(tc, fps)?);
        }
        if frames.is_empty() {
            return Err("--split-at needs at least one timecode".into());
        }
        return Ok(frames);
    }
    if split_chapters {
        let video = chapter_video
            .ok_or("--split-chapters needs a video input to read chapter marks from")?;
        return video_chapter_boundaries(video, fps);
    }
    Ok(Vec::new())
}

/// Re-encode `video` to a lossless intermediate that forces the given decode
/// `range` (full|legal), so the downstream ffmpeg raw-RGB decode is correct even
/// when the source range metadata is wrong or missing. Returns the intermediate path.
fn normalize_input_range(video: &Path, range: &str, out_dir: &Path) -> Result<PathBuf, String> {
    // ffmpeg's scale in_range names: full<->pc, legal<->tv/mpeg
    let in_range = if range == "full" { "full" } else { "tv" };
    let out = out_dir.join("range_corrected.mkv");
    let status = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(video)
        .arg("-vf")
        .arg(format!("scale=in_range={in_range}:out_range=full"))
        .args(["-c:v", "ffv1", "-level", "3", "-pix_fmt", "gbrp16le", "-an"])
        .arg(&out)
        .status()
        .map_err(|e| format!("failed to run ffmpeg for input-range correction: {e}"))?;
    if !status.success() {
        return Err("ffmpeg input-range correction failed".into());
    }
    Ok(out)
}

// a package that was never written keeps the frames and the resume state behind it
fn remove_intermediates_if_packaged(output_dir: &Path, handed_in: &Path, code: i32) -> i32 {
    if code == 0 {
        dcpwizard_core::intermediates::remove_intermediates(output_dir, &[handed_in]);
        return code;
    }
    match dcpwizard_core::encode_qol::EncodeState::load(output_dir) {
        Some(_) => tracing::warn!(
            "packaging failed: the encoded frames are still in {}, --resume reuses them",
            output_dir.display()
        ),
        None => tracing::warn!(
            "packaging failed: the encoded frames are still in {}",
            output_dir.display()
        ),
    }
    code
}

/// Build the combined sign-language sound track (ISDCF Doc 13): VP9-pack the
/// sign video onto channel 15 over the main audio. Returns the combined WAV and
/// the leading main-audio channel count for the SLVS MCA config.
fn build_sign_language_audio(
    slvs_video: &str,
    main_audio: Option<&Path>,
    min_frames: u64,
    fps: u32,
    output_dir: &Path,
) -> Result<(PathBuf, u32), String> {
    let combined = output_dir.join("slvs_sound.wav");
    let main_channels = dcpwizard_core::sign_language::build_slvs_sound(
        &PathBuf::from(slvs_video),
        main_audio,
        min_frames,
        fps,
        &combined,
    )?;
    Ok((combined, main_channels))
}

/// What `resolve_container` answers when neither `--container` nor
/// `--container-dims` was given.
const NO_CONTAINER: (u32, u32) = (0, 0);

/// The raster `create` has to land the picture on: a named container is itself
/// the coded raster, and `--twok`/`--fourk` alone force the full DCI frame.
fn encode_geometry(
    twok: bool,
    fourk: bool,
    container: (u32, u32),
) -> dcpwizard_core::source_picture::EncodeGeometry {
    if container != NO_CONTAINER {
        return dcpwizard_core::source_picture::EncodeGeometry {
            forced_raster: Some(container),
            container: Some(container),
        };
    }
    let forced = fourk
        .then_some(dcpwizard_core::Resolution::FourK)
        .or(twok.then_some(dcpwizard_core::Resolution::TwoK));
    dcpwizard_core::source_picture::EncodeGeometry {
        forced_raster: forced.map(|resolution| (resolution.width(), resolution.height())),
        container: None,
    }
}

/// The DCI family the package declares: a named container names its own, and
/// `--twok`/`--fourk` decide only when there is no container.
fn config_resolution(fourk: bool, container: (u32, u32)) -> dcpwizard_core::Resolution {
    if container != NO_CONTAINER {
        return dcpwizard_core::Resolution::for_raster(container.0, container.1);
    }
    if fourk {
        dcpwizard_core::Resolution::FourK
    } else {
        dcpwizard_core::Resolution::TwoK
    }
}

/// One `-vf` argument, or None when nothing has to happen while decoding.
fn join_decode_filters(picture: &[String], extra: Option<&str>) -> Option<String> {
    let mut filters: Vec<&str> = picture.iter().map(String::as_str).collect();
    filters.extend(extra);
    (!filters.is_empty()).then(|| filters.join(","))
}

/// How a still is decoded: the raster it is encoded at, and the filters that
/// bring it there.
fn still_picture(
    image: &Path,
    picture_options: &dcpwizard_core::source_picture::SourcePictureOptions,
    geometry: &dcpwizard_core::source_picture::EncodeGeometry,
) -> Result<(u32, u32, Vec<String>), String> {
    let info = dcpwizard_core::probe::probe_video(image)
        .ok_or_else(|| format!("cannot read the size of {}", image.display()))?;
    let resolved = dcpwizard_core::source_picture::resolve_picture(
        picture_options,
        image,
        info.width,
        info.height,
        geometry,
        false,
    )?;
    tracing::info!("Picture: {}", resolved.plan.describe());
    Ok((
        resolved.encode_width,
        resolved.encode_height,
        resolved.plan.filters,
    ))
}

/// A resolved `--trim-start`/`--trim-end` request: what is cut, and what is left.
#[derive(Debug, Clone, Copy, Default)]
struct TrimPlan {
    start_frames: u64,
    end_frames: u64,
    /// frames surviving the trim, zero when nothing was asked for
    kept_frames: u64,
}

impl TrimPlan {
    fn is_active(&self) -> bool {
        self.kept_frames > 0
    }

    fn source_trim(&self) -> dcpwizard_core::subtitle::SourceTrim {
        dcpwizard_core::subtitle::SourceTrim {
            start_frames: self.start_frames,
            kept_frames: self.kept_frames,
        }
    }

    /// The window the encoder is given, so nothing outside it is compressed.
    /// None where the picture is not encoded here and [`Self::apply`] links the
    /// kept codestreams instead.
    fn encode_window(&self, picture: &Path) -> Option<postkit::encode::FrameRange> {
        dcpwizard_core::trim::encode_window(picture, self.start_frames, self.kept_frames)
    }

    /// Trim the sound to the kept window, and the picture too when `window` says
    /// the encoder was not given it. Returns the frame directory the package
    /// takes, which is `j2k_dir` unless the codestreams were relinked.
    fn apply(
        &self,
        j2k_dir: &Path,
        out_dir: &Path,
        audio: Option<PathBuf>,
        fps: u32,
        window: Option<postkit::encode::FrameRange>,
    ) -> Result<(PathBuf, Option<PathBuf>), String> {
        if !self.is_active() {
            return Ok((j2k_dir.to_path_buf(), audio));
        }
        let picture = match window {
            Some(_) => j2k_dir.to_path_buf(),
            None => {
                let kept = dcpwizard_core::trim::link_trimmed_frames(
                    j2k_dir,
                    self.start_frames,
                    self.end_frames,
                    out_dir,
                )?;
                tracing::info!("Trimmed the picture to {kept} frame(s)");
                out_dir.to_path_buf()
            }
        };
        let audio = match audio {
            Some(input) => {
                let out = out_dir.with_extension("wav");
                dcpwizard_core::trim::trim_wav(
                    &input,
                    self.start_frames,
                    self.kept_frames,
                    fps,
                    &out,
                )?;
                Some(out)
            }
            None => None,
        };
        Ok((picture, audio))
    }
}

/// Resolve `--trim-start`/`--trim-end` against a `total_frames` source at `fps`.
fn resolve_trim(
    start: Option<&str>,
    end: Option<&str>,
    total_frames: u64,
    fps: u32,
) -> Result<TrimPlan, String> {
    let parse = |spec: Option<&str>, flag: &str| match spec {
        Some(spec) => {
            dcpwizard_core::pad::parse_pad_frames(spec, fps).map_err(|e| format!("{flag}: {e}"))
        }
        None => Ok(0),
    };
    let start_frames = parse(start, "--trim-start")?;
    let end_frames = parse(end, "--trim-end")?;
    if start_frames + end_frames == 0 {
        return Ok(TrimPlan::default());
    }
    let kept_frames = dcpwizard_core::trim::kept_frames(total_frames, start_frames, end_frames)?;
    Ok(TrimPlan {
        start_frames,
        end_frames,
        kept_frames,
    })
}

/// Create-time audio processing (W5): the mix matrix, then filename channel
/// routing when `audio` is a directory (dom#2134), then stereo->5.1 upmix
/// (dom#921/#1080), then the
/// picture/sound delay, then loudness normalization (dom#1382). The delay comes
/// before loudness so normalisation measures the silence that actually ships.
/// Intermediates go under `work_dir` (a scratch dir). Runs before sign-language
/// packing and any pull-up.
#[allow(clippy::too_many_arguments)]
fn prepare_create_audio(
    audio: Option<PathBuf>,
    audio_map: Option<&str>,
    upmix: Option<&str>,
    delay_ms: Option<i64>,
    loudness_target: Option<&str>,
    true_peak_ceiling: Option<f64>,
    adjust: &dcpwizard_core::audio_adjust::AudioAdjust,
    work_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(mut path) = audio else {
        return Ok(None);
    };

    // the map places every channel by hand, so it runs before anything that
    // moves channels for it
    if let Some(spec) = audio_map {
        std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
        let mapped = work_dir.join("mapped.wav");
        let applied = dcpwizard_core::audio_map::apply_audio_map(spec, &path, &mapped)?;
        tracing::info!(
            "Audio map: {} channels to {} over {} frames{}",
            applied.report.input_channels,
            applied.report.output_channels,
            applied.report.frames,
            if applied.pure_routing {
                ", bit-exact routing"
            } else {
                ""
            }
        );
        if applied.report.clipped_samples > 0 {
            tracing::warn!(
                "Audio map clipped {} sample(s): lower the cell gains",
                applied.report.clipped_samples
            );
        }
        path = mapped;
    }

    if path.is_dir() {
        std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
        let routed = work_dir.join("routed.wav");
        path = dcpwizard_core::audio_route::route_directory(&path, &routed)?;
        tracing::info!("Routed channel WAVs from the input directory by filename");
    }

    if let Some(v) = upmix {
        let variant = match v {
            "a" | "A" => postkit::upmix::Upmixer::A,
            "b" | "B" => postkit::upmix::Upmixer::B,
            other => return Err(format!("unknown upmix variant '{other}' (use a or b)")),
        };
        std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("upmix.wav");
        postkit::upmix::upmix_wav(variant, &path, &out).map_err(|e| e.to_string())?;
        tracing::info!("Upmixed stereo to 5.1 (variant {v})");
        path = out;
    }

    if let Some(delay_ms) = delay_ms.filter(|ms| *ms != 0) {
        std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("delayed.wav");
        path = dcpwizard_core::audio_adjust::apply_delay(&path, &out, delay_ms)?;
        tracing::info!("Delayed the sound by {delay_ms}ms against the picture");
    }

    if let Some(spec) = loudness_target {
        let target = dcpwizard_core::loudness::parse_loudness_target(spec)?;
        let ceiling =
            true_peak_ceiling.unwrap_or(dcpwizard_core::loudness::DEFAULT_TRUE_PEAK_CEILING_DBTP);
        std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("loudness.wav");
        let plan = dcpwizard_core::loudness::adjust_loudness(&path, &out, target, ceiling)
            .map_err(|e| e.to_string())?;
        tracing::info!(
            "Loudness adjusted {:.1} -> {:.1} dB (gain {:+.2} dB, peak {:.2} dBTP)",
            plan.measured_db,
            plan.target_db,
            plan.gain_db,
            plan.resulting_true_peak_dbtp,
        );
        path = out;
    }

    if !adjust.is_empty() {
        std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("adjusted.wav");
        let seconds = dcpwizard_core::audio_adjust::duration_seconds(&path)?;
        path = dcpwizard_core::audio_adjust::apply(&path, &out, adjust, seconds)?;
        tracing::info!("Applied audio gain/fades");
    }

    Ok(Some(path))
}

/// Validate the DCI HDR Addendum flag combo and the raised per-codestream cap.
/// The picture MXF is wrapped with TransferCharacteristic=ST 2084 in create_dcp;
/// this only rejects an unusable request up front. `hdr_dcdm_source` says the
/// master's own grade was read, which is the third path to PQ.
fn validate_hdr_dci(
    hdr_to_dci_lut: &Option<String>,
    hdr_already_pq: bool,
    hdr_dcdm_source: bool,
    frame_rate: Option<u32>,
    video_bit_rate: Option<u32>,
) {
    use dcpwizard_core::hdr;
    if hdr_to_dci_lut.is_none() && !hdr_already_pq && !hdr_dcdm_source {
        tracing::error!(
            "--hdr-dci needs the source path to PQ: encode from an HDR master (--hdr-source names \
             its grade), or pass --hdr-to-dci-lut or --hdr-already-pq"
        );
        std::process::exit(1);
    }
    let rate = frame_rate.unwrap_or(24);
    let cap = hdr::hdr_codestream_byte_cap(rate);
    if let Some(mbps) = video_bit_rate
        && mbps > hdr::HDR_MAX_MBPS
    {
        tracing::error!(
            "--hdr-dci caps the codestream at {cap} bytes/frame ({} Mbit/s at {rate} fps); requested {mbps} Mbit/s exceeds it",
            hdr::HDR_MAX_MBPS
        );
        std::process::exit(1);
    }
}

// ── KDM distribution helpers ────────────────────────────────────────────────

/// resolve a validity window: a --template supplies the base window, explicit
/// --valid-from/--valid-to override it, and the fallback is now .. +2 weeks.
fn resolve_window(
    valid_from: Option<String>,
    valid_to: Option<String>,
    template: Option<String>,
    templates_file: Option<String>,
) -> Result<(String, String), String> {
    let (mut vf, mut vt) = ("now".to_string(), "2 weeks".to_string());
    if let Some(name) = template {
        let path = templates_file
            .map(PathBuf::from)
            .unwrap_or_else(dcpwizard_core::store::default_templates_path);
        let store = dcpwizard_core::kdm_template::TemplateStore::load(&path)?;
        let t = store
            .get(&name)
            .ok_or_else(|| format!("template '{name}' not found"))?;
        let (f, t2) = t.expand()?;
        vf = f;
        vt = t2;
    }
    if let Some(f) = valid_from {
        vf = f;
    }
    if let Some(t) = valid_to {
        vt = t;
    }
    Ok((vf, vt))
}

fn history_path(history_file: Option<String>) -> PathBuf {
    history_file
        .map(PathBuf::from)
        .unwrap_or_else(dcpwizard_core::store::default_history_path)
}

fn cinema_db_path(db: Option<String>) -> PathBuf {
    db.map(PathBuf::from)
        .unwrap_or_else(dcpwizard_core::store::default_db_path)
}

fn templates_db_path(templates_file: Option<String>) -> PathBuf {
    templates_file
        .map(PathBuf::from)
        .unwrap_or_else(dcpwizard_core::store::default_templates_path)
}

fn send_kdm_email(
    cfg_path: &str,
    cinema: &str,
    title: &str,
    to: &[String],
    files: &[PathBuf],
) -> Result<(), String> {
    if to.is_empty() {
        return Err("no email recipients: pass --email-to".to_string());
    }
    let cfg = dcpwizard_core::email::SmtpConfig::load(Path::new(cfg_path))?;
    dcpwizard_core::email::send_kdms(&cfg, cinema, title, to, files)
}

fn load_tms_config(
    cfg_path: Option<&str>,
) -> Result<dcpwizard_core::tms_upload::TmsConfig, String> {
    let path = cfg_path
        .map(PathBuf::from)
        .unwrap_or_else(dcpwizard_core::tms_upload::default_config_path);
    dcpwizard_core::tms_upload::load_config(&path)
}

fn sanitize_dir_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn xml_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("xml"))
        .collect();
    v.sort();
    v
}

/// a set of recipients that share one delivery email (one cinema, or the loose
/// --cert/--cert-dir group with an empty name).
struct BatchGroup {
    name: String,
    emails: Vec<String>,
    cert_paths: Vec<PathBuf>,
}

struct KdmBatchArgs {
    cpl_id: String,
    content_title: String,
    certs: Vec<String>,
    cert_dir: Option<String>,
    cinemas: Vec<String>,
    screens: Vec<String>,
    db: Option<String>,
    signer_cert: String,
    signer_key: String,
    signer_chain: Vec<String>,
    output_dir: String,
    valid_from: Option<String>,
    valid_to: Option<String>,
    template: Option<String>,
    templates_file: Option<String>,
    history_file: Option<String>,
    email_to: Vec<String>,
    smtp_config: Option<String>,
    email_only_additional: bool,
    keys: Option<String>,
    format: String,
    options: dcpwizard_core::kdm::KdmOptions,
}

fn run_kdm_batch(a: KdmBatchArgs) -> i32 {
    // a batch carries no device list (see the empty one passed below), so a
    // formulation that lists devices has nothing to name. Refused here, where the
    // reason can be spelled out, rather than by resolve_formulation per recipient.
    if let Some(formulation) = a.options.formulation.filter(|f| f.lists_supplied_devices()) {
        tracing::error!(
            "--formulation {formulation} lists the devices named by --device-cert, and kdm-batch \
             takes none: a batch spans cinemas and one device list cannot fit them all. Use \
             --formulation {}, or issue the restricted KDMs one at a time with `kdm --device-cert`",
            formulation.device_list_counterpart()
        );
        return 1;
    }
    let format = match dcpwizard_core::kdm::parse_format(&a.format) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let (valid_from, valid_to) =
        match resolve_window(a.valid_from, a.valid_to, a.template, a.templates_file) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        };
    let content_keys = match a.keys {
        Some(path) => match dcpwizard_core::kdm::load_content_keys(&PathBuf::from(path), &a.cpl_id)
        {
            Ok(k) => k,
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        },
        None => Vec::new(),
    };

    // materialized inline certs live here for the whole batch.
    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("cannot create temp dir: {e}");
            return 1;
        }
    };

    // loose certs from --cert / --cert-dir go into an unnamed group.
    let mut loose: Vec<String> = a.certs;
    if let Some(dir) = a.cert_dir {
        match dcpwizard_core::kdm::certs_in_dir(&PathBuf::from(&dir)) {
            Ok(found) => loose.extend(found),
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        }
    }

    let mut groups: Vec<BatchGroup> = Vec::new();
    if !loose.is_empty() {
        groups.push(BatchGroup {
            name: String::new(),
            emails: Vec::new(),
            cert_paths: loose.into_iter().map(PathBuf::from).collect(),
        });
    }

    // db-resolved cinema/screen recipients, grouped by cinema.
    if !a.cinemas.is_empty() || !a.screens.is_empty() {
        let db = match dcpwizard_core::cinema::CinemaDb::load(&cinema_db_path(a.db)) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        };
        let recips = match db.resolve(&a.cinemas, &a.screens, tmp.path()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("{e}");
                return 1;
            }
        };
        for r in recips {
            match groups.iter_mut().find(|g| g.name == r.cinema) {
                Some(g) => g.cert_paths.push(r.cert_path),
                None => groups.push(BatchGroup {
                    name: r.cinema,
                    emails: r.emails,
                    cert_paths: vec![r.cert_path],
                }),
            }
        }
    }

    if groups.iter().all(|g| g.cert_paths.is_empty()) {
        tracing::error!("No recipients (use --cert, --cert-dir, --cinema or --screen)");
        return 1;
    }

    let history = Some(history_path(a.history_file));
    let signer_cert = PathBuf::from(&a.signer_cert);
    let signer_key = PathBuf::from(&a.signer_key);
    let signer_chain: Vec<PathBuf> = a.signer_chain.iter().map(PathBuf::from).collect();
    let output_root = PathBuf::from(&a.output_dir);

    // without email: one flat batch into output_dir (preserves prior behaviour).
    if a.smtp_config.is_none() {
        let all: Vec<PathBuf> = groups.into_iter().flat_map(|g| g.cert_paths).collect();
        return dcpwizard_core::kdm::generate_kdm_batch(
            a.cpl_id,
            a.content_title,
            all,
            signer_cert,
            signer_key,
            signer_chain,
            valid_from,
            valid_to,
            content_keys,
            output_root,
            format,
            None,
            history,
            // no --device-cert here: a batch spans cinemas, and one device list
            // shared across them would lock every recipient to someone else's gear
            Vec::new(),
            a.options,
        );
    }

    // with email: one email per group (dom#2516), each with that group's KDMs
    // zipped. multiple groups get their own subdir so files don't collide.
    let cfg_path = a.smtp_config.unwrap();
    let multi = groups.len() > 1;
    let mut failures = 0;
    for g in &groups {
        let out_dir = if multi {
            let sub = if g.name.is_empty() {
                "additional".to_string()
            } else {
                sanitize_dir_name(&g.name)
            };
            output_root.join(sub)
        } else {
            output_root.clone()
        };
        let code = dcpwizard_core::kdm::generate_kdm_batch(
            a.cpl_id.clone(),
            a.content_title.clone(),
            g.cert_paths.clone(),
            signer_cert.clone(),
            signer_key.clone(),
            signer_chain.clone(),
            valid_from.clone(),
            valid_to.clone(),
            content_keys.clone(),
            out_dir.clone(),
            format,
            None,
            history.clone(),
            Vec::new(),
            a.options.clone(),
        );
        if code != 0 {
            failures += 1;
            continue;
        }
        // recipients: cinema contacts (unless only-additional) plus --email-to.
        let mut to: Vec<String> = if a.email_only_additional {
            Vec::new()
        } else {
            g.emails.clone()
        };
        for e in &a.email_to {
            if !to.contains(e) {
                to.push(e.clone());
            }
        }
        let files = xml_files_in(&out_dir);
        let label = if g.name.is_empty() {
            "additional recipients"
        } else {
            &g.name
        };
        match send_kdm_email(&cfg_path, &g.name, &a.content_title, &to, &files) {
            Ok(()) => tracing::info!("emailed {} KDM(s) for {label}", files.len()),
            Err(e) => {
                tracing::error!("{label}: {e}");
                failures += 1;
            }
        }
    }
    if failures == 0 { 0 } else { 1 }
}

fn run_cinema(db: Option<String>, action: CinemaAction) -> i32 {
    let path = cinema_db_path(db);
    let mut store = match dcpwizard_core::cinema::CinemaDb::load(&path) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    use dcpwizard_core::cinema::CertSource;
    let mutated: Result<bool, String> = match action {
        CinemaAction::Add {
            name,
            emails,
            notes,
        } => store.add_cinema(&name, emails, notes).map(|_| true),
        CinemaAction::Remove { name } => store.remove_cinema(&name).map(|_| true),
        CinemaAction::AddScreen {
            cinema,
            name,
            cert,
            inline,
        } => {
            let src = if inline {
                match std::fs::read_to_string(&cert) {
                    Ok(pem) => CertSource::Inline(pem),
                    Err(e) => {
                        tracing::error!("cannot read {cert}: {e}");
                        return 1;
                    }
                }
            } else {
                CertSource::Path(PathBuf::from(&cert))
            };
            store.add_screen(&cinema, &name, src).map(|_| true)
        }
        CinemaAction::RemoveScreen { cinema, name } => {
            store.remove_screen(&cinema, &name).map(|_| true)
        }
        CinemaAction::ImportFlm { file } => match store.import_flm(&PathBuf::from(&file)) {
            Ok(summary) => {
                println!("imported {summary}");
                Ok(true)
            }
            Err(e) => Err(e),
        },
        CinemaAction::List => {
            for c in &store.cinemas {
                println!("{} [{}]", c.name, c.emails.join(", "));
                for s in &c.screens {
                    println!("  - {} (serial {})", s.name, s.cert_serial);
                }
            }
            Ok(false)
        }
        CinemaAction::Search { query } => {
            let hits = store.search(&query);
            if hits.is_empty() {
                println!("no matches for '{query}'");
            }
            for (cinema, screen) in hits {
                if screen.is_empty() {
                    println!("{cinema}");
                } else {
                    println!("{cinema} / {screen}");
                }
            }
            Ok(false)
        }
    };
    match mutated {
        Ok(true) => {
            if let Err(e) = store.save(&path) {
                tracing::error!("{e}");
                return 1;
            }
            0
        }
        Ok(false) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

fn run_kdm_history(
    history_file: Option<String>,
    title: Option<String>,
    recipient: Option<String>,
    since: Option<String>,
    until: Option<String>,
) -> i32 {
    let path = history_path(history_file);
    let all = match dcpwizard_core::kdm_log::read_all(&path) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let recs = dcpwizard_core::kdm_log::filter(
        all,
        title.as_deref(),
        recipient.as_deref(),
        since.as_deref(),
        until.as_deref(),
    );
    if recs.is_empty() {
        println!("no history records");
    }
    for r in recs {
        println!(
            "{}  {}  {}  serial={}  {}..{}  {}",
            r.timestamp,
            r.format,
            r.content_title,
            r.recipient_serial,
            r.valid_from,
            r.valid_to,
            r.output_path
        );
    }
    0
}

/// The library the CLI reads: the folder named, else the app's own.
fn open_library(library_dir: Option<String>) -> dcpwizard_core::library::Library {
    match library_dir {
        Some(dir) => dcpwizard_core::library::Library::open_at(PathBuf::from(dir)),
        None => dcpwizard_core::library::Library::open(),
    }
}

fn run_library(library_dir: Option<String>, action: LibraryAction) -> i32 {
    use dcpwizard_core::library::{LibraryItemKind, item_frames};
    let library = open_library(library_dir);
    let result: Result<(), String> = match action {
        LibraryAction::Add {
            file,
            name,
            kind,
            duration,
        } => LibraryItemKind::parse(&kind).and_then(|kind| {
            let item = library.import(Path::new(&file), &name, kind, duration)?;
            println!(
                "Added '{}' ({}) — {}x{}, {:.2}s, {}",
                item.name,
                item.kind,
                item.width,
                item.height,
                item.seconds,
                if item.has_audio {
                    "with audio"
                } else {
                    "silent"
                }
            );
            Ok(())
        }),
        LibraryAction::List => library.items().map(|items| {
            if items.is_empty() {
                println!("The library is empty. Add items with `dcpwizard library add`.");
            }
            for item in &items {
                println!(
                    "{}\t{}\t{}x{}\t{:.2}s ({} frames at 24 fps)\t{}",
                    item.name,
                    item.kind,
                    item.width,
                    item.height,
                    item.seconds,
                    item_frames(item, DEFAULT_FRAME_RATE),
                    if item.has_audio {
                        "with audio"
                    } else {
                        "silent"
                    }
                );
            }
        }),
        LibraryAction::Remove { name } => library.remove(&name).inspect(|()| {
            println!("Removed '{name}'");
        }),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

fn run_kdm_template(templates_file: Option<String>, action: TemplateAction) -> i32 {
    let path = templates_db_path(templates_file);
    let mut store = match dcpwizard_core::kdm_template::TemplateStore::load(&path) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    use dcpwizard_core::kdm_template::Template;
    let mutated: Result<bool, String> = match action {
        TemplateAction::Add {
            name,
            start_offset,
            duration,
            tz_offset,
        } => {
            let t = Template {
                name,
                start_offset,
                duration,
                tz_offset,
            };
            // validate the window parses before persisting
            match t.expand() {
                Ok(_) => store.add(t).map(|_| true),
                Err(e) => Err(e),
            }
        }
        TemplateAction::Remove { name } => store.remove(&name).map(|_| true),
        TemplateAction::List => {
            for t in &store.templates {
                let tz = if t.tz_offset.is_empty() {
                    "UTC"
                } else {
                    &t.tz_offset
                };
                println!(
                    "{}: start +[{}] duration {} ({})",
                    t.name,
                    if t.start_offset.is_empty() {
                        "now"
                    } else {
                        &t.start_offset
                    },
                    t.duration,
                    tz
                );
            }
            Ok(false)
        }
    };
    match mutated {
        Ok(true) => {
            if let Err(e) = store.save(&path) {
                tracing::error!("{e}");
                return 1;
            }
            0
        }
        Ok(false) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

/// Resolve a parsed timeline against a media dir into a reel plan, write the
/// plan + conform manifest, and print the assembled reels. Per-reel encode/wrap
/// into a DCP is the remaining step; the plan is the executable hand-off.
fn run_conform_assembly(
    timeline: &postkit::conform::Timeline,
    media_dir: &str,
    output: Option<&str>,
    signer: Option<&dcpwizard_core::package_signature::PackageSigner>,
) -> i32 {
    let media = PathBuf::from(media_dir);
    let out = PathBuf::from(output.unwrap_or("conform_out"));
    let plan = match dcpwizard_core::conform::build_reel_plan(timeline, &media) {
        Ok(p) => p,
        Err(missing) => {
            for m in &missing {
                tracing::error!("unresolved reel (no matching media in {media_dir}): {m}");
            }
            return 1;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&out) {
        tracing::error!("cannot create output dir: {e}");
        return 1;
    }
    if let Err(e) = dcpwizard_core::conform::write_conform_manifest(timeline, &out) {
        tracing::error!("cannot write conform manifest: {e}");
        return 1;
    }
    // keep the reel plan as an artifact next to the manifest
    let plan_path = out.join("conform_plan.json");
    let plan_json = serde_json::to_string_pretty(&plan).unwrap_or_default();
    if let Err(e) = std::fs::write(&plan_path, plan_json) {
        tracing::error!("cannot write reel plan: {e}");
        return 1;
    }
    println!(
        "Conforming {} reel(s) from \"{}\" -> {}",
        plan.reels.len(),
        plan.title,
        out.display()
    );
    for r in &plan.reels {
        println!(
            "  {} [{}] {} ({}..{})",
            r.reel_name,
            r.track_type,
            r.media_path.display(),
            r.source_in,
            r.source_out
        );
    }

    // drive the plan to a finished multi-reel DCP (per-reel encode + wrap + assembly)
    dcpwizard_core::conform::assemble_dcp(&plan, &out, signer)
}

const TRAILER_FALLBACK_TITLE: &str = "Trailer";

fn trailer_content_title(title: &str, content: &Path) -> String {
    let given = title.trim();
    if !given.is_empty() {
        return given.to_string();
    }
    content
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(TRAILER_FALLBACK_TITLE)
        .to_string()
}

/// Encode the packaged trailer mp4 to J2K and build a DCP (ContentKind=trailer)
/// in `<output_dir>/dcp`, reusing the same grok encode + create_dcp path as
/// `create --video`. The mp4 stays in place as the intermediate.
fn trailer_to_dcp(mp4: &Path, output_dir: &Path, fps_arg: u32, content_title: &str) -> i32 {
    use postkit::grok_encoder::{self, CompressParams, EncodeProgress};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    if let Err(e) = dcpwizard_core::probe::ensure_video_decodable(mp4) {
        tracing::error!("{e}");
        return 1;
    }
    let info = dcpwizard_core::probe::probe_video(mp4);
    let (width, height, total_frames) = info
        .as_ref()
        .map(|v| (v.width, v.height, v.total_frames))
        .unwrap_or((1920, 1080, 0));
    let fps = if fps_arg > 0 { fps_arg } else { 24 };

    let j2k_dir = output_dir.join("j2k");
    if let Err(e) = std::fs::create_dir_all(&j2k_dir) {
        tracing::error!("Failed to create j2k dir: {e}");
        return 1;
    }
    let params = CompressParams {
        compression_ratio: 10.0,
        edit_rate: postkit::encode::FrameRate::whole(fps),
        apply_xyz_transform: true,
        ..CompressParams::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let result = grok_encoder::encode_video_pipeline(
        mp4,
        &j2k_dir,
        &params,
        total_frames as u64,
        width,
        height,
        &cancel,
        |_p: EncodeProgress| {},
    );
    if !result.success {
        tracing::error!("Trailer encode failed: {}", result.error);
        return 1;
    }

    // demux audio if the packaged trailer carries any (card/leader are silent)
    let audio_path = {
        let wav = output_dir.join("audio_demux.wav");
        let demux = std::process::Command::new("ffmpeg")
            .args(["-y", "-i"])
            .arg(mp4)
            .args(["-vn", "-acodec", "pcm_s24le", "-ar", "48000"])
            .arg(&wav)
            .output();
        match demux {
            Ok(o) if o.status.success() => Some(wav),
            _ => None,
        }
    };

    let dcp_dir = output_dir.join("dcp");
    let config = dcpwizard_core::dcp::DcpConfig {
        title: content_title.to_string(),
        standard: dcpwizard_core::Standard::Smpte,
        resolution: dcpwizard_core::Resolution::TwoK,
        content_type: dcpwizard_core::ContentType::Trailer,
        frame_rate_num: fps,
        frame_rate_den: 1,
        // declare the CPL container at the encoded essence size
        container_width: width,
        container_height: height,
        output_dir: dcp_dir.clone(),
        j2k_dir: Some(j2k_dir),
        audio_path,
        subtitle_language: "en".to_string(),
        ..Default::default()
    };
    let code = dcpwizard_core::dcp::create_dcp(&config);
    if code == 0 {
        tracing::info!("Trailer DCP created: {}", dcp_dir.display());
        0
    } else {
        tracing::error!("Trailer DCP creation failed");
        1
    }
}

fn run_cert_fetch(
    vendor: String,
    serial: String,
    device_type: Option<String>,
    user: Option<String>,
    password: Option<String>,
    output: String,
) -> i32 {
    let v = match dcpwizard_core::cert_fetch::parse_vendor(&vendor) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let creds = match (user, password) {
        (Some(user), Some(password)) => {
            Some(dcpwizard_core::cert_fetch::Credentials { user, password })
        }
        (None, None) => None,
        _ => {
            tracing::error!("pass both --user and --password, or neither");
            return 1;
        }
    };
    match dcpwizard_core::cert_fetch::fetch(
        v,
        &serial,
        device_type.as_deref(),
        creds.as_ref(),
        &PathBuf::from(&output),
    ) {
        Ok(summary) => {
            println!("downloaded {summary} -> {output}");
            0
        }
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn run_preferences_command(action: &PreferencesCommand) -> i32 {
    let result = match action {
        PreferencesCommand::Show => dcpwizard_core::preferences::load_preferences()
            .map_err(|error| error.to_string())
            .and_then(|preferences| {
                serde_json::to_string_pretty(&preferences).map_err(|error| error.to_string())
            }),
        PreferencesCommand::Path => Ok(dcpwizard_core::preferences::preferences_path()
            .display()
            .to_string()),
        PreferencesCommand::Reset => dcpwizard_core::preferences::reset_preferences()
            .map_err(|error| error.to_string())
            .and_then(|preferences| {
                serde_json::to_string_pretty(&preferences).map_err(|error| error.to_string())
            }),
        PreferencesCommand::Set { name, value } => {
            dcpwizard_core::preferences::set_preference(name, value).and_then(|preferences| {
                serde_json::to_string_pretty(&preferences).map_err(|error| error.to_string())
            })
        }
    };

    match result {
        Ok(output) => {
            println!("{output}");
            0
        }
        Err(error) => {
            tracing::error!("{error}");
            1
        }
    }
}

fn main() {
    // Windows debug builds overflow the default 1MB stack due to large clap
    // derive enum (102 args across 34 subcommands). Spawn with 8MB stack.
    let thread = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(run)
        .expect("failed to spawn main thread");
    thread.join().unwrap();
}

fn run() {
    // User-friendly panic handler
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unexpected error".to_string()
        };
        let location = info
            .location()
            .map(|l| format!(" ({}:{})", l.file(), l.line()))
            .unwrap_or_default();
        eprintln!("\nerror: dcpwizard crashed: {payload}{location}");
        eprintln!(
            "This is a bug. Please report it at https://github.com/PostPerfection/dcpwizard/issues"
        );
        eprintln!("Include the command you ran and any input files if possible.");
        if std::env::var("RUST_BACKTRACE").is_ok() {
            eprintln!(
                "\nBacktrace:\n{:?}",
                std::backtrace::Backtrace::force_capture()
            );
        } else {
            eprintln!("Set RUST_BACKTRACE=1 for a detailed backtrace.");
        }
    }));

    let cli = Cli::parse();

    let filter = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    if let Commands::Preferences { action } = &cli.command {
        std::process::exit(run_preferences_command(action));
    }

    let preferences = match dcpwizard_core::preferences::load_preferences() {
        Ok(preferences) => preferences,
        Err(error) => {
            tracing::error!("could not load preferences: {error}");
            std::process::exit(1);
        }
    };
    let gpu_enabled = if cli.no_gpu {
        false
    } else {
        cli.gpu || preferences.gpu
    };
    let license = cli
        .license
        .as_deref()
        .or_else(|| nonempty(&preferences.gpu_license));
    let registration_url = cli
        .registration_url
        .as_deref()
        .or_else(|| nonempty(&preferences.gpu_registration_url));

    if (cli.license.is_some() || cli.registration_url.is_some()) && !gpu_enabled {
        eprintln!("--license and --registration-url require GPU encoding");
        std::process::exit(2);
    }
    if registration_url.is_some() && license.is_none() {
        eprintln!("--registration-url requires --license");
        std::process::exit(2);
    }

    postkit::grok_encoder::initialize(0);

    if gpu_enabled
        && let Err(e) =
            postkit::grok_encoder::use_gpu_with_authentication(license, registration_url)
    {
        // the preference file is the GUI's too
        if cli.gpu {
            tracing::error!("{e}");
            std::process::exit(1);
        }
        tracing::warn!("{e} The GPU preference is on, so this run stays on the CPU.");
    }

    let code = match cli.command {
        Commands::Preferences { .. } => unreachable!(),
        Commands::Create {
            title,
            video,
            audio,
            audio_input_order,
            hdr_to_dci_lut,
            allow_generic_hdr_tonemap,
            hdr_dci,
            hdr_source,
            hdr_peak_nits,
            hdr_already_pq,
            sign_language_video,
            sign_language_lang,
            subtitle,
            versions,
            subtitle_language,
            subtitle_qol,
            watermark_opts,
            output,
            standard,
            encrypt,
            key_out,
            frame_rate,
            twok,
            fourk,
            container,
            threads,
            video_bit_rate,
            quality_psnr,
            reel_length,
            profile,
            right_eye,
            atmos,
            hi_channel,
            vi_channel,
            container_dims,
            split_at,
            split_chapters,
            input_range,
            markers,
            padding,
            composition_metadata,
            audio_qol,
            source_opts,
            picture_opts,
            isdcf_naming,
            signer_opts,
            library_items,
            delivery,
            check,
        } => {
            let CreateDelivery {
                upload_to_tms,
                tms_config,
            } = *delivery;
            let CreateAudioQol {
                loudness_target,
                true_peak_ceiling,
                upmix,
                audio_channels,
                audio_map,
                audio_gain,
                audio_delay,
                audio_fade_in,
                audio_fade_out,
                video_fade_in,
                video_fade_out,
                start_at,
                resume,
                shutdown_when_done,
            } = *audio_qol;
            let CreatePadding {
                pad_head,
                pad_tail,
                pad_color,
            } = *padding;
            let CreateSourceOpts {
                source_colourspace,
                trim_start,
                trim_end,
                still_length,
            } = *source_opts;
            let picture_options = match picture_opts.resolve() {
                Ok(options) => options,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            let naming = match isdcf_naming.resolve() {
                Ok(naming) => naming,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            let CreateCompositionMetadata {
                content_type,
                release_territory,
                version_number,
                chain,
                distributor,
                facility,
                luminance,
            } = *composition_metadata;
            // fail loud on shutdown up front, before the long encode, so the
            // user is not left with a finished DCP and no power-off.
            if shutdown_when_done
                && let Err(e) = dcpwizard_core::encode_qol::resolve_shutdown_command()
            {
                tracing::error!("{e}");
                std::process::exit(1);
            }
            // same for the TMS config: an unreadable one must fail before the
            // encode, not after it.
            let tms_target = if upload_to_tms {
                match load_tms_config(tms_config.as_deref()) {
                    Ok(config) => Some(config),
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                }
            } else {
                None
            };
            // same for the signer: an unusable key or certificate must fail
            // before the encode, not after it.
            let package_signer = package_signer(&signer_opts);
            if let Some(signer) = package_signer.as_ref()
                && let Err(e) = signer.check_usable()
            {
                tracing::error!("{e}");
                std::process::exit(1);
            }
            // resolve the library items before the encode: a name the library
            // does not hold, or media it has lost, fails in a second
            let CreateLibraryItems {
                head_items,
                tail_items,
                library_dir,
            } = *library_items;
            let (head_items, tail_items) = {
                let library = open_library(library_dir);
                let resolve = |names: &[String]| {
                    dcpwizard_core::library_reel::attach_by_name(&library, names).unwrap_or_else(
                        |e| {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        },
                    )
                };
                (resolve(&head_items), resolve(&tail_items))
            };
            // scheduled start: block until the wall-clock time before any work.
            if let Some(spec) = start_at.as_deref() {
                match dcpwizard_core::encode_qol::parse_start_at(
                    spec,
                    dcpwizard_core::encode_qol::now_local(),
                ) {
                    Ok(target) => {
                        tracing::info!("Scheduled start: waiting until {target}");
                        dcpwizard_core::encode_qol::wait_until(target);
                    }
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                }
            }
            // resolved up front: it costs nothing and a bad value used to be
            // caught only after the whole encode had run
            let (container_width, container_height) =
                match resolve_container(container.as_deref(), container_dims.as_deref(), fourk) {
                    Ok(dims) => dims,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
            let audio_adjust = dcpwizard_core::audio_adjust::AudioAdjust {
                gain_db: audio_gain,
                fade_in_seconds: audio_fade_in,
                fade_out_seconds: audio_fade_out,
            };
            // source colour space, and the HDR flags that decide the encoder
            // transform themselves: two answers to one question, so refuse both.
            let source_space =
                match dcpwizard_core::encode::parse_source_colourspace(&source_colourspace) {
                    Ok(space) => space,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
            let xyz_route = match dcpwizard_core::encode::xyz_route(source_space) {
                Ok(route) => route,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            if !xyz_route.compressor_transform()
                && (hdr_dci || hdr_to_dci_lut.is_some() || allow_generic_hdr_tonemap)
            {
                tracing::error!(
                    "--source-colourspace {source_colourspace} and the HDR source flags both decide \
                     the encoder's colour transform: pass one or the other"
                );
                std::process::exit(1);
            }
            let parsed_luminance = match luminance
                .as_deref()
                .map(dcpwizard_core::cpl::Luminance::parse)
            {
                Some(Ok(l)) => Some(l),
                Some(Err(e)) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
                None => None,
            };
            let video_path = PathBuf::from(&video);
            let output_dir = PathBuf::from(&output);
            let std_val = if standard == "interop" {
                dcpwizard_core::Standard::Interop
            } else {
                dcpwizard_core::Standard::Smpte
            };
            let audio_input_order = match audio_input_order.as_str() {
                "dcp" => dcpwizard_core::mxf_wrap::AudioInputOrder::Canonical51,
                "lrc-ls-rs-lfe" => dcpwizard_core::mxf_wrap::AudioInputOrder::LrcLsRsLfe,
                value => {
                    tracing::error!("Unknown audio input order: {value}");
                    std::process::exit(1);
                }
            };
            // --audio-map places every channel by hand, and each of these places
            // channels its own way, so two of them would fight over the same
            // lanes.
            if audio_map.is_some() {
                let competing = [
                    (
                        audio_input_order == dcpwizard_core::mxf_wrap::AudioInputOrder::LrcLsRsLfe,
                        "--audio-input-order lrc-ls-rs-lfe",
                    ),
                    (upmix.is_some(), "--upmix"),
                    (
                        audio.as_deref().map(|a| Path::new(a).is_dir()) == Some(true),
                        "a channel WAV directory as --audio",
                    ),
                ];
                if let Some((_, name)) = competing.into_iter().find(|(set, _)| *set) {
                    tracing::error!(
                        "--audio-map and {name} both decide which DCP lane each channel lands \
                         on: pass one or the other"
                    );
                    std::process::exit(1);
                }
            }

            // parse the multi-version manifest up front so a bad manifest fails
            // before any encoding
            let versions_specs = match versions.as_deref() {
                Some(path) => match dcpwizard_core::versions::load_versions(&PathBuf::from(path)) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                },
                None => None,
            };

            // Resolve delivery profile and apply its presets as defaults; explicit
            // flags still win.
            let profile = match profile.as_deref() {
                Some(name) => match dcpwizard_core::profiles::get_profile(name) {
                    Some(p) => {
                        tracing::info!("Using profile '{}': {}", p.name, p.description);
                        Some(p)
                    }
                    None => {
                        let names: Vec<String> = dcpwizard_core::profiles::all_profiles()
                            .into_iter()
                            .map(|p| p.name)
                            .collect();
                        tracing::error!(
                            "Unknown profile '{name}'. Available: {}",
                            names.join(", ")
                        );
                        std::process::exit(1);
                    }
                },
                None => None,
            };
            let fourk = fourk
                || (!twok
                    && profile
                        .as_ref()
                        .map(|p| p.resolution_width >= 4096)
                        .unwrap_or(false));
            let package_resolution = config_resolution(fourk, (container_width, container_height));
            let package_is_four_k = package_resolution == dcpwizard_core::Resolution::FourK;
            let frame_rate = frame_rate.or_else(|| profile.as_ref().map(|p| p.frame_rate));
            let video_bit_rate =
                video_bit_rate.or_else(|| profile.as_ref().map(|p| p.bitrate_mbps));

            let quality_psnr_range = dcpwizard_core::encode::MINIMUM_QUALITY_PSNR_DB
                ..=dcpwizard_core::encode::MAXIMUM_QUALITY_PSNR_DB;
            if let Some(db) = quality_psnr
                && !quality_psnr_range.contains(&db)
            {
                tracing::error!(
                    "--quality-psnr {db} is outside the range: at least {} and at most {} dB",
                    quality_psnr_range.start(),
                    quality_psnr_range.end()
                );
                std::process::exit(1);
            }

            // Detect if input is a video file (not a J2K directory)
            let is_video_file = video_path.is_file()
                && video_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| {
                        matches!(
                            e.to_lowercase().as_str(),
                            "mp4"
                                | "mov"
                                | "mkv"
                                | "avi"
                                | "mxf"
                                | "ts"
                                | "m2ts"
                                | "mpg"
                                | "mpeg"
                                | "webm"
                                | "apv"
                        )
                    })
                    .unwrap_or(false);

            // DCI HDR Addendum: the master's grade and peak luminance, unless a
            // LUT or --hdr-already-pq already hands the encoder PQ.
            let hdr_dcdm_colour =
                if hdr_dci && is_video_file && hdr_to_dci_lut.is_none() && !hdr_already_pq {
                    match dcpwizard_core::hdr::plan_hdr_dcdm(
                        &video_path,
                        hdr_source.map(Into::into),
                        hdr_peak_nits,
                    ) {
                        Ok(colour) => Some(colour),
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    }
                } else {
                    None
                };
            if hdr_dci {
                validate_hdr_dci(
                    &hdr_to_dci_lut,
                    hdr_already_pq,
                    hdr_dcdm_colour.is_some(),
                    frame_rate,
                    video_bit_rate,
                );
            }

            // the colour the encoder's frames arrive in, which the burn, the
            // mark and the preflight all read
            let source_colour = match (&hdr_dcdm_colour, hdr_to_dci_lut.as_deref(), hdr_already_pq)
            {
                (Some(colour), _, _) => colour.clone(),
                (None, Some(lut), _) => postkit::encode::SourceColour::DciLut(PathBuf::from(lut)),
                (None, None, true) => postkit::encode::SourceColour::AlreadyPq,
                (None, None, false) => xyz_route.source_colour(),
            };

            // a single image is a third input shape beside a video and a
            // codestream directory, and it is the only one with no length of its
            // own, so the hold has to be asked for and cannot be asked for
            // anywhere else.
            let still_input = postkit::still::is_still_image(&video_path);
            // a codestream directory is picture that is already encoded: no
            // transform runs over it, so a colour space here would be ignored
            if !is_video_file && !still_input {
                if let Err(e) =
                    dcpwizard_core::encode::check_precompressed_colourspace(source_space)
                {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
                if let Err(e) =
                    dcpwizard_core::source_picture::check_precompressed_picture(&picture_options)
                {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            }
            if still_input && still_length.is_none() {
                tracing::error!(
                    "--video {} is a single image and has no length: pass --still-length",
                    video_path.display()
                );
                std::process::exit(1);
            }
            if !still_input && still_length.is_some() {
                tracing::error!(
                    "--still-length applies to a single-image --video; a video or codestream \
                     directory carries its own length"
                );
                std::process::exit(1);
            }
            if still_input && (trim_start.is_some() || trim_end.is_some()) {
                tracing::error!(
                    "--trim-start/--trim-end cut a source down; a still is held for exactly \
                     --still-length, so shorten that instead"
                );
                std::process::exit(1);
            }

            // a burn draws display-RGB text onto decoded frames, so refuse
            // every route that hands the encoder X'Y'Z' or nothing to draw on,
            // before anything is encoded
            let packaged_timed_text: Vec<&Path> =
                [subtitle.as_deref(), subtitle_qol.ccap.as_deref()]
                    .into_iter()
                    .flatten()
                    .map(Path::new)
                    .collect();
            let input_is_codestreams = !is_video_file && !still_input;
            if let Some(ref burn) = subtitle_qol.burn_subtitle
                && let Err(e) = dcpwizard_core::subtitle::check_burn_supported(
                    Path::new(burn),
                    &packaged_timed_text,
                    &source_colour,
                    input_is_codestreams,
                )
            {
                tracing::error!("{e}");
                std::process::exit(1);
            }

            // the mark is drawn in display RGB and onto decoded frames, the
            // same two things a subtitle burn needs
            if watermark_opts.watermark.is_some() {
                if input_is_codestreams {
                    tracing::error!(
                        "--watermark needs frames to draw on, and a J2K directory is already \
                         compressed: mark the finished DCP with the watermark command instead"
                    );
                    std::process::exit(1);
                }
                if let Some(frames) = dcpwizard_core::encode::frames_not_display_rgb(&source_colour)
                {
                    tracing::error!(
                        "--watermark draws in display RGB, but this source reaches the encoder \
                         as {frames}: mark the finished DCP with the watermark command instead"
                    );
                    std::process::exit(1);
                }
            }

            // an appearance flag styles one track, so refuse the ones whose
            // track was never asked for instead of packaging without them
            let unstyled_subtitle = subtitle_qol.named_appearance();
            if subtitle.is_none() && !unstyled_subtitle.is_empty() {
                tracing::error!(
                    "{} styles the timed-text track --subtitle packages, and --ccap keeps the \
                     default appearance: pass --subtitle",
                    unstyled_subtitle.join(", ")
                );
                std::process::exit(1);
            }
            let unstyled_watermark = watermark_opts.named_appearance();
            if watermark_opts.watermark.is_none() && !unstyled_watermark.is_empty() {
                tracing::error!(
                    "{} styles the mark --watermark burns into the picture: pass --watermark",
                    unstyled_watermark.join(", ")
                );
                std::process::exit(1);
            }
            let watermark = match watermark_options(
                watermark_opts.watermark.as_deref(),
                watermark_opts.watermark_font_size,
                watermark_opts.watermark_colour.as_deref(),
                watermark_opts.watermark_position.as_deref(),
                &CREATE_WATERMARK_FLAGS,
            ) {
                Ok(options) => options,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            let unstyled_burn = subtitle_qol.burn_appearance.named();
            if subtitle_qol.burn_subtitle.is_none() && !unstyled_burn.is_empty() {
                tracing::error!(
                    "{} styles the text --burn-subtitle draws into the picture: pass \
                     --burn-subtitle",
                    unstyled_burn.join(", ")
                );
                std::process::exit(1);
            }
            let burn_style = match subtitle_qol.burn_appearance.overrides() {
                Ok(style) => style,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            let subtitle_appearance = match subtitle_qol.appearance() {
                Ok(appearance) => appearance,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };

            let burnt_in_subtitle = subtitle_qol.burn_subtitle.is_some();
            let CreateSubtitleOpts {
                subtitle_halign,
                subtitle_valign,
                subtitle_vposition,
                subtitle_zposition,
                subtitle_rtl,
                subtitle_wrap,
                subtitle_font,
                subtitle_no_subset,
                burn_subtitle,
                burn_subtitle_font,
                ccap,
                ccap_language,
                ..
            } = *subtitle_qol;
            let subtitle_rtl_mode = match subtitle_rtl.as_str() {
                "on" => dcpwizard_core::subtitle::RtlMode::On,
                "off" => dcpwizard_core::subtitle::RtlMode::Off,
                _ => dcpwizard_core::subtitle::RtlMode::Auto,
            };
            let subtitle_opts = dcpwizard_core::subtitle::SubtitleOptions {
                halign: subtitle_halign,
                valign: subtitle_valign,
                vposition: subtitle_vposition,
                zposition: subtitle_zposition,
                rtl: subtitle_rtl_mode,
                wrap_cols: subtitle_wrap,
                font_path: subtitle_font.map(PathBuf::from),
                no_subset: subtitle_no_subset,
                appearance: subtitle_appearance,
            };

            // the edit rate is settled inside each branch, and the cue timings
            // are read against it, so the burn is built there
            let build_subtitle_burn = |fps: postkit::encode::FrameRate| -> Option<
                std::sync::Arc<postkit::subtitle_raster::SubtitleBurn>,
            > {
                let path = burn_subtitle.as_deref()?;
                match dcpwizard_core::subtitle::prepare_subtitle_burn(
                    Path::new(path),
                    burn_subtitle_font.as_deref().map(Path::new),
                    fps,
                    &burn_style,
                ) {
                    Ok(burn) => Some(burn),
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                }
            };
            // the mark is shaped with the font --burn-subtitle draws with, so
            // one --create run has one typeface in the picture
            let build_watermark = |fps: postkit::encode::FrameRate| -> Option<
                std::sync::Arc<postkit::subtitle_raster::SubtitleBurn>,
            > {
                let options = watermark.as_ref()?;
                match dcpwizard_core::watermark::watermark_burn(
                    options,
                    burn_subtitle_font.as_deref().map(Path::new),
                    fps.as_f64(),
                ) {
                    Ok(mark) => Some(mark),
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                }
            };

            // one description of the job, checked and hinted before anything is
            // encoded. The frame count costs a decode, so the source is probed
            // once here and the video branch reuses it.
            let source_info = (is_video_file || still_input)
                .then(|| dcpwizard_core::probe::probe_video(&video_path))
                .flatten();
            let plan_fps = frame_rate.unwrap_or_else(|| {
                source_info
                    .as_ref()
                    .filter(|_| is_video_file)
                    .map(|info| {
                        dcpwizard_core::hfr::source_rate_to_dcp(info.fps_num, info.fps_den).0
                    })
                    .unwrap_or(DEFAULT_FRAME_RATE)
            });
            let duration_frames = |spec: Option<&str>, flag: &str| -> u64 {
                match spec {
                    Some(spec) => match dcpwizard_core::pad::parse_pad_frames(spec, plan_fps) {
                        Ok(frames) => frames,
                        Err(e) => {
                            tracing::error!("{flag}: {e}");
                            std::process::exit(1);
                        }
                    },
                    None => 0,
                }
            };
            let plan = dcpwizard_core::preflight::CreatePlan {
                picture: video_path.clone(),
                picture_kind: match (still_input, is_video_file) {
                    (true, _) => dcpwizard_core::preflight::PictureKind::Still,
                    (_, true) => dcpwizard_core::preflight::PictureKind::Video,
                    _ => dcpwizard_core::preflight::PictureKind::Codestreams,
                },
                source: source_info.clone(),
                still_frames: duration_frames(still_length.as_deref(), "--still-length"),
                fps: plan_fps,
                picture_options: picture_options.clone(),
                geometry: encode_geometry(twok, fourk, (container_width, container_height)),
                trim_start_frames: duration_frames(trim_start.as_deref(), "--trim-start"),
                trim_end_frames: duration_frames(trim_end.as_deref(), "--trim-end"),
                pad_head_frames: duration_frames(pad_head.as_deref(), "--pad-head"),
                pad_tail_frames: duration_frames(pad_tail.as_deref(), "--pad-tail"),
                audio: audio.as_deref().map(PathBuf::from),
                audio_map: audio_map.clone(),
                upmix: upmix.is_some(),
                audio_channels,
                audio_language: naming.audio_language.clone(),
                subtitle: subtitle.as_deref().map(PathBuf::from),
                ccap: ccap.as_deref().map(PathBuf::from),
                burn_subtitle: burn_subtitle.as_deref().map(PathBuf::from),
                burn_subtitle_font: burn_subtitle_font.as_deref().map(PathBuf::from),
                burn_style: burn_style.clone(),
                source_colourspace: source_space,
                source_colour: source_colour.clone(),
                atmos: atmos.as_deref().map(PathBuf::from),
                markers: markers.clone(),
                standard: std_val,
                content_type: content_type
                    .as_deref()
                    .and_then(dcpwizard_core::ContentType::from_abbrev)
                    .unwrap_or_default(),
                encrypt,
                hdr_dci,
                video_bit_rate_mbps: video_bit_rate.unwrap_or(0),
                right_eye: right_eye.as_deref().map(PathBuf::from),
                four_k: package_is_four_k,
                reel_length_minutes: reel_length.unwrap_or(0),
                reel_split_frames: match resolve_reel_splits(
                    split_at.as_deref(),
                    split_chapters,
                    is_video_file.then_some(video_path.as_path()),
                    plan_fps,
                ) {
                    Ok(frames) => frames,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                },
                library_items: head_items.len() + tail_items.len(),
            };
            if let Err(e) = dcpwizard_core::preflight::check_before_encode(&plan) {
                tracing::error!("{e}");
                std::process::exit(1);
            }
            // the audio level hint measures the whole WAV, minutes on a feature,
            // so the hints run beside the encode and print before packaging
            let hints_pass = std::thread::spawn(move || dcpwizard_core::hints::gather_hints(&plan));
            let print_hints =
                |hints_pass: std::thread::JoinHandle<Vec<postkit::hints::Hint>>| -> usize {
                    let hints = hints_pass.join().expect("the hint pass does not panic");
                    for hint in &hints {
                        tracing::warn!("hint: {}", hint.text);
                    }
                    hints.len()
                };
            if check {
                println!(
                    "Pre-build check passed with {} hint(s); nothing was encoded or written",
                    print_hints(hints_pass)
                );
                return;
            }

            let code = if is_video_file {
                // Full pipeline: video → J2K encode → MXF wrap → DCP
                use postkit::grok_encoder::{self, CompressParams, EncodeProgress};
                use std::sync::Arc;
                use std::sync::atomic::AtomicBool;

                // fail loud if ffmpeg cannot decode the source codec (e.g. APV on
                // an older ffmpeg); the whole pipeline decodes through ffmpeg
                if let Err(e) = dcpwizard_core::probe::ensure_video_decodable(&video_path) {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }

                let _ = std::fs::create_dir_all(&output_dir);
                let j2k_dir = output_dir.join("j2k");
                let _ = std::fs::create_dir_all(&j2k_dir);

                // optional decode-range correction: re-decode the source at a
                // forced range into a lossless intermediate the encode reads from
                let range_src = if let Some(range) = input_range.as_deref() {
                    match normalize_input_range(&video_path, range, &output_dir) {
                        Ok(p) => {
                            tracing::info!("Forcing {range}-range decode of the source");
                            p
                        }
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    }
                } else {
                    video_path.clone()
                };

                let mut encode_video_path = range_src.clone();
                // the hdr-lut branch outputs x'y'z' already, and so does an
                // --source-colourspace xyz source. p3 and rec2020 are display
                // rgb grok cannot convert, so postkit transforms those frames
                // itself; everything else is grok's own dcdm transform
                let mut content_already_xyz =
                    matches!(xyz_route, dcpwizard_core::encode::XyzRoute::AlreadyXyz);
                let transform_source = hdr_dcdm_colour.as_ref().map_or_else(
                    || xyz_route.frame_transform(),
                    |colour| colour.frame_transform(),
                );
                let frame_transform = match transform_source {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
                // an HDR master postkit transforms itself needs no conversion pass
                let convert_hdr_source = hdr_dcdm_colour.is_none()
                    && !hdr_already_pq
                    && dcpwizard_core::dolby_vision::detect_hdr_type(&range_src)
                        != postkit::dolby_vision::HdrType::Sdr;
                if hdr_already_pq {
                    // the operator's assertion beats detection: a pq source can
                    // probe as sdr, and transforming it would stamp pq on frames
                    // that are no longer pq
                    content_already_xyz = true;
                } else if convert_hdr_source {
                    let converted = output_dir.join("hdr_to_dci_source.mov");
                    if let Some(lut) = hdr_to_dci_lut.as_ref() {
                        let lut = PathBuf::from(lut);
                        if !lut.is_file() {
                            tracing::error!("HDR-to-DCI LUT not found: {}", lut.display());
                            std::process::exit(1);
                        }
                        let opts = postkit::colour::ColourConvertOptions {
                            input: range_src.clone(),
                            output: converted.clone(),
                            source_space: postkit::colour::ColourSpace::Rec2020,
                            target_space: postkit::colour::ColourSpace::Xyz,
                            lut_path: Some(lut),
                        };
                        if let Err(e) = postkit::colour::convert_colour(&opts) {
                            tracing::error!("HDR-to-DCI LUT conversion failed: {e}");
                            std::process::exit(1);
                        }
                        content_already_xyz = true;
                    } else if allow_generic_hdr_tonemap {
                        tracing::warn!(
                            "Using generic FFmpeg HDR tone mapping. It is not suitable as a default delivery transform."
                        );
                        if dcpwizard_core::dolby_vision::convert_hdr(
                            &range_src,
                            postkit::dolby_vision::HdrType::Sdr,
                            &converted,
                        ) != 0
                        {
                            return;
                        }
                    } else {
                        tracing::error!(
                            "HDR source requires --hdr-to-dci-lut. Use --allow-generic-hdr-tonemap only for an explicitly accepted generic transform."
                        );
                        std::process::exit(1);
                    }
                    encode_video_path = converted;
                }

                tracing::info!("Detected video file input — using grok encoder");

                // Probe video for frame rate and resolution. The plan already
                // probed the source, so only a rewritten one is read again.
                let video_info = if encode_video_path == video_path {
                    source_info.clone()
                } else {
                    dcpwizard_core::probe::probe_video(&encode_video_path)
                };
                let Some(source_pixel_format) = video_info.as_ref().map(|info| info.pixel_format())
                else {
                    tracing::error!(
                        "ffprobe cannot read the input video: {}",
                        encode_video_path.display()
                    );
                    std::process::exit(1);
                };
                if dcpwizard_core::probe::pixel_format_has_alpha(&source_pixel_format.pix_fmt) {
                    tracing::error!(
                        "Input video has alpha. Composite it over an opaque background before creating a DCP."
                    );
                    std::process::exit(1);
                }
                let source_fps = video_info
                    .as_ref()
                    .map(|v| dcpwizard_core::hfr::source_rate_to_dcp(v.fps_num, v.fps_den).0)
                    .unwrap_or(24);
                let fps = frame_rate.unwrap_or(source_fps);
                let conform = video_info
                    .as_ref()
                    .map(|v| dcpwizard_core::hfr::conform_source_to_dcp(v.fps_num, v.fps_den, fps))
                    .unwrap_or_default();
                let (mut width, mut height, total_frames) = video_info
                    .as_ref()
                    .map(|v| (v.width, v.height, v.total_frames))
                    .unwrap_or((2048, 1080, 0));

                // head/tail trim of the source, resolved before the encode so a
                // trim that leaves nothing fails in a second rather than an hour
                let trim = match resolve_trim(
                    trim_start.as_deref(),
                    trim_end.as_deref(),
                    total_frames as u64,
                    fps,
                ) {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };

                // reject an illegal fps/resolution combo before the encode runs
                if let Err(e) = dcpwizard_core::hfr::validate_fps_resolution(
                    fps,
                    package_is_four_k,
                    std_val == dcpwizard_core::Standard::Smpte,
                ) {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }

                // the source is fitted onto the forced raster while it decodes,
                // so the encode raster is what the plan produces, never the
                // source size on its own
                let resolved_picture = match dcpwizard_core::source_picture::resolve_picture(
                    &picture_options,
                    &encode_video_path,
                    width,
                    height,
                    &encode_geometry(twok, fourk, (container_width, container_height)),
                    false,
                ) {
                    Ok(resolved) => resolved,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
                tracing::info!("Picture: {}", resolved_picture.plan.describe());
                width = resolved_picture.encode_width;
                height = resolved_picture.encode_height;

                if let Some(ref info) = video_info {
                    tracing::info!(
                        "Input: {}x{} @ {}/{} fps, ~{} frames",
                        info.width,
                        info.height,
                        info.fps_num,
                        info.fps_den,
                        info.total_frames,
                    );
                }

                // both eyes of a 3D encode use these params
                let target_codestream_bytes = video_bit_rate.map(|mbps| {
                    dcpwizard_core::encode::video_codestream_byte_cap(
                        fps,
                        mbps,
                        right_eye.is_some(),
                    )
                });

                let dci_codestream_byte_cap = if hdr_dci {
                    dcpwizard_core::hdr::hdr_codestream_byte_cap(fps)
                } else {
                    postkit::j2k::dci_codestream_byte_cap(fps)
                };
                // under a PSNR target the bandwidth is a ceiling per frame
                let codestream_byte_cap = match (quality_psnr, target_codestream_bytes) {
                    (Some(_), Some(target)) => dci_codestream_byte_cap.min(target),
                    _ => dci_codestream_byte_cap,
                };
                match (quality_psnr, video_bit_rate, target_codestream_bytes) {
                    (Some(db), Some(mbps), _) => tracing::info!(
                        "PSNR {db} dB (bandwidth {mbps} Mbit/s, at most {codestream_byte_cap} bytes a frame)"
                    ),
                    (Some(db), None, _) => {
                        tracing::info!("PSNR {db} dB (at most {codestream_byte_cap} bytes a frame)")
                    }
                    (None, Some(mbps), Some(target)) => tracing::info!(
                        "Target: {target} bytes a frame ({mbps} Mbit/s), cap {codestream_byte_cap} bytes"
                    ),
                    (None, _, _) => {}
                }

                let _num_threads = threads.unwrap_or(0); // reserved for future use

                let params = CompressParams {
                    compression_ratio: dcpwizard_core::encode::DEFAULT_COMPRESSION_RATIO,
                    target_codestream_bytes,
                    quality_psnr,
                    codestream_byte_cap: Some(codestream_byte_cap),
                    edit_rate: postkit::encode::FrameRate::whole(fps),
                    // grok converts only what nothing else has converted
                    apply_xyz_transform: !content_already_xyz && frame_transform.is_none(),
                    source_preparation: postkit::grok_encoder::SourcePreparation {
                        subtitle_burn: build_subtitle_burn(postkit::encode::FrameRate::whole(fps)),
                        watermark: build_watermark(postkit::encode::FrameRate::whole(fps)),
                        colour_transform: frame_transform,
                    },
                    ..CompressParams::default()
                };

                let cancel = Arc::new(AtomicBool::new(false));
                let cancel_clone = cancel.clone();
                let _ = ctrlc::set_handler(move || {
                    cancel_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                });

                // only the kept frames are compressed, so a trim never pays for
                // the encoder time it then throws away
                let encode_window = trim.encode_window(&encode_video_path);
                let encode_frames = match encode_window {
                    Some(window) => window.frame_count,
                    None => total_frames as u64,
                };
                if let Some(window) = encode_window {
                    tracing::info!(
                        "Encoding frames {}..{} of the source",
                        window.first_frame,
                        window.end_frame()
                    );
                }

                // persist encode identity so an interrupted run can --resume the
                // J2K frames on disk (dom#344). --resume verifies the params match
                // before reusing them. The count is the window's, so changing a
                // trim refuses the resume rather than reusing another window's
                // codestreams.
                let encode_state = dcpwizard_core::encode_qol::EncodeState {
                    source: video_path.to_string_lossy().to_string(),
                    total_frames: encode_frames,
                    fps,
                    width,
                    height,
                    bitrate_mbps: video_bit_rate.unwrap_or(0),
                };
                if resume && let Err(e) = encode_state.check_resumable(&output_dir) {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
                if let Err(e) = encode_state.save(&output_dir) {
                    tracing::warn!("could not save resume state: {e}");
                }

                // the window is cut after this chain, so a fade is placed in
                // source time and the fade-out belongs at the end of the window
                let faded_frames = match encode_window {
                    Some(window) => window.end_frame(),
                    None => total_frames as u64,
                };
                let fade_filter = match dcpwizard_core::audio_adjust::video_fade_filter(
                    video_fade_in,
                    video_fade_out,
                    faded_frames as f64 / fps.max(1) as f64,
                ) {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
                let picture_filter = join_decode_filters(&resolved_picture.plan.filters, None);
                let video_filter =
                    join_decode_filters(&resolved_picture.plan.filters, fade_filter.as_deref());
                // the picture MXF is written as the frames finish where the run
                // allows it, so packaging never reads the J2K directory back
                let overlap_refusal = dcpwizard_core::overlapped_picture::overlap_refusal(
                    &dcpwizard_core::overlapped_picture::PictureSource {
                        input_type: postkit::encode::InputType::Video,
                        still_hold: false,
                    },
                    &dcpwizard_core::overlapped_picture::PackageShape {
                        stereoscopic: right_eye.is_some(),
                        pads: pad_head.is_some() || pad_tail.is_some(),
                        splits_reels: reel_length.unwrap_or(0) > 0
                            || split_at.is_some()
                            || split_chapters,
                        multiple_versions: versions_specs.is_some(),
                        encrypts: encrypt,
                    },
                )
                .or(resume.then_some(
                    "a resumed encode reuses frames already on disk, which never pass through \
                     the wrap",
                ))
                .or((trim.is_active() && encode_window.is_none())
                    .then_some("the trim relinks the codestreams after the encode"));
                let picture_wrap = match overlap_refusal {
                    Some(reason) => {
                        tracing::info!("Wrapping the picture MXF after the encode: {reason}");
                        None
                    }
                    None => match dcpwizard_core::overlapped_picture::PictureWrapInProgress::start(
                        dcpwizard_core::overlapped_picture::PictureWrapTarget {
                            dcp_dir: output_dir.clone(),
                            fps,
                            hdr_dci,
                        },
                    ) {
                        Ok(wrap) => Some(wrap),
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    },
                };
                let encode_start = std::time::Instant::now();
                let mut last_encode_progress: Option<EncodeProgress> = None;
                let result = grok_encoder::encode_video_pipeline_resumable_with_mxf_feed(
                    &encode_video_path,
                    &j2k_dir,
                    &params,
                    encode_frames,
                    width,
                    height,
                    &source_pixel_format,
                    hdr_dcdm_colour
                        .as_ref()
                        .unwrap_or(&postkit::encode::SourceColour::DisplayRgb),
                    &cancel,
                    resume,
                    video_filter.as_deref(),
                    encode_window,
                    picture_wrap.as_ref().map(|wrap| wrap.sender()),
                    |p: EncodeProgress| {
                        let percent = if p.total_frames > 0 {
                            (p.frames_encoded as f64 / p.total_frames as f64) * 100.0
                        } else {
                            0.0
                        };
                        // ETA from average fps since the encode started (dom#502):
                        // steadier than the instantaneous rate.
                        let elapsed = encode_start.elapsed().as_secs_f64();
                        let avg_fps = if elapsed > 0.0 {
                            p.frames_encoded as f64 / elapsed
                        } else {
                            0.0
                        };
                        let eta = dcpwizard_core::encode_qol::eta_seconds(
                            p.frames_encoded,
                            p.total_frames,
                            avg_fps,
                        )
                        .map(dcpwizard_core::encode_qol::format_eta)
                        .unwrap_or_else(|| "--:--".to_string());
                        eprint!(
                            "\r[encode] {}/{} frames ({:.0}%) {:.1} fps  avg {:.1}  eta {}   ",
                            p.frames_encoded, p.total_frames, percent, p.fps, avg_fps, eta
                        );
                        last_encode_progress = Some(p);
                    },
                );
                eprintln!();

                if !result.success {
                    let wrap_error = picture_wrap.and_then(|wrap| wrap.abandon());
                    tracing::error!("Encode failed: {}", wrap_error.unwrap_or(result.error));
                    std::process::exit(1);
                }
                tracing::info!("Encoded {} frames", result.frames_encoded);
                let picture_mxf = match picture_wrap {
                    Some(wrap) => match wrap.finish(result.frames_encoded) {
                        Ok(wrapped) => {
                            tracing::info!(
                                "Picture MXF written during the encode: {} ({} frames)",
                                wrapped.mxf_name(),
                                wrapped.duration
                            );
                            Some(wrapped)
                        }
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    },
                    None => None,
                };
                // the wrap's hash ran while the hints finished, so this waits less
                print_hints(hints_pass);
                // a window restamps its kept frames from zero ahead of the
                // detection branch, so a trim needs no offset here
                for finding in result.picture_findings.describe(fps as f64) {
                    tracing::warn!("{finding}");
                }
                if let Some(progress) = last_encode_progress.as_ref() {
                    tracing::info!("Encode breakdown: {}", encode_phase_breakdown(progress));
                }

                // Stereoscopic: encode the right eye into its own dir at the same
                // settings (main input is the left eye).
                let right_eye_dir = if let Some(ref re) = right_eye {
                    let re_path = PathBuf::from(re);
                    let j2k_right = output_dir.join("j2k_right");
                    let _ = std::fs::create_dir_all(&j2k_right);
                    tracing::info!("Encoding right eye: {}", re_path.display());
                    // both eyes are one picture track, so the right eye is
                    // cropped, turned, fitted and windowed exactly as the left
                    // one was
                    let re_result = grok_encoder::encode_video_pipeline_resumable(
                        &re_path,
                        &j2k_right,
                        &params,
                        encode_frames,
                        width,
                        height,
                        &postkit::probe::probe_pixel_format(&re_path),
                        &cancel,
                        false,
                        picture_filter.as_deref(),
                        encode_window,
                        |_p: EncodeProgress| {},
                    );
                    if !re_result.success {
                        tracing::error!("Right-eye encode failed: {}", re_result.error);
                        std::process::exit(1);
                    }
                    for finding in re_result.picture_findings.describe(fps as f64) {
                        tracing::warn!("right eye: {finding}");
                    }
                    Some(j2k_right)
                } else {
                    None
                };

                // Auto-demux audio from video if --audio not provided
                let raw_audio = if let Some(a) = audio {
                    Some(PathBuf::from(a))
                } else {
                    let wav_out = output_dir.join("audio_demux.wav");
                    let demux = std::process::Command::new("ffmpeg")
                        .arg("-y")
                        .arg("-i")
                        .arg(&video_path)
                        .arg("-vn")
                        .arg("-acodec")
                        .arg("pcm_s24le")
                        .arg("-ar")
                        .arg("48000")
                        .arg(&wav_out)
                        .output();
                    match demux {
                        Ok(o) if o.status.success() => {
                            tracing::info!("Demuxed audio: {}", wav_out.display());
                            Some(wav_out)
                        }
                        Ok(_) => {
                            tracing::warn!("No audio stream found in input (or demux failed)");
                            None
                        }
                        Err(e) => {
                            tracing::warn!("ffmpeg not available for audio demux: {e}");
                            None
                        }
                    }
                };
                // W5 audio processing: filename channel routing (a --audio
                // directory), stereo->5.1 upmix, then loudness normalization.
                let audio_path = match prepare_create_audio(
                    raw_audio,
                    audio_map.as_deref(),
                    upmix.as_deref(),
                    audio_delay,
                    loudness_target.as_deref(),
                    true_peak_ceiling,
                    &audio_adjust,
                    &output_dir.join("audio_work"),
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };

                let audio_path = match (conform.audio_pull_up, audio_path) {
                    (true, Some(input)) => {
                        let output = output_dir.join("audio_pullup.wav");
                        if let Err(error) = dcpwizard_core::hfr::audio_pull_up(&input, &output) {
                            tracing::error!("{error}");
                            std::process::exit(1);
                        }
                        tracing::info!("Applied 23.976-to-24 audio pull-up");
                        Some(output)
                    }
                    (_, audio_path) => audio_path,
                };

                // the picture already holds only the kept frames, so this cuts
                // the sound: after the pull-up, whose resample changes what a
                // frame of audio is worth, and before sign language, which is
                // packed to cover the picture the package actually carries
                let (packaged_j2k_dir, audio_path) = match trim.apply(
                    &j2k_dir,
                    &output_dir.join("j2k_trimmed"),
                    audio_path,
                    fps,
                    encode_window,
                ) {
                    Ok(pair) => pair,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
                let packaged_right_eye_dir = match right_eye_dir.as_ref() {
                    Some(dir) => match trim.apply(
                        dir,
                        &output_dir.join("j2k_right_trimmed"),
                        None,
                        fps,
                        encode_window,
                    ) {
                        Ok((trimmed, _)) => Some(trimmed),
                        Err(e) => {
                            tracing::error!("right eye: {e}");
                            std::process::exit(1);
                        }
                    },
                    None => None,
                };
                let picture_frames = if trim.is_active() {
                    trim.kept_frames
                } else {
                    total_frames as u64
                };

                // sign-language video (ISDCF Doc 13): pack VP9 onto channel 15,
                // overriding the sound track with the combined 16-channel WAV
                let (audio_path, sl_main_channels) = if let Some(slv) = sign_language_video.as_ref()
                {
                    match build_sign_language_audio(
                        slv,
                        audio_path.as_deref(),
                        picture_frames,
                        fps,
                        &output_dir,
                    ) {
                        Ok((wav, ch)) => (Some(wav), Some(ch)),
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    }
                } else {
                    (audio_path, None)
                };

                let resolution = package_resolution;
                let ct = content_type
                    .as_deref()
                    .and_then(dcpwizard_core::ContentType::from_abbrev)
                    .unwrap_or_default();

                // reel-split boundaries from --split-at / --split-chapters
                let reel_split_frames = match resolve_reel_splits(
                    split_at.as_deref(),
                    split_chapters,
                    Some(&video_path),
                    fps,
                ) {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };

                let mut config = dcpwizard_core::dcp::DcpConfig {
                    title,
                    standard: std_val,
                    encrypt,
                    key_out: key_out.map(PathBuf::from),
                    output_dir: output_dir.clone(),
                    frame_rate_num: fps,
                    frame_rate_den: 1,
                    resolution,
                    content_type: ct,
                    container_width,
                    container_height,
                    max_bitrate_mbps: video_bit_rate.unwrap_or(0),
                    j2k_dir: Some(packaged_j2k_dir.clone()),
                    picture_mxf,
                    audio_path: audio_path.clone(),
                    audio_input_order,
                    audio_channels,
                    subtitle_path: subtitle.clone().map(PathBuf::from),
                    subtitle_language: subtitle_language.clone(),
                    subtitle_opts: subtitle_opts.clone(),
                    ccap_path: ccap.clone().map(PathBuf::from),
                    ccap_language: ccap_language.clone(),
                    reel_length_minutes: reel_length.unwrap_or(0),
                    right_eye_dir: packaged_right_eye_dir.clone(),
                    atmos_path: atmos.clone().map(PathBuf::from),
                    hi_channel,
                    vi_channel,
                    stereo_3d: packaged_right_eye_dir.is_some(),
                    pad_head: pad_head.clone(),
                    pad_tail: pad_tail.clone(),
                    pad_color: pad_color.clone(),
                    source_trim: trim.source_trim(),
                    reel_split_frames,
                    sign_language_lang: sign_language_lang.clone(),
                    release_territory: release_territory.clone(),
                    version_number,
                    chain: chain.clone(),
                    distributor: distributor.clone(),
                    facility: facility.clone(),
                    luminance: parsed_luminance.clone(),
                    sign_language_main_channels: sl_main_channels,
                    hdr_dci,
                    signer: package_signer.clone(),
                    markers: markers.clone(),
                    audio_language: naming.audio_language.clone(),
                    ratings: naming.ratings.clone(),
                    content_versions: naming.content_versions.clone(),
                    head_items: head_items.clone(),
                    tail_items: tail_items.clone(),
                };
                apply_isdcf_name(&mut config, &naming, burnt_in_subtitle);
                let code = match versions_specs.as_ref() {
                    Some(v) => dcpwizard_core::versions::create_versioned_dcp(&config, v),
                    None => dcpwizard_core::dcp::create_dcp(&config),
                };

                remove_intermediates_if_packaged(&output_dir, &video_path, code)
            } else {
                // Input is a J2K directory or image sequence
                print_hints(hints_pass);
                let resolution = package_resolution;
                let ct = content_type
                    .as_deref()
                    .and_then(dcpwizard_core::ContentType::from_abbrev)
                    .unwrap_or_default();

                if input_range.is_some() {
                    tracing::error!(
                        "--input-range applies to a video input; a J2K/image sequence carries no decode range"
                    );
                    std::process::exit(1);
                }

                let fps = frame_rate.unwrap_or(24);
                let reel_split_frames =
                    match resolve_reel_splits(split_at.as_deref(), split_chapters, None, fps) {
                        Ok(f) => f,
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    };

                // a still becomes a codestream directory here: one encode, then
                // the codestream linked for every frame of the hold
                let still_j2k_dir = output_dir.join(postkit::still::HELD_PICTURE_DIR);
                let source_j2k_dir = if still_input {
                    let spec = still_length.as_deref().unwrap_or_default();
                    let frames = match dcpwizard_core::pad::parse_pad_frames(spec, fps) {
                        Ok(0) => {
                            tracing::error!(
                                "--still-length: a still must be held for at least one frame"
                            );
                            std::process::exit(1);
                        }
                        Ok(n) => n,
                        Err(e) => {
                            tracing::error!("--still-length: {e}");
                            std::process::exit(1);
                        }
                    };
                    let (width, height, still_filters) = match still_picture(
                        &video_path,
                        &picture_options,
                        &encode_geometry(twok, fourk, (container_width, container_height)),
                    ) {
                        Ok(picture) => picture,
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    };
                    let _ = std::fs::create_dir_all(&output_dir);
                    let colour_transform = match xyz_route.frame_transform() {
                        Ok(transform) => transform,
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    };
                    if let Err(e) = postkit::still::build_still_frames(&postkit::still::StillHold {
                        image: &video_path,
                        frames,
                        fps: postkit::encode::FrameRate::whole(fps),
                        width,
                        height,
                        filters: &still_filters,
                        apply_xyz_transform: xyz_route.compressor_transform(),
                        rsiz: postkit::encode::default_rsiz(),
                        colour_transform,
                        burn: build_subtitle_burn(postkit::encode::FrameRate::whole(fps)),
                        watermark: build_watermark(postkit::encode::FrameRate::whole(fps)),
                        out_dir: &still_j2k_dir,
                    }) {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                    tracing::info!("Held the still for {frames} frame(s) at {width}x{height}");
                    still_j2k_dir.clone()
                } else {
                    video_path.clone()
                };

                let source_frames = dcpwizard_core::trim::frame_count(&source_j2k_dir);
                let trim = match resolve_trim(
                    trim_start.as_deref(),
                    trim_end.as_deref(),
                    source_frames,
                    fps,
                ) {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };

                // W5 audio processing: filename channel routing (a --audio
                // directory), stereo->5.1 upmix, then loudness normalization.
                let work_dir = output_dir.join("audio_work");
                let prepared_audio = match prepare_create_audio(
                    audio.map(PathBuf::from),
                    audio_map.as_deref(),
                    upmix.as_deref(),
                    audio_delay,
                    loudness_target.as_deref(),
                    true_peak_ceiling,
                    &audio_adjust,
                    &work_dir,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };

                // nothing was encoded here, so the kept codestreams are linked
                // out of the source directory
                let (packaged_j2k_dir, prepared_audio) = match trim.apply(
                    &source_j2k_dir,
                    &output_dir.join("j2k_trimmed"),
                    prepared_audio,
                    fps,
                    None,
                ) {
                    Ok(pair) => pair,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };

                // sign-language video (ISDCF Doc 13): pack VP9 onto channel 15.
                // Cover at least the J2K frame count so the sound spans the picture.
                let (audio_path, sl_main_channels) = if let Some(slv) = sign_language_video.as_ref()
                {
                    let frames = dcpwizard_core::trim::frame_count(&packaged_j2k_dir);
                    match build_sign_language_audio(
                        slv,
                        prepared_audio.as_deref(),
                        frames,
                        fps,
                        &output_dir,
                    ) {
                        Ok((wav, ch)) => (Some(wav), Some(ch)),
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    }
                } else {
                    (prepared_audio, None)
                };

                let mut config = dcpwizard_core::dcp::DcpConfig {
                    title,
                    standard: std_val,
                    encrypt,
                    key_out: key_out.map(PathBuf::from),
                    output_dir: output_dir.clone(),
                    frame_rate_num: fps,
                    frame_rate_den: 1,
                    resolution,
                    content_type: ct,
                    container_width,
                    container_height,
                    max_bitrate_mbps: video_bit_rate.unwrap_or(0),
                    j2k_dir: Some(packaged_j2k_dir),
                    // create encodes through the resumable encoder, which has no wrap feed
                    picture_mxf: None,
                    audio_path,
                    audio_input_order,
                    audio_channels,
                    subtitle_path: subtitle.map(PathBuf::from),
                    subtitle_language,
                    subtitle_opts,
                    ccap_path: ccap.map(PathBuf::from),
                    ccap_language,
                    reel_length_minutes: reel_length.unwrap_or(0),
                    stereo_3d: right_eye.is_some(),
                    right_eye_dir: right_eye.map(PathBuf::from),
                    atmos_path: atmos.map(PathBuf::from),
                    hi_channel,
                    vi_channel,
                    pad_head,
                    pad_tail,
                    pad_color,
                    source_trim: trim.source_trim(),
                    reel_split_frames,
                    sign_language_lang,
                    sign_language_main_channels: sl_main_channels,
                    hdr_dci,
                    signer: package_signer,
                    markers,
                    release_territory,
                    version_number,
                    chain,
                    distributor,
                    facility,
                    luminance: parsed_luminance,
                    audio_language: naming.audio_language.clone(),
                    ratings: naming.ratings.clone(),
                    content_versions: naming.content_versions.clone(),
                    head_items,
                    tail_items,
                };
                apply_isdcf_name(&mut config, &naming, burnt_in_subtitle);
                let code = match versions_specs.as_ref() {
                    Some(v) => dcpwizard_core::versions::create_versioned_dcp(&config, v),
                    None => dcpwizard_core::dcp::create_dcp(&config),
                };
                remove_intermediates_if_packaged(&output_dir, &video_path, code)
            };

            // upload to the TMS (dom's upload_after_make_dcp): opt-in, only
            // after a clean run, and before any power-off.
            let code = match tms_target {
                Some(config) if code == 0 => {
                    match dcpwizard_core::tms_upload::upload_package(&config, &output_dir) {
                        Ok(()) => 0,
                        Err(e) => {
                            tracing::error!("{e}");
                            1
                        }
                    }
                }
                _ => code,
            };

            // shutdown on completion (dom#1394): opt-in, only after a clean run.
            // resolve_shutdown_command already failed loud up front if missing.
            if shutdown_when_done && code == 0 {
                tracing::info!("Encode complete; powering off (--shutdown-when-done)");
                if let Err(e) = dcpwizard_core::encode_qol::run_shutdown() {
                    tracing::error!("{e}");
                    1
                } else {
                    code
                }
            } else {
                code
            }
        }

        Commands::Encode {
            input,
            output,
            bandwidth,
            fps,
        } => {
            use std::sync::Arc;
            use std::sync::atomic::AtomicBool;

            let cancel = Arc::new(AtomicBool::new(false));
            let cancel_clone = cancel.clone();
            let _ = ctrlc::set_handler(move || {
                cancel_clone.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            let encode = dcpwizard_core::encode::ImageSequenceEncode {
                input_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                bandwidth_mbps: bandwidth,
                fps,
            };
            let result = dcpwizard_core::encode::encode_image_sequence(
                &encode,
                &cancel,
                print_encode_progress,
            );
            eprintln!();
            match result {
                Ok(result) => {
                    tracing::info!(
                        "Encoded {} frames to {}",
                        result.frames_encoded,
                        result.j2k_dir.display()
                    );
                    0
                }
                Err(e) => {
                    tracing::error!("{e}");
                    1
                }
            }
        }

        Commands::Pipeline {
            input,
            title,
            output,
            audio,
            ratio,
            fps,
            input_range,
            split_chapters,
        } => {
            use postkit::pipeline::{EncodeRunOptions, run_encode_with_options};
            use std::sync::Arc;
            use std::sync::atomic::AtomicBool;

            let input_path = PathBuf::from(&input);
            let output_dir = PathBuf::from(&output);

            if !input_path.exists() {
                tracing::error!("Input not found: {input}");
                std::process::exit(1);
            }

            let _ = std::fs::create_dir_all(&output_dir);
            let j2k_dir = output_dir.join("j2k");
            let _ = std::fs::create_dir_all(&j2k_dir);

            tracing::info!("Pipeline: {} -> {}", input, output);

            // reel-split boundaries from the source's chapter marks
            let reel_split_frames = if split_chapters {
                match video_chapter_boundaries(&input_path, fps) {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                }
            } else {
                Vec::new()
            };

            // optional decode-range correction into a lossless intermediate
            let encode_input = if let Some(range) = input_range.as_deref() {
                match normalize_input_range(&input_path, range, &output_dir) {
                    Ok(p) => {
                        tracing::info!("Forcing {range}-range decode of the source");
                        p
                    }
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                }
            } else {
                input_path.clone()
            };

            let conform = dcpwizard_core::probe::probe_video(&encode_input)
                .map(|info| {
                    dcpwizard_core::hfr::conform_source_to_dcp(info.fps_num, info.fps_den, fps)
                })
                .unwrap_or_default();

            let encode_options = EncodeRunOptions {
                compression_ratio: ratio,
                fps: postkit::encode::FrameRate::whole(fps),
                read_source_at: conform.read_source_at,
                codestream_byte_cap: Some(postkit::j2k::dci_codestream_byte_cap(fps)),
                ..EncodeRunOptions::default()
            };

            let cancel = Arc::new(AtomicBool::new(false));
            let pause = Arc::new(AtomicBool::new(false));

            // Handle Ctrl+C
            let cancel_clone = cancel.clone();
            let _ = ctrlc::set_handler(move || {
                cancel_clone.store(true, std::sync::atomic::Ordering::Relaxed);
            });

            let encode = run_encode_with_options(
                &encode_input,
                &output_dir,
                &encode_options,
                &cancel,
                &pause,
                print_encode_progress,
                |message: &str| tracing::debug!("{message}"),
            );
            eprintln!();

            let encode = match encode {
                Ok(encode) => encode,
                Err(e) => {
                    tracing::error!("Encode failed: {e}");
                    std::process::exit(1);
                }
            };
            tracing::info!("Encoded {} frames", encode.frames_encoded);
            for finding in encode.picture_findings.describe(fps as f64) {
                tracing::warn!("{finding}");
            }

            // Auto-demux audio from video if --audio not provided
            let audio_path = if let Some(a) = audio {
                Some(PathBuf::from(a))
            } else {
                let wav_out = output_dir.join("audio_demux.wav");
                let demux = std::process::Command::new("ffmpeg")
                    .arg("-y")
                    .arg("-i")
                    .arg(&input_path)
                    .arg("-vn")
                    .arg("-acodec")
                    .arg("pcm_s24le")
                    .arg("-ar")
                    .arg("48000")
                    .arg(&wav_out)
                    .output();
                match demux {
                    Ok(o) if o.status.success() => {
                        tracing::info!("Demuxed audio: {}", wav_out.display());
                        Some(wav_out)
                    }
                    Ok(_) => {
                        tracing::warn!("No audio stream in input (or demux failed)");
                        None
                    }
                    Err(e) => {
                        tracing::warn!("ffmpeg not available for audio demux: {e}");
                        None
                    }
                }
            };

            let audio_path = match (conform.audio_pull_up, audio_path) {
                (true, Some(input)) => {
                    let pulled_up = output_dir.join("audio_pullup.wav");
                    if let Err(error) = dcpwizard_core::hfr::audio_pull_up(&input, &pulled_up) {
                        tracing::error!("{error}");
                        std::process::exit(1);
                    }
                    tracing::info!("Applied 23.976-to-24 audio pull-up");
                    Some(pulled_up)
                }
                (_, audio_path) => audio_path,
            };

            // Package
            let config = dcpwizard_core::dcp::DcpConfig {
                title,
                standard: dcpwizard_core::Standard::Smpte,
                output_dir: output_dir.clone(),
                frame_rate_num: fps,
                frame_rate_den: 1,
                j2k_dir: Some(j2k_dir.clone()),
                audio_path: audio_path.clone(),
                audio_input_order: dcpwizard_core::mxf_wrap::AudioInputOrder::Canonical51,
                reel_split_frames: reel_split_frames.clone(),
                ..Default::default()
            };
            let code = dcpwizard_core::dcp::create_dcp(&config);

            remove_intermediates_if_packaged(&output_dir, &input_path, code)
        }

        Commands::Transcode {
            input,
            output,
            format,
            bit_depth,
        } => {
            // 8/10/16-bit packed RGB pixel formats ffmpeg understands for these codecs
            let pixel_format = match bit_depth {
                8 => "rgb24",
                10 | 16 => "rgb48le",
                other => {
                    tracing::error!("unsupported bit depth {other}; use 8, 10 or 16");
                    std::process::exit(1);
                }
            };
            let config = dcpwizard_core::transcode::TranscodeConfig {
                input_file: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                image_format: format,
                pixel_format: pixel_format.to_string(),
                ..Default::default()
            };
            dcpwizard_core::transcode::transcode_to_sequence(&config)
        }

        Commands::TranscodeDcp {
            input,
            output,
            video_bit_rate,
            width,
            height,
            kdm,
            recipient_key,
            keys,
        } => {
            let config = dcpwizard_core::j2k_transcode::DcpTranscodeConfig {
                input_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                target_bitrate_mbps: Some(video_bit_rate),
                target_width: width.unwrap_or(0),
                target_height: height.unwrap_or(0),
                kdm: kdm.map(PathBuf::from),
                recipient_key: recipient_key.map(PathBuf::from),
                keys: keys.map(PathBuf::from),
                watermark: None,
            };
            dcpwizard_core::j2k_transcode::transcode_dcp(&config)
        }

        Commands::Decrypt {
            input,
            output,
            kdm,
            recipient_key,
            keys,
        } => {
            let config = dcpwizard_core::decrypt::DcpDecryptConfig {
                input_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                kdm: kdm.map(PathBuf::from),
                recipient_key: recipient_key.map(PathBuf::from),
                keys: keys.map(PathBuf::from),
            };
            dcpwizard_core::decrypt::decrypt_dcp(&config)
        }

        Commands::Verify {
            dcp_dir,
            no_hash_check,
            no_picture_check,
            strict,
            output,
            quiet,
        } => {
            let result = dcpwizard_core::verify::verify_dcp_with_options(
                &PathBuf::from(&dcp_dir),
                &dcpwizard_core::verify::VerifyCliOptions {
                    skip_hash_check: no_hash_check,
                    skip_picture_check: no_picture_check,
                    strict,
                    scan_every_frame: false,
                },
            );

            if let Some(ref out_path) = output
                && let Err(e) =
                    dcpwizard_core::verify::write_verify_report(&result, Path::new(out_path))
            {
                tracing::error!("Failed to write report: {e}");
                std::process::exit(1);
            }

            if !quiet {
                if result.valid {
                    tracing::info!("DCP verification PASSED");
                } else {
                    for e in &result.errors {
                        tracing::error!("{e}");
                    }
                }
                for w in &result.warnings {
                    tracing::warn!("{w}");
                }
                for i in &result.info {
                    tracing::info!("{i}");
                }
            }

            if result.valid { 0 } else { 1 }
        }

        Commands::Info { dcp_dir } => {
            match dcpwizard_core::info::inspect_dcp(&PathBuf::from(dcp_dir)) {
                Ok(info) => {
                    tracing::info!("Title: {}", info.title);
                    tracing::info!("Standard: {}", info.standard);
                    tracing::info!("Frame rate: {}", info.frame_rate);
                    tracing::info!("Duration: {} frames", info.duration_frames);
                    tracing::info!("Reels: {}", info.reel_count);
                    tracing::info!("Encrypted: {}", if info.encrypted { "yes" } else { "no" });
                    0
                }
                Err(e) => {
                    tracing::error!("{e}");
                    1
                }
            }
        }

        Commands::Kdm {
            cpl_id,
            content_title,
            cert,
            signer_cert,
            signer_key,
            signer_chain,
            output,
            valid_from,
            valid_to,
            template,
            templates_file,
            history_file,
            email_to,
            smtp_config,
            keys,
            format,
            annotation,
            device_cert,
            kdm_options,
        } => {
            let format = match dcpwizard_core::kdm::parse_format(&format) {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            let (valid_from, valid_to) =
                match resolve_window(valid_from, valid_to, template, templates_file) {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
            let content_keys = match keys {
                Some(path) => {
                    match dcpwizard_core::kdm::load_content_keys(&PathBuf::from(path), &cpl_id) {
                        Ok(k) => k,
                        Err(e) => {
                            tracing::error!("{e}");
                            std::process::exit(1);
                        }
                    }
                }
                None => Vec::new(),
            };
            let out_path = PathBuf::from(&output);
            let title = content_title.clone();
            let code = dcpwizard_core::kdm::generate_kdm(
                cpl_id,
                content_title,
                PathBuf::from(cert),
                PathBuf::from(signer_cert),
                PathBuf::from(signer_key),
                signer_chain.into_iter().map(PathBuf::from).collect(),
                valid_from,
                valid_to,
                content_keys,
                out_path.clone(),
                format,
                annotation,
                Some(history_path(history_file)),
                device_cert.into_iter().map(PathBuf::from).collect(),
                kdm_options.into(),
            );
            if code == 0 {
                if let Some(cfg_path) = smtp_config {
                    match send_kdm_email(&cfg_path, "", &title, &email_to, &[out_path]) {
                        Ok(()) => 0,
                        Err(e) => {
                            tracing::error!("{e}");
                            1
                        }
                    }
                } else {
                    0
                }
            } else {
                code
            }
        }

        Commands::KdmRewrap {
            dkdm,
            dkdm_key,
            cert,
            signer_cert,
            signer_key,
            signer_chain,
            valid_from,
            valid_to,
            output,
            device_cert,
            kdm_options,
        } => dcpwizard_core::kdm::rewrap_dkdm(
            PathBuf::from(dkdm),
            PathBuf::from(dkdm_key),
            PathBuf::from(cert),
            PathBuf::from(signer_cert),
            PathBuf::from(signer_key),
            signer_chain.into_iter().map(PathBuf::from).collect(),
            valid_from,
            valid_to,
            PathBuf::from(output),
            device_cert.into_iter().map(PathBuf::from).collect(),
            kdm_options.into(),
        ),

        Commands::Copy { src, dst } => {
            dcpwizard_core::copy_drive::copy_to_drive(&PathBuf::from(src), &PathBuf::from(dst))
        }

        Commands::FormatDrive {
            target,
            fs,
            label,
            yes,
            image,
        } => {
            let fs = match dcpwizard_core::disk::ExtFs::parse(&fs) {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            match dcpwizard_core::disk::format_drive(
                &PathBuf::from(&target),
                fs,
                label.as_deref(),
                yes,
                image,
            ) {
                Ok(()) => {
                    tracing::info!("Formatted {target} as {fs:?}");
                    0
                }
                Err(e) => {
                    tracing::error!("{e}");
                    1
                }
            }
        }

        Commands::CheckDrive { target } => {
            match dcpwizard_core::disk::check_drive(&PathBuf::from(&target)) {
                Ok(info) => {
                    tracing::info!(
                        "{target}: fs={} label={}",
                        info.fstype.as_deref().unwrap_or("unknown"),
                        info.label.as_deref().unwrap_or("(none)")
                    );
                    0
                }
                Err(e) => {
                    tracing::error!("{e}");
                    1
                }
            }
        }

        Commands::Loudness { audio_file } => {
            let result = dcpwizard_core::loudness::measure_loudness(&PathBuf::from(audio_file));
            if result.success {
                tracing::info!("Integrated: {:.1} LUFS", result.integrated_lufs);
                tracing::info!("True Peak: {:.1} dBTP", result.true_peak_dbtp);
                tracing::info!("LRA: {:.1} LU", result.range_lu);
                0
            } else {
                tracing::error!("{}", result.error);
                1
            }
        }

        Commands::Crossfade {
            a,
            b,
            output,
            overlap,
        } => match postkit::crossfade::crossfade_join_wav(
            &PathBuf::from(a),
            &PathBuf::from(b),
            &PathBuf::from(&output),
            overlap,
        ) {
            Ok(()) => {
                tracing::info!("Wrote crossfade join: {output}");
                0
            }
            Err(e) => {
                tracing::error!("crossfade failed: {e}");
                1
            }
        },

        Commands::MidSideDecode {
            input,
            output,
            mid,
            side,
        } => match postkit::mid_side::decode_mid_side_wav(
            &PathBuf::from(input),
            &PathBuf::from(&output),
            mid,
            side,
        ) {
            Ok(()) => {
                tracing::info!("Wrote mid-side decoded WAV: {output}");
                0
            }
            Err(e) => {
                tracing::error!("mid-side decode failed: {e}");
                1
            }
        },

        Commands::Report {
            dcp,
            output,
            scan_picture,
        } => dcpwizard_core::report::generate_report(
            &PathBuf::from(dcp),
            &PathBuf::from(output),
            scan_picture,
        ),

        Commands::Serve { bind, api_key } => {
            dcpwizard_core::rest_api::start_rest_api(&bind, api_key.as_deref())
        }

        Commands::Watch {
            dir,
            output,
            webhook_url,
            interval,
            create_arguments,
        } => {
            use dcpwizard_core::watch::{
                AUDIO_SIDECAR_EXTENSION, DONE_DIRECTORY_NAME, FAILED_DIRECTORY_NAME,
                SUBTITLE_SIDECAR_EXTENSION,
            };

            fn free_destination(directory: &Path, file_name: &str) -> PathBuf {
                let taken = directory.join(file_name);
                if !taken.exists() {
                    return taken;
                }
                let stem = Path::new(file_name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(file_name);
                let extension = Path::new(file_name).extension().and_then(|e| e.to_str());
                let mut suffix = 1u32;
                loop {
                    let candidate = match extension {
                        Some(extension) => directory.join(format!("{stem}-{suffix}.{extension}")),
                        None => directory.join(format!("{stem}-{suffix}")),
                    };
                    if !candidate.exists() {
                        return candidate;
                    }
                    suffix += 1;
                }
            }

            fn move_aside(source: &Path, directory: &Path) {
                if let Err(e) = std::fs::create_dir_all(directory) {
                    tracing::error!("cannot create {}: {e}", directory.display());
                    return;
                }
                let Some(file_name) = source.file_name().and_then(|n| n.to_str()) else {
                    tracing::error!("cannot read a file name from {}", source.display());
                    return;
                };
                let destination = free_destination(directory, file_name);
                if let Err(e) = std::fs::rename(source, &destination) {
                    tracing::error!(
                        "cannot move {} into {}: {e}",
                        source.display(),
                        directory.display()
                    );
                }
            }

            fn post_event(
                webhook: Option<&postkit::webhook::WebhookConfig>,
                event_type: &str,
                job_id: &str,
                payload_json: String,
            ) {
                let Some(config) = webhook else {
                    return;
                };
                let event = postkit::webhook::WebhookEvent {
                    event_type: event_type.to_string(),
                    job_id: job_id.to_string(),
                    payload_json,
                    timestamp: String::new(),
                };
                let result = postkit::webhook::send_webhook(config, &event);
                if !result.success {
                    tracing::warn!("webhook delivery failed: {}", result.error);
                }
            }

            let watch_dir = PathBuf::from(&dir);
            let output_dir = PathBuf::from(&output);
            if let Err(e) = std::fs::create_dir_all(&output_dir) {
                tracing::error!("cannot create {}: {e}", output_dir.display());
                std::process::exit(1);
            }
            let executable = match std::env::current_exe() {
                Ok(path) => path,
                Err(e) => {
                    tracing::error!("cannot find the running dcpwizard binary: {e}");
                    std::process::exit(1);
                }
            };
            let webhook = webhook_url.map(|url| postkit::webhook::WebhookConfig {
                url,
                ..Default::default()
            });

            dcpwizard_core::watch::watch_directory(
                &watch_dir,
                std::time::Duration::from_secs(interval),
                &|| false,
                |master| {
                    let Some(stem) = master.file_stem().and_then(|s| s.to_str()) else {
                        tracing::error!("cannot read a file stem from {}", master.display());
                        return;
                    };
                    let package_dir = output_dir.join(stem);
                    let log_path = output_dir.join(format!("{stem}.log"));

                    let audio = watch_dir.join(format!("{stem}.{AUDIO_SIDECAR_EXTENSION}"));
                    let subtitle = watch_dir.join(format!("{stem}.{SUBTITLE_SIDECAR_EXTENSION}"));
                    let audio = audio.is_file().then_some(audio);
                    let subtitle = subtitle.is_file().then_some(subtitle);

                    let mut arguments: Vec<std::ffi::OsString> = vec![
                        "create".into(),
                        "--title".into(),
                        stem.into(),
                        "--video".into(),
                        master.into(),
                        "--output".into(),
                        package_dir.as_path().into(),
                    ];
                    if let Some(audio) = audio.as_deref() {
                        arguments.push("--audio".into());
                        arguments.push(audio.into());
                    }
                    if let Some(subtitle) = subtitle.as_deref() {
                        arguments.push("--subtitle".into());
                        arguments.push(subtitle.into());
                    }
                    arguments.extend(create_arguments.iter().map(std::ffi::OsString::from));

                    let log = match std::fs::File::create(&log_path) {
                        Ok(file) => file,
                        Err(e) => {
                            tracing::error!("cannot write {}: {e}", log_path.display());
                            return;
                        }
                    };
                    let log_for_stderr = match log.try_clone() {
                        Ok(file) => file,
                        Err(e) => {
                            tracing::error!("cannot write {}: {e}", log_path.display());
                            return;
                        }
                    };

                    tracing::info!(
                        "building {} from {}",
                        package_dir.display(),
                        master.display()
                    );
                    let started = std::time::Instant::now();
                    let outcome = std::process::Command::new(&executable)
                        .args(&arguments)
                        .stdout(std::process::Stdio::from(log))
                        .stderr(std::process::Stdio::from(log_for_stderr))
                        .status();
                    let elapsed_seconds = started.elapsed().as_secs_f64();

                    let failure = match outcome {
                        Ok(status) if status.success() => None,
                        Ok(status) => Some(match status.code() {
                            Some(code) => {
                                format!("create exited {code}, log at {}", log_path.display())
                            }
                            None => format!(
                                "create was killed by a signal, log at {}",
                                log_path.display()
                            ),
                        }),
                        Err(e) => Some(format!("could not run create: {e}")),
                    };
                    let sidecars = [audio.as_deref(), subtitle.as_deref()]
                        .into_iter()
                        .flatten();

                    let Some(message) = failure else {
                        let done = watch_dir.join(DONE_DIRECTORY_NAME);
                        move_aside(master, &done);
                        for sidecar in sidecars {
                            move_aside(sidecar, &done);
                        }
                        tracing::info!("built {} in {elapsed_seconds:.1} s", package_dir.display());
                        post_event(
                            webhook.as_ref(),
                            "dcp.created",
                            stem,
                            postkit::webhook::build_job_completed_payload(
                                stem,
                                &package_dir,
                                elapsed_seconds,
                            ),
                        );
                        return;
                    };

                    let failed = watch_dir.join(FAILED_DIRECTORY_NAME);
                    move_aside(master, &failed);
                    for sidecar in sidecars {
                        move_aside(sidecar, &failed);
                    }
                    tracing::error!("create failed for {stem}: see {}", log_path.display());
                    post_event(
                        webhook.as_ref(),
                        "dcp.failed",
                        stem,
                        postkit::webhook::build_job_failed_payload(stem, &message),
                    );
                },
            );
            0
        }

        Commands::Export {
            input,
            output,
            format,
            crf,
            audio,
        } => {
            use dcpwizard_core::export::{ExportConfig, ExportFormat, export_dcp};
            let fmt = match format.to_lowercase().as_str() {
                "prores" => ExportFormat::ProRes,
                "h264" | "x264" | "avc" => ExportFormat::H264,
                "h265" | "hevc" | "x265" => ExportFormat::H265,
                "dnxhr" | "dnxhd" => ExportFormat::DnxHr,
                "image-sequence" | "images" | "png" => ExportFormat::ImageSequence,
                other => {
                    tracing::error!(
                        "unknown export format '{other}'; use prores, h264, h265, dnxhr or image-sequence"
                    );
                    std::process::exit(1);
                }
            };
            let config = ExportConfig {
                input_mxf: PathBuf::from(input),
                output_path: PathBuf::from(output),
                format: fmt,
                quality_crf: crf,
                audio_mxf: audio.map(PathBuf::from),
            };
            match export_dcp(&config) {
                Ok(()) => 0,
                Err(message) => {
                    eprintln!("export failed: {message}");
                    1
                }
            }
        }

        Commands::Completion { shell } => {
            print!(
                "{}",
                dcpwizard_core::shell_completion::generate_completion(&shell, "dcpwizard")
            );
            0
        }

        Commands::Daemon => {
            let addr = dcpwizard_core::job_queue::daemon_addr();
            println!("Starting dcpwizard daemon on {addr}...");
            let queue = dcpwizard_core::job_queue::JobQueue::new();
            dcpwizard_core::job_queue::start_daemon_ipc(&queue)
        }

        Commands::SubtitleConvert {
            input,
            output,
            language,
            fps,
            vposition,
        } => {
            let input_path = PathBuf::from(&input);
            let output_path = PathBuf::from(&output);
            if !input_path.exists() {
                tracing::error!("Input file not found: {input}");
                std::process::exit(1);
            }
            match dcpwizard_core::subtitle::convert_srt_to_dcp_xml(
                &input_path,
                &output_path,
                &language,
                fps,
                vposition,
            ) {
                Ok(()) => {
                    tracing::info!(
                        "Converted {} -> {} (lang={}, fps={}, vposition={})",
                        input,
                        output,
                        language,
                        fps,
                        vposition
                    );
                    0
                }
                Err(e) => {
                    tracing::error!("Subtitle conversion failed: {e}");
                    1
                }
            }
        }

        Commands::SubtitleExtract { input, output } => {
            let input_path = PathBuf::from(&input);
            let output_path = PathBuf::from(&output);
            if !input_path.exists() {
                tracing::error!("Input not found: {input}");
                std::process::exit(1);
            }
            match dcpwizard_core::subtitle_extract::extract(&input_path, &output_path) {
                Ok(()) => {
                    tracing::info!("Extracted subtitles -> {output}");
                    0
                }
                Err(e) => {
                    tracing::error!("Subtitle extraction failed: {e}");
                    1
                }
            }
        }
        Commands::SubtitleEdit {
            input,
            output,
            list,
            shift_ms,
            index,
            text,
            set_start_ms,
            set_end_ms,
            fps,
        } => {
            use dcpwizard_core::subtitle_edit as se;
            let input_path = PathBuf::from(&input);
            let mut cues = match se::load(&input_path, fps) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to load subtitles: {e}");
                    std::process::exit(1);
                }
            };
            if list {
                for (i, c) in cues.iter().enumerate() {
                    println!("{}", se::summary_line(i + 1, c));
                }
                0
            } else {
                if let Some(delta) = shift_ms {
                    se::shift_all(&mut cues, delta);
                }
                if let Some(idx) = index {
                    if let Some(t) = text.as_deref()
                        && let Err(e) = se::set_text(&mut cues, idx, t)
                    {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                    match (set_start_ms, set_end_ms) {
                        (Some(s), Some(e)) => {
                            if let Err(err) = se::set_timing(&mut cues, idx, s, e) {
                                tracing::error!("{err}");
                                std::process::exit(1);
                            }
                        }
                        (None, None) => {}
                        _ => {
                            tracing::error!(
                                "--set-start-ms and --set-end-ms must be given together"
                            );
                            std::process::exit(1);
                        }
                    }
                } else if text.is_some() || set_start_ms.is_some() || set_end_ms.is_some() {
                    tracing::error!("--text / --set-start-ms / --set-end-ms need --index");
                    std::process::exit(1);
                }
                let Some(out) = output else {
                    tracing::error!("--output is required to write edits (use --list to inspect)");
                    std::process::exit(1);
                };
                match std::fs::write(&out, se::format_srt(&cues)) {
                    Ok(()) => {
                        tracing::info!("Wrote {} cues -> {out}", cues.len());
                        0
                    }
                    Err(e) => {
                        tracing::error!("Failed to write {out}: {e}");
                        1
                    }
                }
            }
        }

        Commands::Burnin {
            input,
            subtitles,
            output,
            font_size,
            colour,
            position,
            video_codec,
            crf,
        } => {
            let opts = dcpwizard_core::burnin::BurninOptions {
                input: PathBuf::from(&input),
                output: PathBuf::from(&output),
                subtitle_file: Some(PathBuf::from(&subtitles)),
                font_size,
                font_colour: colour.unwrap_or_default(),
                position,
                video_codec: video_codec.unwrap_or_default(),
                video_crf: crf,
                ..Default::default()
            };
            match dcpwizard_core::burnin::burnin(&opts) {
                Ok(()) => {
                    tracing::info!("Burned subtitles into: {output}");
                    0
                }
                Err(e) => {
                    tracing::error!("Burn-in failed: {e}");
                    1
                }
            }
        }

        Commands::Convert {
            input,
            output,
            target,
            method,
        } => {
            // Parse target resolution
            let (tw, th) = match target.to_lowercase().as_str() {
                "2k-scope" => (2048, 858),
                "2k-flat" => (1998, 1080),
                "2k-full" => (2048, 1080),
                "4k-scope" => (4096, 1716),
                "4k-flat" => (3996, 2160),
                "4k-full" => (4096, 2160),
                _ => {
                    tracing::error!(
                        "Unknown target: {target}. Use: 2k-scope, 2k-flat, 2k-full, 4k-scope, 4k-flat, 4k-full"
                    );
                    std::process::exit(1);
                }
            };

            let vf = match method.to_lowercase().as_str() {
                "letterbox" => format!(
                    "scale={tw}:{th}:force_original_aspect_ratio=decrease,pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2"
                ),
                "crop" => {
                    format!("scale={tw}:{th}:force_original_aspect_ratio=increase,crop={tw}:{th}")
                }
                "scale" => format!("scale={tw}:{th}"),
                _ => {
                    tracing::error!("Unknown method: {method}. Use: letterbox, crop, or scale");
                    std::process::exit(1);
                }
            };

            let status = std::process::Command::new("ffmpeg")
                .arg("-y")
                .arg("-i")
                .arg(&input)
                .arg("-vf")
                .arg(&vf)
                .arg("-c:a")
                .arg("copy")
                .arg(&output)
                .status();
            match status {
                Ok(s) if s.success() => {
                    tracing::info!("Converted to {target} ({method}): {output} ({}x{})", tw, th);
                    0
                }
                Ok(s) => {
                    tracing::error!("ffmpeg exited with code {}", s.code().unwrap_or(-1));
                    1
                }
                Err(e) => {
                    tracing::error!("Failed to run ffmpeg: {e}");
                    1
                }
            }
        }

        Commands::Dcdm {
            input,
            output,
            colour_space,
            lut,
        } => {
            let cs = parse_colour_space(&colour_space);
            let opts = postkit::dcdm::DcdmOptions {
                input_dir: std::path::PathBuf::from(&input),
                output_dir: std::path::PathBuf::from(&output),
                encoding: postkit::dcdm::DcdmColourEncoding::Xyz12Bit,
                width: 0,
                height: 0,
                fps_num: 24,
                fps_den: 1,
                colour_space: format!("{cs:?}"),
                target: postkit::dcdm::DcdmTarget::Xyz,
                lut_path: lut.map(std::path::PathBuf::from).unwrap_or_default(),
            };
            let result = postkit::dcdm::create_dcdm(&opts);
            if result.success {
                tracing::info!("DCDM created: {} frames written", result.frames_written);
                0
            } else {
                tracing::error!("DCDM creation failed: {}", result.error);
                1
            }
        }

        Commands::Colour {
            input,
            output,
            source,
            target,
            lut,
        } => {
            // X'Y'Z' (DCDM) and P3-D65 are dcdm-module transforms, not ffmpeg
            // colorspace-filter targets; route them through the real
            // Rec.709/P3/Rec.2020 transform (fails loud on an unsupported source).
            if let Some(dcdm_target) = parse_dcdm_target(&target) {
                let opts = postkit::dcdm::DcdmOptions {
                    input_dir: std::path::PathBuf::from(&input),
                    output_dir: std::path::PathBuf::from(&output),
                    encoding: postkit::dcdm::DcdmColourEncoding::Xyz12Bit,
                    width: 0,
                    height: 0,
                    fps_num: 24,
                    fps_den: 1,
                    colour_space: source.clone(),
                    target: dcdm_target,
                    lut_path: lut.map(std::path::PathBuf::from).unwrap_or_default(),
                };
                let result = postkit::dcdm::create_dcdm(&opts);
                if result.success {
                    tracing::info!(
                        "Colour converted {source} -> {target}: {} frames written",
                        result.frames_written
                    );
                    0
                } else {
                    tracing::error!("Colour conversion failed: {}", result.error);
                    1
                }
            } else {
                let opts = postkit::colour::ColourConvertOptions {
                    input: std::path::PathBuf::from(&input),
                    output: std::path::PathBuf::from(&output),
                    source_space: parse_colour_space(&source),
                    target_space: parse_colour_space(&target),
                    lut_path: lut.map(std::path::PathBuf::from),
                };
                match postkit::colour::convert_colour(&opts) {
                    Ok(()) => {
                        tracing::info!("Colour converted {source} -> {target}: {output}");
                        0
                    }
                    Err(e) => {
                        tracing::error!("Colour conversion failed: {e}");
                        1
                    }
                }
            }
        }

        Commands::Conform {
            input,
            media_dir,
            output,
            json,
            signer_opts,
        } => match dcpwizard_core::conform::parse_timeline(std::path::Path::new(&input)) {
            Err(e) => {
                tracing::error!("Timeline parse failed: {e}");
                1
            }
            Ok(timeline) => {
                if let Some(media_dir) = media_dir {
                    run_conform_assembly(
                        &timeline,
                        &media_dir,
                        output.as_deref(),
                        package_signer(&signer_opts).as_ref(),
                    )
                } else if json {
                    println!("{}", serde_json::to_string_pretty(&timeline).unwrap());
                    0
                } else {
                    println!("Timeline: {}", timeline.title);
                    println!("Format: {:?}", timeline.format);
                    println!("Frame rate: {}", timeline.frame_rate);
                    println!("Events: {}", timeline.events.len());
                    for (i, evt) in timeline.events.iter().enumerate() {
                        println!("  [{i}] {} -> {}", evt.source_in, evt.source_out);
                    }
                    if !timeline.skipped.is_empty() {
                        println!(
                            "Skipped: {} construct(s) with no place in a reel plan, listed in \
                             the warnings above",
                            timeline.skipped.len()
                        );
                    }
                    0
                }
            }
        },

        Commands::FrameExtract {
            input,
            frame,
            output,
            key,
            keys_json,
        } => {
            let key = match postkit::preview::resolve_picture_key(
                std::path::Path::new(&input),
                key.as_deref(),
                keys_json.as_deref().map(std::path::Path::new),
            ) {
                Ok(key) => key,
                Err(error) => {
                    tracing::error!("{error}");
                    std::process::exit(1);
                }
            };
            postkit::preview::extract_frame(
                std::path::Path::new(&input),
                frame,
                std::path::Path::new(&output),
                key,
            )
        }

        Commands::DvInject { input, rpu, output } => {
            let opts = postkit::dolby_vision::DolbyVisionOptions {
                input: std::path::PathBuf::from(&input),
                rpu_file: std::path::PathBuf::from(&rpu),
                profile: postkit::dolby_vision::DolbyVisionProfile::Profile8,
                output: std::path::PathBuf::from(&output),
                embed_rpu: true,
            };
            dcpwizard_core::dolby_vision::inject_dolby_vision(&opts)
        }

        Commands::Hdr10Inject {
            input,
            output,
            max_cll,
            max_fall,
        } => {
            let opts = postkit::dolby_vision::HdrMetadataOptions {
                input: std::path::PathBuf::from(&input),
                hdr_type: postkit::dolby_vision::HdrType::Hdr10,
                hdr10: postkit::dolby_vision::Hdr10Metadata {
                    display_primaries_rx: P3D65_MASTERING_DISPLAY.red.0,
                    display_primaries_ry: P3D65_MASTERING_DISPLAY.red.1,
                    display_primaries_gx: P3D65_MASTERING_DISPLAY.green.0,
                    display_primaries_gy: P3D65_MASTERING_DISPLAY.green.1,
                    display_primaries_bx: P3D65_MASTERING_DISPLAY.blue.0,
                    display_primaries_by: P3D65_MASTERING_DISPLAY.blue.1,
                    white_point_x: P3D65_MASTERING_DISPLAY.white_point.0,
                    white_point_y: P3D65_MASTERING_DISPLAY.white_point.1,
                    max_luminance: P3D65_MASTERING_DISPLAY.max_luminance,
                    min_luminance: P3D65_MASTERING_DISPLAY.min_luminance,
                    max_cll,
                    max_fall,
                },
                dolby_vision_xml: std::path::PathBuf::new(),
                output: std::path::PathBuf::from(&output),
            };
            dcpwizard_core::dolby_vision::inject_hdr10_metadata(&opts)
        }

        Commands::HdrConvert {
            input,
            output,
            target,
        } => dcpwizard_core::dolby_vision::convert_hdr(
            std::path::Path::new(&input),
            target.into(),
            std::path::Path::new(&output),
        ),

        Commands::Watermark {
            input,
            output,
            payload,
            font_size,
            colour,
            position,
            font,
            video_bit_rate,
            kdm,
            recipient_key,
            keys,
        } => {
            let options = match watermark_options(
                Some(&payload),
                font_size,
                colour.as_deref(),
                position.as_deref(),
                &WATERMARK_COMMAND_FLAGS,
            ) {
                Ok(Some(options)) => options,
                Ok(None) => unreachable!("the payload is a required flag"),
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            let mark = match dcpwizard_core::watermark::watermark_burn(
                &options,
                font.as_deref().map(Path::new),
                WATERMARK_BURN_FPS,
            ) {
                Ok(mark) => mark,
                Err(e) => {
                    tracing::error!("{e}");
                    std::process::exit(1);
                }
            };
            let config = dcpwizard_core::j2k_transcode::DcpTranscodeConfig {
                input_dir: PathBuf::from(input),
                output_dir: PathBuf::from(output),
                target_bitrate_mbps: video_bit_rate,
                target_width: 0,
                target_height: 0,
                kdm: kdm.map(PathBuf::from),
                recipient_key: recipient_key.map(PathBuf::from),
                keys: keys.map(PathBuf::from),
                watermark: Some(mark),
            };
            dcpwizard_core::j2k_transcode::transcode_dcp(&config)
        }

        Commands::Certificate { action } => match action {
            CertAction::Chain {
                organization,
                output,
            } => {
                let output_dir = PathBuf::from(&output);
                dcpwizard_core::certificate::generate_chain(&organization, &output_dir)
            }
            CertAction::Generate {
                cert_type,
                cn,
                organization,
                output_cert,
                output_key,
                issuer_cert,
                issuer_key,
                key_bits,
                validity_days,
            } => {
                let ct = match cert_type.to_lowercase().as_str() {
                    "root" => dcpwizard_core::certificate::CertType::Root,
                    "intermediate" => dcpwizard_core::certificate::CertType::Intermediate,
                    "leaf" => dcpwizard_core::certificate::CertType::Leaf,
                    _ => dcpwizard_core::certificate::CertType::Signer,
                };
                let opts = dcpwizard_core::certificate::CertOptions {
                    cert_type: ct,
                    common_name: cn,
                    organization,
                    output_cert: PathBuf::from(&output_cert),
                    output_key: PathBuf::from(&output_key),
                    issuer_cert: issuer_cert.map(PathBuf::from).unwrap_or_default(),
                    issuer_key: issuer_key.map(PathBuf::from).unwrap_or_default(),
                    key_bits,
                    validity_days,
                    ..Default::default()
                };
                dcpwizard_core::certificate::generate_certificate(&opts)
            }
            CertAction::Inspect { cert_file } => {
                let info = dcpwizard_core::certificate::read_certificate(Path::new(&cert_file));
                println!("Subject CN:  {}", info.subject_cn);
                println!("Issuer CN:   {}", info.issuer_cn);
                println!("Serial:      {}", info.serial);
                println!("Not Before:  {}", info.not_before);
                println!("Not After:   {}", info.not_after);
                println!("Key Size:    {} bits", info.key_bits);
                println!("Is CA:       {}", info.is_ca);
                println!("Expired:     {}", info.is_expired);
                println!("Thumbprint:  {}", info.thumbprint);
                0
            }
        },

        Commands::Batch { action } => {
            use dcpwizard_core::job_queue::{IpcRequest, IpcResponse, send_ipc_request};

            match action {
                BatchAction::List => match send_ipc_request(&IpcRequest::List) {
                    Ok(IpcResponse::Jobs(jobs)) => {
                        if jobs.is_empty() {
                            println!("No jobs in queue");
                        } else {
                            println!(
                                "{:<38} {:<12} {:<10} {:<14} Message",
                                "ID", "State", "Progress", "Type"
                            );
                            for j in &jobs {
                                println!(
                                    "{:<38} {:?} {:<10}% {:?} {}",
                                    j.id, j.state, j.progress_percent, j.job_type, j.message
                                );
                            }
                        }
                        0
                    }
                    Ok(IpcResponse::Error(e)) => {
                        tracing::error!("{e}");
                        1
                    }
                    Err(e) => {
                        tracing::error!("{e}");
                        1
                    }
                    _ => 1,
                },
                BatchAction::Add { r#type, params } => {
                    if !dcpwizard_core::job_queue::is_daemon_running() {
                        tracing::error!("Daemon is not running. Start it with: dcpwizard daemon");
                        std::process::exit(1);
                    }
                    let job_type = match r#type.as_str() {
                        "create-dcp" => dcpwizard_core::job_queue::JobType::CreateDcp,
                        "verify-dcp" => dcpwizard_core::job_queue::JobType::VerifyDcp,
                        "export-dcp" => dcpwizard_core::job_queue::JobType::ExportDcp,
                        "import-video" => dcpwizard_core::job_queue::JobType::ImportVideo,
                        "encode-j2k" => dcpwizard_core::job_queue::JobType::EncodeJ2k,
                        "wrap-mxf" => dcpwizard_core::job_queue::JobType::WrapMxf,
                        "copy-to-drive" => dcpwizard_core::job_queue::JobType::CopyToDrive,
                        other => {
                            tracing::error!("Unknown job type: {other}");
                            std::process::exit(1);
                        }
                    };
                    match send_ipc_request(&IpcRequest::Submit { job_type, params }) {
                        Ok(IpcResponse::Submitted { id }) => {
                            println!("Submitted job {id}");
                            0
                        }
                        Ok(IpcResponse::Error(e)) => {
                            tracing::error!("{e}");
                            1
                        }
                        Err(e) => {
                            tracing::error!("{e}");
                            1
                        }
                        _ => 1,
                    }
                }
                BatchAction::Cancel { id } => {
                    match send_ipc_request(&IpcRequest::Cancel { id: id.clone() }) {
                        Ok(IpcResponse::Cancelled(true)) => {
                            println!("Cancelled job {id}");
                            0
                        }
                        Ok(IpcResponse::Cancelled(false)) => {
                            println!("Could not cancel job {id}");
                            1
                        }
                        Ok(IpcResponse::Error(e)) => {
                            tracing::error!("{e}");
                            1
                        }
                        Err(e) => {
                            tracing::error!("{e}");
                            1
                        }
                        _ => 1,
                    }
                }
            }
        }

        Commands::KdmBatch {
            cpl_id,
            content_title,
            certs,
            cert_dir,
            cinemas,
            screens,
            db,
            signer_cert,
            signer_key,
            signer_chain,
            output_dir,
            valid_from,
            valid_to,
            template,
            templates_file,
            history_file,
            email_to,
            smtp_config,
            email_only_additional,
            keys,
            format,
            kdm_options,
        } => run_kdm_batch(KdmBatchArgs {
            cpl_id,
            content_title,
            certs,
            cert_dir,
            cinemas,
            screens,
            db,
            signer_cert,
            signer_key,
            signer_chain,
            output_dir,
            valid_from,
            valid_to,
            template,
            templates_file,
            history_file,
            email_to,
            smtp_config,
            email_only_additional,
            keys,
            format,
            options: kdm_options.into(),
        }),

        Commands::Cinema { db, action } => run_cinema(db, action),

        Commands::KdmHistory {
            history_file,
            title,
            recipient,
            since,
            until,
        } => run_kdm_history(history_file, title, recipient, since, until),

        Commands::KdmTemplate {
            templates_file,
            action,
        } => run_kdm_template(templates_file, action),

        Commands::Library {
            library_dir,
            action,
        } => run_library(library_dir, action),

        Commands::CertFetch {
            vendor,
            serial,
            device_type,
            user,
            password,
            output,
        } => run_cert_fetch(vendor, serial, device_type, user, password, output),

        Commands::Trailer {
            content,
            output,
            title,
            rating,
            rating_system,
            band,
            countdown,
            fps,
        } => {
            let content_title = trailer_content_title(&title, Path::new(&content));
            let opts = postkit::trailer::TrailerOptions {
                content_dir: PathBuf::from(&content),
                audio_file: PathBuf::new(),
                output_dir: PathBuf::from(&output),
                title,
                rating,
                rating_system: match rating_system.to_lowercase().as_str() {
                    "bbfc" => postkit::trailer::RatingSystem::Bbfc,
                    "fsk" => postkit::trailer::RatingSystem::Fsk,
                    "custom" => postkit::trailer::RatingSystem::Custom,
                    _ => postkit::trailer::RatingSystem::Mpaa,
                },
                band: match band.to_lowercase().as_str() {
                    "red" => postkit::trailer::TrailerBand::Red,
                    "yellow" => postkit::trailer::TrailerBand::Yellow,
                    _ => postkit::trailer::TrailerBand::Green,
                },
                countdown_seconds: countdown,
                fps_num: fps,
                fps_den: 1,
            };
            let result = postkit::trailer::package_trailer(&opts);
            if !result.success {
                tracing::error!("Trailer packaging failed: {}", result.error);
                1
            } else {
                tracing::info!(
                    "Trailer packaged: {} ({})",
                    result.output_dir.display(),
                    result.output_file.display()
                );
                // route the packaged mp4 through the encode + create path so the
                // deliverable is a real DCP, not just an mp4.
                trailer_to_dcp(&result.output_file, &result.output_dir, fps, &content_title)
            }
        }

        Commands::Markers {
            frames,
            markers,
            fps,
            xml,
        } => {
            let entries =
                match dcpwizard_core::markers::markers_for_composition(&markers, fps, frames) {
                    Ok(entries) => entries,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
            if xml {
                print!("{}", dcpwizard_core::markers::markers_to_xml(&entries, ""));
            } else if entries.is_empty() {
                println!("No markers (composition length is 0 frames)");
            } else {
                for m in &entries {
                    println!("{}\t{}", m.marker.label(), m.frame);
                }
            }
            0
        }

        Commands::Accessibility { dcp_dir, standard } => {
            let std_val = postkit::accessibility::AccessibilityStandard::from(standard);
            let result =
                dcpwizard_core::accessibility::check_accessibility(Path::new(&dcp_dir), std_val);
            println!("Standard:  {:?}", result.standard);
            println!("Compliant: {}", result.compliant);
            println!("Errors:    {}", result.errors);
            println!("Warnings:  {}", result.warnings);
            for f in &result.findings {
                println!(
                    "  [{:?}] {} ({:?}): {}",
                    f.severity, f.rule_id, f.track_type, f.description
                );
            }
            if result.compliant { 0 } else { 1 }
        }

        Commands::Webhook {
            url,
            event,
            job_id,
            secret,
            payload,
        } => {
            let config = postkit::webhook::WebhookConfig {
                url,
                secret,
                ..Default::default()
            };
            let result = if payload.is_empty() && event == "ping" {
                postkit::webhook::test_webhook(&config)
            } else {
                let evt = postkit::webhook::WebhookEvent {
                    event_type: event,
                    job_id,
                    payload_json: payload,
                    timestamp: String::new(),
                };
                postkit::webhook::send_webhook(&config, &evt)
            };
            if result.success {
                tracing::info!(
                    "Webhook delivered (HTTP {}, {} attempt(s))",
                    result.http_status,
                    result.attempts
                );
                0
            } else {
                tracing::error!(
                    "Webhook failed after {} attempt(s): {}",
                    result.attempts,
                    result.error
                );
                1
            }
        }

        Commands::Tms {
            package,
            tms_config,
        } => match load_tms_config(tms_config.as_deref()).and_then(|config| {
            dcpwizard_core::tms_upload::upload_package(&config, Path::new(&package))
        }) {
            Ok(()) => 0,
            Err(e) => {
                tracing::error!("{e}");
                1
            }
        },

        Commands::Version { action } => match action {
            VersionAction::Record {
                db,
                package_uuid,
                title,
                version,
                destination,
                method,
                verified,
            } => {
                let mut tracker = dcpwizard_core::version_tracker::VersionTracker::new();
                if !tracker.open(Path::new(&db)) {
                    tracing::error!("Failed to open tracker database: {db}");
                    std::process::exit(1);
                }
                let record = dcpwizard_core::version_tracker::DeliveryRecord {
                    package_uuid,
                    title,
                    version,
                    destination,
                    delivery_method: method,
                    timestamp: dcpwizard_core::version_tracker::now_iso(),
                    verified,
                };
                if tracker.record(&record) {
                    println!("Recorded delivery of {}", record.package_uuid);
                    0
                } else {
                    tracing::error!("Failed to record delivery");
                    1
                }
            }
            VersionAction::List {
                db,
                package_uuid,
                destination,
            } => {
                let mut tracker = dcpwizard_core::version_tracker::VersionTracker::new();
                if !tracker.open(Path::new(&db)) {
                    tracing::error!("Failed to open tracker database: {db}");
                    std::process::exit(1);
                }
                let query = dcpwizard_core::version_tracker::VersionQuery {
                    package_uuid,
                    destination,
                    ..Default::default()
                };
                let records = tracker.query(&query);
                if records.is_empty() {
                    println!("No deliveries recorded");
                } else {
                    for r in &records {
                        println!(
                            "{}  {}  {}  -> {}  ({}, verified={})",
                            r.timestamp,
                            r.package_uuid,
                            r.title,
                            r.destination,
                            r.delivery_method,
                            r.verified
                        );
                    }
                }
                0
            }
            VersionAction::Export { db, output } => {
                let mut tracker = dcpwizard_core::version_tracker::VersionTracker::new();
                if !tracker.open(Path::new(&db)) {
                    tracing::error!("Failed to open tracker database: {db}");
                    std::process::exit(1);
                }
                let out = PathBuf::from(&output);
                let ok = if output.to_lowercase().ends_with(".csv") {
                    tracker.export_csv(&out)
                } else {
                    tracker.export_json(&out)
                };
                if ok {
                    println!("Exported delivery history to {output}");
                    0
                } else {
                    tracing::error!("Failed to export delivery history");
                    1
                }
            }
        },

        Commands::Dashboard { action } => {
            // register/list/status/matrix operate on postkit's default database;
            // ensure its schema exists first.
            let db_path = dcpwizard_core::dashboard::default_db_path();
            if dcpwizard_core::dashboard::init_database(&db_path) != 0 {
                tracing::error!("Failed to initialise dashboard database");
                std::process::exit(1);
            }
            match action {
                DashboardAction::Register {
                    uuid,
                    title,
                    version_type,
                    territory,
                    language,
                    standard,
                    dcp_path,
                    status,
                    kdm_recipients,
                } => {
                    let entry = dcpwizard_core::dashboard::VersionEntry {
                        uuid,
                        title,
                        version_type,
                        territory,
                        language,
                        standard,
                        dcp_path: PathBuf::from(dcp_path),
                        ov_uuid: String::new(),
                        created_date: dcpwizard_core::version_tracker::now_iso(),
                        status,
                        kdm_recipients,
                    };
                    if dcpwizard_core::dashboard::register_version(&entry) == 0 {
                        println!("Registered version {}", entry.uuid);
                        0
                    } else {
                        1
                    }
                }
                DashboardAction::List { territory, status } => {
                    let versions = dcpwizard_core::dashboard::list_versions(
                        territory.as_deref(),
                        status.as_deref(),
                    );
                    if versions.is_empty() {
                        println!("No versions registered");
                    } else {
                        for v in &versions {
                            println!(
                                "{}  {}  {}  {}  [{}]",
                                v.uuid, v.title, v.version_type, v.territory, v.status
                            );
                        }
                    }
                    0
                }
                DashboardAction::Status { uuid, status } => {
                    if dcpwizard_core::dashboard::update_status(&uuid, &status) == 0 {
                        println!("Updated {uuid} -> {status}");
                        0
                    } else {
                        tracing::error!("Failed to update status (unknown UUID?)");
                        1
                    }
                }
                DashboardAction::Matrix { output } => {
                    if dcpwizard_core::dashboard::export_distribution_matrix(Path::new(&output))
                        == 0
                    {
                        println!("Exported distribution matrix to {output}");
                        0
                    } else {
                        tracing::error!("Failed to export distribution matrix");
                        1
                    }
                }
                DashboardAction::Serve { port, bind } => {
                    let opts = dcpwizard_core::dashboard::DashboardOptions {
                        database_path: db_path,
                        http_port: port,
                        bind_address: bind,
                    };
                    dcpwizard_core::dashboard::serve_dashboard(&opts)
                }
            }
        }

        Commands::IngestPackage { dir, signer_opts } => {
            let code = dcpwizard_core::ingest_package::ingest_package(
                &PathBuf::from(&dir),
                package_signer(&signer_opts).as_ref(),
            );
            if code == 0 {
                println!("Repackaged {dir} (regenerated ASSETMAP and PKL)");
            }
            code
        }

        Commands::Combine {
            inputs,
            output,
            separate_pkls,
            sort,
            annotation,
            signer_opts,
        } => {
            let config = dcpwizard_core::combine::CombineConfig {
                inputs: inputs.iter().map(PathBuf::from).collect(),
                output_dir: PathBuf::from(&output),
                separate_pkls,
                sort,
                annotation,
                signer: package_signer(&signer_opts),
            };
            let code = dcpwizard_core::combine::combine(&config);
            if code == 0 {
                println!("Combined into {output}");
            }
            code
        }

        Commands::CreateVf {
            ov,
            output,
            title,
            replace_picture,
            replace_sound,
            replace_subtitle,
            add_subtitle,
            replace_ccap,
            add_ccap,
            subtitle_language,
            subtitle_font,
            signer_opts,
        } => {
            // Parse REEL=PATH into a per-reel map. picture/sound/subtitle/ccap share
            // reels; --add-* and --replace-* both set the track.
            #[derive(Clone, Copy)]
            enum Track {
                Picture,
                Sound,
                Subtitle,
                Ccap,
            }
            let mut reels: std::collections::BTreeMap<u32, dcpwizard_core::vf::ReplacementReel> =
                std::collections::BTreeMap::new();
            let mut parse_ok = true;
            for (specs, track) in [
                (&replace_picture, Track::Picture),
                (&replace_sound, Track::Sound),
                (&replace_subtitle, Track::Subtitle),
                (&add_subtitle, Track::Subtitle),
                (&replace_ccap, Track::Ccap),
                (&add_ccap, Track::Ccap),
            ] {
                for spec in specs {
                    let Some((reel_str, path)) = spec.split_once('=') else {
                        tracing::error!("bad --replace spec '{spec}', expected REEL=PATH");
                        parse_ok = false;
                        continue;
                    };
                    let Ok(reel_number) = reel_str.trim().parse::<u32>() else {
                        tracing::error!("bad reel number in '{spec}'");
                        parse_ok = false;
                        continue;
                    };
                    let entry =
                        reels
                            .entry(reel_number)
                            .or_insert(dcpwizard_core::vf::ReplacementReel {
                                reel_number,
                                ..Default::default()
                            });
                    let p = Some(PathBuf::from(path.trim()));
                    match track {
                        Track::Picture => entry.picture = p,
                        Track::Sound => entry.sound = p,
                        Track::Subtitle => entry.subtitle = p,
                        Track::Ccap => entry.ccap = p,
                    }
                }
            }

            if !parse_ok {
                1
            } else {
                let config = dcpwizard_core::vf::VfConfig {
                    ov_dir: PathBuf::from(&ov),
                    vf_dir: PathBuf::from(&output),
                    title,
                    subtitle_language,
                    subtitle_opts: dcpwizard_core::subtitle::SubtitleOptions {
                        font_path: subtitle_font.map(PathBuf::from),
                        ..Default::default()
                    },
                    replacement_reels: reels.into_values().collect(),
                    signer: package_signer(&signer_opts),
                };
                let code = dcpwizard_core::vf::create_vf(&config);
                if code == 0 {
                    println!("Created VF DCP at {output}");
                }
                code
            }
        }

        Commands::Assemble {
            input,
            output,
            title,
            signer_opts,
        } => {
            let config = dcpwizard_core::assemble::AssembleConfig {
                inputs: input.iter().map(PathBuf::from).collect(),
                output_dir: PathBuf::from(&output),
                title,
                signer: package_signer(&signer_opts),
            };
            let code = dcpwizard_core::assemble::assemble(&config);
            if code == 0 {
                println!("Assembled OV at {output}");
            }
            code
        }

        Commands::Edit {
            input,
            output,
            title,
            annotation,
            content_kind,
            issuer,
        } => {
            let config = dcpwizard_core::edit::EditConfig {
                input: PathBuf::from(&input),
                output: output.as_ref().map(PathBuf::from),
                title,
                annotation,
                content_kind,
                issuer,
            };
            let code = dcpwizard_core::edit::edit_dcp(&config);
            if code == 0 {
                println!(
                    "Edited DCP CPL metadata ({})",
                    output.as_deref().unwrap_or(&input)
                );
            }
            code
        }

        Commands::CreateMulti {
            compositions,
            output,
            standard,
            frame_rate,
            fourk,
            container,
            container_dims,
            subtitle_language,
            content_type,
            encrypt,
            key_out,
            signer_opts,
        } => {
            let comps =
                match dcpwizard_core::multi_cpl::load_compositions(&PathBuf::from(&compositions)) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
            let std_val = if standard == "interop" {
                dcpwizard_core::Standard::Interop
            } else {
                dcpwizard_core::Standard::Smpte
            };
            let (container_width, container_height) =
                match resolve_container(container.as_deref(), container_dims.as_deref(), fourk) {
                    Ok(dims) => dims,
                    Err(e) => {
                        tracing::error!("{e}");
                        std::process::exit(1);
                    }
                };
            let resolution = config_resolution(fourk, (container_width, container_height));
            let ct = content_type
                .as_deref()
                .and_then(dcpwizard_core::ContentType::from_abbrev)
                .unwrap_or_default();
            let config = dcpwizard_core::dcp::DcpConfig {
                title: String::new(),
                standard: std_val,
                resolution,
                content_type: ct,
                frame_rate_num: frame_rate,
                frame_rate_den: 1,
                encrypt,
                key_out: key_out.map(PathBuf::from),
                container_width,
                container_height,
                output_dir: PathBuf::from(&output),
                subtitle_language,
                signer: package_signer(&signer_opts),
                ..Default::default()
            };
            let code = dcpwizard_core::multi_cpl::create_multi_composition(&config, &comps);
            if code == 0 {
                println!("Created multi-composition DCP at {output}");
            }
            code
        }
    };

    if gpu_enabled {
        tracing::info!(
            "grok's accelerator plugin ran {} frames on the device",
            postkit::grok_encoder::accelerated_frames()
        );
    }

    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trailer_cpl_title_is_the_given_title_or_the_content_stem() {
        let content = Path::new("/media/Sunrise_Teaser.mov");
        assert_eq!(
            trailer_content_title("Sunrise", content),
            "Sunrise".to_string()
        );
        assert_eq!(
            trailer_content_title("  ", content),
            "Sunrise_Teaser".to_string()
        );
        assert_eq!(
            trailer_content_title("", Path::new("/")),
            TRAILER_FALLBACK_TITLE.to_string()
        );
    }

    #[test]
    fn colour_target_selects_dcdm_target() {
        assert_eq!(
            parse_dcdm_target("xyz"),
            Some(postkit::dcdm::DcdmTarget::Xyz)
        );
        assert_eq!(
            parse_dcdm_target("p3-d65"),
            Some(postkit::dcdm::DcdmTarget::P3D65)
        );
        assert_eq!(
            parse_dcdm_target("p3d65"),
            Some(postkit::dcdm::DcdmTarget::P3D65)
        );
        // ffmpeg colorspace targets are not dcdm-module targets
        assert_eq!(parse_dcdm_target("rec709"), None);
        assert_eq!(parse_dcdm_target("p3"), None);
    }

    #[test]
    fn a_named_container_names_its_own_dci_family() {
        let scope_4k = resolve_container(Some("4k-scope"), None, false).unwrap();
        assert_eq!(scope_4k, (4096, 1716));
        assert_eq!(
            config_resolution(false, scope_4k),
            dcpwizard_core::Resolution::FourK,
            "a 4K container is a 4K package without --fourk"
        );

        // --fourk beside a 2K container is accepted, and the container wins
        let scope_2k = resolve_container(Some("2k-scope"), None, true).unwrap();
        assert_eq!(scope_2k, (2048, 858));
        assert_eq!(
            config_resolution(true, scope_2k),
            dcpwizard_core::Resolution::TwoK
        );

        assert_eq!(
            config_resolution(true, NO_CONTAINER),
            dcpwizard_core::Resolution::FourK
        );
        assert_eq!(
            config_resolution(false, NO_CONTAINER),
            dcpwizard_core::Resolution::TwoK
        );
    }

    fn codestream_dir(root: &Path, frames: u64) -> PathBuf {
        let dir = root.join("j2k");
        std::fs::create_dir_all(&dir).unwrap();
        for index in 0..frames {
            std::fs::write(dir.join(format!("frame_{index:08}.j2c")), [index as u8]).unwrap();
        }
        dir
    }

    #[test]
    fn a_windowed_encode_leaves_the_codestreams_where_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let trim = resolve_trim(Some("2f"), Some("2f"), 12, 24).unwrap();
        assert_eq!(trim.kept_frames, 8);

        let window = trim.encode_window(Path::new("/in/movie.mov")).unwrap();
        assert_eq!(window.first_frame, 2);
        assert_eq!(window.frame_count, 8);

        // the encoder already wrote only the kept frames
        let encoded = codestream_dir(dir.path(), 8);
        let trimmed = dir.path().join("j2k_trimmed");
        let (picture, audio) = trim
            .apply(&encoded, &trimmed, None, 24, Some(window))
            .unwrap();
        assert_eq!(picture, encoded);
        assert!(audio.is_none());
        assert!(
            !trimmed.exists(),
            "a windowed encode must not relink a second copy of the picture"
        );
    }

    #[test]
    fn a_codestream_directory_is_relinked_instead() {
        let dir = tempfile::tempdir().unwrap();
        let source = codestream_dir(dir.path(), 12);
        let trim = resolve_trim(Some("2f"), Some("2f"), 12, 24).unwrap();
        assert!(
            trim.encode_window(&source).is_none(),
            "nothing encodes a codestream directory, so it has no window to ask for"
        );

        let trimmed = dir.path().join("j2k_trimmed");
        let (picture, _) = trim.apply(&source, &trimmed, None, 24, None).unwrap();
        assert_eq!(picture, trimmed);
        assert_eq!(dcpwizard_core::trim::frame_count(&trimmed), 8);
    }
}
