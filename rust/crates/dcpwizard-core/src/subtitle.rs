use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use postkit::subtitle_formats::{self, HAlign, Rgba, StyledCue, StyledRun, VAlign};
use postkit::subtitle_raster::{BurnEffect, BurnStyle, BurnStyleOverrides};

/// SMPTE 640 KB embedded-font size limit (ST 428-7 / interop).
const FONT_SIZE_LIMIT: usize = 640 * 1024;

/// The `ID` the packaged track's `LoadFont` declares and its `Font` names. A
/// `Font` carries it only when a `LoadFont` introduced it, because ST 428-7 has
/// nothing else for it to refer to.
const SUBTITLE_FONT_ID: &str = "font1";

/// The one namespace a ST 428-7 document declares on its root.
const DCST_NAMESPACE: &str = "http://www.smpte-ra.org/schemas/428-7/2010/DCST";

/// Bv2.1 §7.2.2 wants a SMPTE timed-text document to start at zero, so every
/// cue plays at the time it declares.
const DCST_START_TIME: &str = "00:00:00:00";

/// `IssueDate` shape: no timezone suffix and no fractional seconds, which is
/// what Deluxe QC accepts and libdcp warns about.
const DCST_ISSUE_DATE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// What a reel with nothing to show says: a space, which draws nothing but is
/// still a cue, so the document has the `Subtitle` element ST 428-7 requires.
const PLACEHOLDER_CUE_TEXT: &str = " ";

/// How long the placeholder cue runs from the reel start.
const PLACEHOLDER_CUE_SECONDS: u64 = 1;

/// What a text subtitle track says when this machine has no font to embed.
const NO_FONT_TO_EMBED: &str =
    "no font to embed for the subtitle track: install one or pass --subtitle-font";

/// How to handle right-to-left (Hebrew/Arabic) subtitle text (dom#860).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RtlMode {
    /// Reorder to visual order only when RTL characters are detected.
    #[default]
    Auto,
    /// Always reorder to visual order.
    On,
    /// Never reorder.
    Off,
}

/// Placement / rendering controls for subtitle conversion, applied to every
/// non-SMPTE-XML input (SRT and the styled formats). All fields default to the
/// previous centred-bottom behaviour so a plain `--subtitle x.srt` is unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubtitleOptions {
    /// Global horizontal alignment override: "left" | "center" | "right".
    pub halign: Option<String>,
    /// Global vertical anchor override: "top" | "center" | "bottom".
    pub valign: Option<String>,
    /// Global vertical position (percent from the valign edge) override.
    pub vposition: Option<f64>,
    /// 3D subtitle depth (SMPTE ST 428-7 Zposition), emitted on every cue.
    pub zposition: Option<f64>,
    /// RTL reordering mode (dom#860).
    pub rtl: RtlMode,
    /// Auto line-wrap at this many characters per line (dom#1626).
    pub wrap_cols: Option<usize>,
    /// TTF/OTF font to embed (subset to the used glyphs unless `no_subset`).
    pub font_path: Option<PathBuf>,
    /// Skip glyph subsetting and embed the whole font.
    pub no_subset: bool,
    /// How the packaged track looks: the `Font` attributes and the fades.
    pub appearance: TimedTextAppearance,
}

impl SubtitleOptions {
    /// The same options for a closed-caption track. The `--subtitle-*`
    /// appearance flags style the open subtitle track only, which is what the
    /// CLI refuses them without, so a caption keeps the default look and takes
    /// the font and the placement.
    pub fn for_closed_caption(&self) -> Self {
        SubtitleOptions {
            appearance: TimedTextAppearance::default(),
            ..self.clone()
        }
    }
}

/// Point size the packaged `Font` line carries when nothing names one.
const DEFAULT_TIMED_TEXT_SIZE: u32 = 42;

/// Opaque white, the `Font` line's text colour when nothing names one.
const DEFAULT_TIMED_TEXT_COLOUR: &str = "FFFFFFFF";

/// The ST 428-7 `Effect` the `Font` line carries when nothing names one.
const DEFAULT_TIMED_TEXT_EFFECT: &str = "shadow";

/// Opaque black, the `Font` line's effect colour when nothing names one.
const DEFAULT_TIMED_TEXT_EFFECT_COLOUR: &str = "FF000000";

/// A fade of a twelfth of a second, in frames at the edit rate, when nothing
/// names a length.
const DEFAULT_FADE_DIVISOR: f64 = 12.0;

/// How the packaged timed-text track looks: the ST 428-7 `Font` attributes and
/// the per-cue fade lengths. Colours are held as ARGB hex and the effect as the
/// `Effect` attribute spells it, because postkit's `Rgba` and `BurnEffect` carry
/// no serde derives and this rides along with [`SubtitleOptions`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedTextAppearance {
    pub font_size: Option<u32>,
    pub colour: Option<String>,
    pub effect: Option<String>,
    pub effect_colour: Option<String>,
    pub fade_up_ms: Option<u64>,
    pub fade_down_ms: Option<u64>,
}

impl TimedTextAppearance {
    /// Read the `--subtitle-*` appearance flags into the spellings the DCST
    /// carries, refusing a bad value under the flag's own name.
    pub fn from_flags(
        font_size: Option<u32>,
        colour: Option<&str>,
        effect: Option<&str>,
        effect_colour: Option<&str>,
        fade_up_ms: Option<u64>,
        fade_down_ms: Option<u64>,
    ) -> Result<Self, String> {
        Ok(TimedTextAppearance {
            font_size,
            colour: match colour {
                Some(text) => Some(argb(parse_colour_flag("--subtitle-colour", text)?)),
                None => None,
            },
            effect: match effect {
                Some(text) => Some(
                    dcst_effect_name(parse_effect_flag("--subtitle-effect", text)?).to_string(),
                ),
                None => None,
            },
            effect_colour: match effect_colour {
                Some(text) => Some(argb(parse_colour_flag("--subtitle-effect-colour", text)?)),
                None => None,
            },
            fade_up_ms,
            fade_down_ms,
        })
    }

    /// The ST 428-7 `Font` attributes, each falling back to what the packaged
    /// track has always carried.
    fn font_attributes(&self) -> String {
        format!(
            "Color=\"{}\" Size=\"{}\" Effect=\"{}\" EffectColor=\"{}\"",
            self.colour.as_deref().unwrap_or(DEFAULT_TIMED_TEXT_COLOUR),
            self.font_size.unwrap_or(DEFAULT_TIMED_TEXT_SIZE),
            self.effect.as_deref().unwrap_or(DEFAULT_TIMED_TEXT_EFFECT),
            self.effect_colour
                .as_deref()
                .unwrap_or(DEFAULT_TIMED_TEXT_EFFECT_COLOUR),
        )
    }

    /// The fade up and fade down lengths as timecodes at the edit rate.
    fn fades(&self, fps: u32) -> (String, String) {
        (
            fade_timecode(self.fade_up_ms, fps),
            fade_timecode(self.fade_down_ms, fps),
        )
    }
}

/// A fade length in milliseconds as a timecode, rounded to whole frames at
/// `fps`. Without one, the twelfth of a second the packaged tracks have always
/// used.
fn fade_timecode(ms: Option<u64>, fps: u32) -> String {
    let rate = fps.max(1) as f64;
    let frames = match ms {
        Some(ms) => (ms as f64 * rate / 1000.0).round(),
        None => (rate / DEFAULT_FADE_DIVISOR).round(),
    };
    frames_to_dcst(frames as u64, fps)
}

/// A colour flag written `RRGGBB` or `RRGGBBAA`, refused under the flag's own
/// name.
pub fn parse_colour_flag(flag: &str, text: &str) -> Result<Rgba, String> {
    Rgba::parse_hex(text).map_err(|e| format!("{flag}: {e}"))
}

/// An effect flag (none, outline or shadow), refused under the flag's own name.
pub fn parse_effect_flag(flag: &str, text: &str) -> Result<BurnEffect, String> {
    postkit::subtitle_raster::parse_burn_effect(text).map_err(|e| format!("{flag}: {e}"))
}

/// The ST 428-7 `Effect` spelling for an effect: the standard writes an outline
/// `border`, where postkit names it `Outline`.
fn dcst_effect_name(effect: BurnEffect) -> &'static str {
    match effect {
        BurnEffect::None => "none",
        BurnEffect::Outline => "border",
        BurnEffect::Shadow => "shadow",
    }
}

/// The head/tail trim already applied to the picture and sound, so timed text
/// can follow them. Cues slide back by `start_frames` and are clamped to the
/// `kept_frames` that survive; `kept_frames == 0` means nothing was trimmed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceTrim {
    pub start_frames: u64,
    pub kept_frames: u64,
}

impl SourceTrim {
    pub fn is_active(&self) -> bool {
        self.kept_frames > 0
    }
}

/// Where a source cue lands in the packaged programme: the trim already applied
/// to picture and sound, then the head padding prepended after it. Trim first,
/// pad second, matching the order the picture went through.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CueTiming {
    pub trim: SourceTrim,
    pub pad_head_frames: u64,
}

/// Move cues onto the trimmed programme's timeline: a cue wholly outside the
/// kept window is dropped, one straddling a boundary is clamped to it, and the
/// rest slide back so the first kept frame is zero.
pub fn apply_source_trim(cues: &[StyledCue], trim: SourceTrim, fps: u32) -> Vec<StyledCue> {
    if !trim.is_active() {
        return cues.to_vec();
    }
    let fps = fps.max(1) as u64;
    let to_ms = |frames: u64| frames * 1000 / fps;
    let window_start = to_ms(trim.start_frames);
    let window_end = to_ms(trim.start_frames + trim.kept_frames);
    cues.iter()
        .filter(|cue| cue.end_ms > window_start && cue.start_ms < window_end)
        .map(|cue| StyledCue {
            start_ms: cue.start_ms.max(window_start) - window_start,
            end_ms: cue.end_ms.min(window_end) - window_start,
            ..cue.clone()
        })
        .collect()
}

/// Result of building a subtitle track: the DCST XML plus any ancillary
/// resources (embedded font, bitmap PNGs) with the asset id each is referenced
/// by from the XML. `dcp.rs`/reel splitting wrap these into the timed-text MXF.
pub struct PreparedSubtitle {
    pub dcst_path: PathBuf,
    /// (file, asset id) pairs; the id matches the `urn:uuid` in the XML.
    pub resources: Vec<(PathBuf, [u8; 16])>,
}

/// Supported `--subtitle` input formats, detected from extension and content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleInputKind {
    Srt,
    Ass,
    Pac,
    Mks,
    Fcpxml,
    /// Interop DCSubtitle carrying PNG bitmap subs (dom#1376).
    InteropPng,
    /// A supplied SMPTE ST 428-7 DCST XML: wrapped unchanged, never re-rendered.
    SmpteDcstPassthrough,
}

/// Detect the subtitle input format. `.xml` is disambiguated by content:
/// a SMPTE `SubtitleReel` is passed through; a `DCSubtitle` with `<Image>`
/// elements is parsed as interop bitmap subs.
pub fn detect_subtitle_kind(path: &Path) -> Result<SubtitleInputKind, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "srt" => Ok(SubtitleInputKind::Srt),
        "ass" | "ssa" => Ok(SubtitleInputKind::Ass),
        "pac" => Ok(SubtitleInputKind::Pac),
        "mks" | "mkv" => Ok(SubtitleInputKind::Mks),
        "fcpxml" => Ok(SubtitleInputKind::Fcpxml),
        "xml" => {
            let head = read_head(path, 4096)?;
            if head.contains("DCSubtitle") && head.to_lowercase().contains("<image") {
                Ok(SubtitleInputKind::InteropPng)
            } else {
                // SMPTE DCST, or a text DCSubtitle we still wrap unchanged
                Ok(SubtitleInputKind::SmpteDcstPassthrough)
            }
        }
        other => Err(format!("unsupported subtitle format '.{other}'")),
    }
}

fn read_head(path: &Path, n: usize) -> Result<String, String> {
    use std::io::Read;
    let mut f =
        std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut buf = vec![0u8; n];
    let read = f.read(&mut buf).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf[..read]).into_owned())
}

/// Load any styled subtitle format into `StyledCue`s. Not for the SMPTE-DCST
/// pass-through kind (that XML is wrapped unchanged, never parsed to cues here).
pub fn load_styled_cues(path: &Path, fps: u32) -> Result<Vec<StyledCue>, String> {
    let kind = detect_subtitle_kind(path)?;
    let cues = match kind {
        SubtitleInputKind::Srt => {
            let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            postkit::subtitle_retime::parse_srt(&content)
                .into_iter()
                .filter(|c| !c.text.is_empty())
                .map(|c| StyledCue::text(c.start_ms, c.end_ms, vec![StyledRun::plain(c.text)]))
                .collect()
        }
        SubtitleInputKind::Ass => {
            let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let parsed = subtitle_formats::ass::parse_ass(&content).map_err(|e| e.to_string())?;
            for w in &parsed.warnings {
                tracing::warn!("ASS override tag not modelled, dropped: {w}");
            }
            parsed.cues
        }
        SubtitleInputKind::Pac => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            subtitle_formats::pac::parse_pac(&bytes, subtitle_formats::pac::CODEPAGE_LATIN)
                .map_err(|e| e.to_string())?
        }
        SubtitleInputKind::Mks => {
            subtitle_formats::mks::parse_mks(path, None).map_err(|e| e.to_string())?
        }
        SubtitleInputKind::Fcpxml => {
            let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            subtitle_formats::fcpxml::parse_fcpxml(&content).map_err(|e| e.to_string())?
        }
        SubtitleInputKind::InteropPng => {
            subtitle_formats::interop::parse_interop_png(path, fps as f64)
                .map_err(|e| e.to_string())?
        }
        SubtitleInputKind::SmpteDcstPassthrough => {
            return Err("SMPTE DCST XML is wrapped unchanged, not parsed to cues".into());
        }
    };
    if cues.is_empty() {
        return Err(format!("no subtitle cues in {}", path.display()));
    }
    Ok(cues)
}

/// Prepare `create --burn-subtitle`: parse the cue file and build the burn the
/// encoder threads composite onto every decoded frame.
///
/// `font` is the face to shape text with; without one the system faces fontdb
/// finds are used, and a machine with no font at all is an error rather than a
/// silently subtitle-free encode. `style` is what the `--burn-*` appearance
/// flags named, laid over postkit's defaults.
pub fn prepare_subtitle_burn(
    input: &Path,
    font: Option<&Path>,
    fps: postkit::encode::FrameRate,
    style: &BurnStyleOverrides,
) -> Result<std::sync::Arc<postkit::subtitle_raster::SubtitleBurn>, String> {
    if detect_subtitle_kind(input)? == SubtitleInputKind::SmpteDcstPassthrough {
        return Err(format!(
            "{} is SMPTE DCST XML, which has no cue reader here: burn from the SRT, ASS, PAC, \
             MKS, FCPXML or Interop source it was made from",
            input.display()
        ));
    }
    if let Some(path) = font
        && !path.is_file()
    {
        return Err(format!("burn-in font not found: {}", path.display()));
    }
    let style = style
        .apply(BurnStyle::default())
        .map_err(|e| format!("burn-in appearance: {e}"))?;
    // a frame-timed cue file is read against the DCP edit rate, which is whole
    let cues = load_styled_cues(input, fps.as_f64().round() as u32)?;
    postkit::subtitle_raster::SubtitleBurn::new(cues, font, style, fps.as_f64())
        .map(std::sync::Arc::new)
        .map_err(|e| format!("cannot burn {}: {e}", input.display()))
}

/// Read a timed-text input the way the wrap will, so a file the packager cannot
/// use is refused before the encode instead of after it. Supplied SMPTE DCST is
/// wrapped unchanged, so detecting the kind is all the reading it gets.
pub fn check_timed_text_readable(path: &Path, fps: u32) -> Result<(), String> {
    if detect_subtitle_kind(path)? == SubtitleInputKind::SmpteDcstPassthrough {
        return Ok(());
    }
    load_styled_cues(path, fps).map(|_| ())
}

/// Refuse a `--burn-subtitle` the encode cannot honour, before anything is
/// encoded.
///
/// `frames_already_xyz` covers every route that hands the encoder X'Y'Z'
/// frames: an `--source-colourspace xyz` source, the HDR-to-DCI LUT branch, and
/// `--hdr-already-pq`. Text is drawn in display RGB, so it would land in the
/// wrong space on any of them. P3 and Rec.2020 sources are fine: the burn goes
/// on first and the DCDM matrix converts it with the picture.
pub fn check_burn_supported(
    burn_path: &Path,
    timed_text_path: Option<&Path>,
    frames_already_xyz: bool,
    input_is_codestreams: bool,
) -> Result<(), String> {
    if !burn_path.is_file() {
        return Err(format!(
            "--burn-subtitle file not found: {}",
            burn_path.display()
        ));
    }
    if let Some(timed_text) = timed_text_path
        && same_file(burn_path, timed_text)
    {
        return Err(format!(
            "{} is given to both --burn-subtitle and --subtitle: a burnt-in subtitle must not \
             also be a timed-text track, so pick one",
            burn_path.display()
        ));
    }
    if input_is_codestreams {
        return Err(
            "--burn-subtitle needs frames to draw on, and a J2K directory is already compressed"
                .into(),
        );
    }
    if frames_already_xyz {
        return Err(
            "--burn-subtitle draws in display RGB, but this source reaches the encoder as \
             X'Y'Z' already (--source-colourspace xyz, --hdr-already-pq, or the HDR-to-DCI \
             LUT): burn from the display-RGB master instead"
                .into(),
        );
    }
    Ok(())
}

/// Whether two paths name the same file, falling back to the paths themselves
/// when either cannot be canonicalised.
fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// Build a subtitle track from any supported input, applying wrap/RTL/placement/
/// font options and the `timing` the packaged programme puts the cues on. Writes
/// the DCST XML to `out` and returns it plus any ancillary resources (font, PNGs)
/// to embed in the timed-text MXF. Callers wrap `[dcst_path]` + the resources.
pub fn prepare_subtitle_track(
    input: &Path,
    timing: CueTiming,
    lang: &str,
    fps: u32,
    opts: &SubtitleOptions,
    out: &Path,
) -> Result<PreparedSubtitle, String> {
    let mut cues = apply_source_trim(&load_styled_cues(input, fps)?, timing.trim, fps);

    // wrap first (adds '\n'), then RTL reorder each line to visual order
    if let Some(cols) = opts.wrap_cols.filter(|c| *c > 0) {
        cues = cues
            .iter()
            .map(|c| subtitle_formats::wrap::wrap_styled(c, cols))
            .collect();
    }
    apply_rtl(&mut cues, opts.rtl);

    // head padding shifts the program: slide every cue later by the pad, applied
    // in the frame domain so the timecodes stay frame-accurate
    let resources = write_dcst_styled(&cues, lang, fps, opts, timing.pad_head_frames, out)?;
    Ok(PreparedSubtitle {
        dcst_path: out.to_path_buf(),
        resources,
    })
}

/// Whether any cue draws text, which is what makes ST 428-7 ask for a `LoadFont`
/// and gives the subsetter glyphs to keep. A bitmap-only track has neither.
fn cues_have_text(cues: &[StyledCue]) -> bool {
    cues.iter()
        .any(|c| c.image.is_none() && !c.plain_text().is_empty())
}

/// The font file a track embeds: the caller's, or a system sans face found the
/// way the burn rasteriser finds one. `None` only for a track with no text at
/// all. A text track with no font anywhere is refused, because a `Font` naming a
/// face the package does not carry is what players fall back from.
fn font_to_embed(opts: &SubtitleOptions, cues: &[StyledCue]) -> Result<Option<PathBuf>, String> {
    if let Some(path) = opts.font_path.as_ref() {
        return Ok(Some(path.clone()));
    }
    if !cues_have_text(cues) {
        return Ok(None);
    }
    postkit::subtitle_raster::find_system_sans_font()
        .map(Some)
        .ok_or_else(|| NO_FONT_TO_EMBED.to_string())
}

/// Write the DCST for `cues` to `out` and stage the ancillary resources the
/// timed-text MXF wraps alongside it: the embedded font, then each distinct
/// bitmap. The `LoadFont` urn is the font resource's own id, so a player can
/// find the face inside the MXF.
fn write_dcst_styled(
    cues: &[StyledCue],
    lang: &str,
    fps: u32,
    opts: &SubtitleOptions,
    head_frames: u64,
    out: &Path,
) -> Result<Vec<(PathBuf, [u8; 16])>, String> {
    let mut resources: Vec<(PathBuf, [u8; 16])> = Vec::new();
    let font_ref = match font_to_embed(opts, cues)? {
        Some(font_path) => {
            let stage = out.with_extension(font_ext(&font_path));
            let (font_file, id) = build_embedded_font(&font_path, cues, opts.no_subset, &stage)?;
            resources.push((font_file, id));
            Some(id)
        }
        None => None,
    };
    assign_image_ids(cues, &mut resources);
    let xml = render_dcst_styled(cues, lang, fps, opts, font_ref, &resources, head_frames);
    std::fs::write(out, xml).map_err(|e| format!("write {}: {e}", out.display()))?;
    Ok(resources)
}

/// Reorder RTL cue text to visual order per [`RtlMode`]. Applied per run; a
/// single-run cue (the common RTL case) reorders exactly, multi-run styled RTL
/// reorders within each run.
fn apply_rtl(cues: &mut [StyledCue], mode: RtlMode) {
    for cue in cues {
        let active = match mode {
            RtlMode::Off => false,
            RtlMode::On => true,
            RtlMode::Auto => subtitle_formats::bidi::has_rtl(&cue.plain_text()),
        };
        if active {
            for run in &mut cue.runs {
                run.text = subtitle_formats::bidi::to_visual(&run.text);
            }
        }
    }
}

fn font_ext(font_path: &Path) -> String {
    let ext = font_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("ttf")
        .to_lowercase();
    format!("font.{ext}")
}

/// Subset (unless opted out) and stage the font to embed at `stage`, returning
/// the staged file and the asset id the DCST `LoadFont` references. Fails loud
/// if the result exceeds the SMPTE 640 KB font limit.
fn build_embedded_font(
    font_path: &Path,
    cues: &[StyledCue],
    no_subset: bool,
    stage: &Path,
) -> Result<(PathBuf, [u8; 16]), String> {
    let bytes =
        std::fs::read(font_path).map_err(|e| format!("read font {}: {e}", font_path.display()))?;
    let used: std::collections::BTreeSet<char> = cues
        .iter()
        .flat_map(|c| c.plain_text().chars().collect::<Vec<_>>())
        .collect();
    let out_bytes = if no_subset {
        bytes
    } else {
        postkit::font_subset::subset_font(&bytes, used.iter().copied())
            .map_err(|e| format!("font subset failed: {e}"))?
    };
    if out_bytes.len() > FONT_SIZE_LIMIT {
        return Err(format!(
            "embedded font is {} bytes, over the SMPTE 640 KB limit; subset it or use a smaller font",
            out_bytes.len()
        ));
    }
    std::fs::write(stage, &out_bytes)
        .map_err(|e| format!("write font {}: {e}", stage.display()))?;
    Ok((stage.to_path_buf(), *uuid::Uuid::new_v4().as_bytes()))
}

/// Assign an asset id to each distinct bitmap image referenced by the cues,
/// appending them to `resources` (same file reused keeps one id).
fn assign_image_ids(cues: &[StyledCue], resources: &mut Vec<(PathBuf, [u8; 16])>) {
    for cue in cues {
        if let Some(img) = cue.image.as_ref()
            && !resources.iter().any(|(p, _)| p == img)
        {
            resources.push((img.clone(), *uuid::Uuid::new_v4().as_bytes()));
        }
    }
}

/// A subtitle track prepared for reel splitting: styled cues (wrap + RTL already
/// applied) and, for font embedding, one shared font asset id reused by every
/// reel so the font is referenced identically across reels (dom#2533).
pub struct ReelSubtitlePlan {
    pub cues: Vec<StyledCue>,
    /// (staged font file, shared asset id) or None.
    pub font: Option<(PathBuf, [u8; 16])>,
}

/// Parse any supported subtitle format for reel splitting, applying the source
/// `trim`, wrap/RTL and staging a shared embedded font. A supplied SMPTE DCST XML
/// is rejected: its authored timing cannot be safely re-split across reels.
pub fn plan_reel_subtitles(
    input: &Path,
    trim: SourceTrim,
    fps: u32,
    opts: &SubtitleOptions,
    stage_dir: &Path,
) -> Result<ReelSubtitlePlan, String> {
    if detect_subtitle_kind(input)? == SubtitleInputKind::SmpteDcstPassthrough {
        return Err(
            "reel splitting cannot re-time a supplied SMPTE subtitle XML; supply SRT or a parsable format".into(),
        );
    }
    let mut cues = apply_source_trim(&load_styled_cues(input, fps)?, trim, fps);
    if let Some(cols) = opts.wrap_cols.filter(|c| *c > 0) {
        cues = cues
            .iter()
            .map(|c| subtitle_formats::wrap::wrap_styled(c, cols))
            .collect();
    }
    apply_rtl(&mut cues, opts.rtl);
    let font = match font_to_embed(opts, &cues)? {
        Some(font_path) => {
            let stage = stage_dir.join(format!(
                "subtitle_font_{}.{}",
                uuid::Uuid::new_v4(),
                font_ext(&font_path)
            ));
            Some(build_embedded_font(
                &font_path,
                &cues,
                opts.no_subset,
                &stage,
            )?)
        }
        None => None,
    };
    Ok(ReelSubtitlePlan { cues, font })
}

/// Styled cues starting in `[start_frame, end_frame)`, rebased to reel-local time
/// (0 = reel start) with runs/alignment/image kept. Cues overrunning the reel end
/// are truncated. Frame boundaries convert to ms at `fps`.
pub fn rebase_styled_for_reel(
    cues: &[StyledCue],
    start_frame: u64,
    end_frame: u64,
    fps: u32,
) -> Vec<StyledCue> {
    let fps64 = fps.max(1) as u64;
    let to_frame = |ms: u64| ms * fps64 / 1000;
    let to_ms = |f: u64| f * 1000 / fps64;
    cues.iter()
        .filter_map(|c| {
            let sf = to_frame(c.start_ms);
            if sf < start_frame || sf >= end_frame {
                return None;
            }
            let ef = to_frame(c.end_ms).min(end_frame);
            if ef <= sf {
                return None;
            }
            Some(StyledCue {
                start_ms: to_ms(sf - start_frame),
                end_ms: to_ms(ef - start_frame),
                runs: c.runs.clone(),
                align: c.align,
                valign: c.valign,
                vposition: c.vposition,
                image: c.image.clone(),
            })
        })
        .collect()
}

/// Render reel-local styled cues to a DCST, embedding a shared font (its asset id
/// in `font_id`) and returning the ancillary resources (font + any bitmap PNGs
/// used by this reel) to wrap alongside the XML.
pub fn render_reel_dcst(
    reel_cues: &[StyledCue],
    lang: &str,
    fps: u32,
    opts: &SubtitleOptions,
    font: Option<&(PathBuf, [u8; 16])>,
) -> (String, Vec<(PathBuf, [u8; 16])>) {
    let mut resources: Vec<(PathBuf, [u8; 16])> = Vec::new();
    if let Some((f, id)) = font {
        resources.push((f.clone(), *id));
    }
    assign_image_ids(reel_cues, &mut resources);
    let font_ref = font.map(|(_, id)| *id);
    // reel splitting rejects head padding, so no frame shift here
    let xml = render_dcst_styled(reel_cues, lang, fps, opts, font_ref, &resources, 0);
    (xml, resources)
}

/// Subtitle format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SubtitleFormat {
    #[default]
    SmpteXml,
    InteropXml,
    Srt,
}

/// Subtitle configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubtitleConfig {
    pub input_file: PathBuf,
    pub output_file: PathBuf,
    pub format: SubtitleFormat,
    pub language: String,
    pub font_size: u32,
    pub font_color: String,
    /// Edit rate for the SMPTE timecode / EditRate (frames per second).
    pub fps: u32,
    /// Bottom-line Vposition as a percentage from the bottom of the screen
    /// (Valign="bottom"). Zero falls back to the default 8%.
    pub vposition: f64,
}

/// Default bottom-line position: 8% up from the bottom of the screen.
const DEFAULT_VPOSITION: f64 = 8.0;
/// Vertical gap between stacked subtitle lines, in percent of screen height.
const LINE_SPACING: f64 = 7.0;

/// Vposition (percent from the bottom, Valign="bottom") for line `j` of a cue
/// with `line_count` lines: the last line sits at `base`, earlier lines stack
/// upward at LINE_SPACING each.
fn line_vposition(base: f64, line_count: usize, j: usize) -> f64 {
    base + (line_count - 1 - j) as f64 * LINE_SPACING
}

/// Import subtitles from SRT format and convert to TTML/XML for DCP packaging.
pub fn import_subtitles(config: &SubtitleConfig) -> i32 {
    let srt_content = match std::fs::read_to_string(&config.input_file) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to read SRT file: {e}");
            return -1;
        }
    };

    let entries = parse_srt(&srt_content);
    if entries.is_empty() {
        tracing::error!(
            "No subtitle entries found in {}",
            config.input_file.display()
        );
        return -1;
    }

    let lang = if config.language.is_empty() {
        "en"
    } else {
        &config.language
    };

    let font_size = if config.font_size == 0 {
        42
    } else {
        config.font_size
    };
    let font_color = if config.font_color.is_empty() {
        "FFFFFFFF"
    } else {
        &config.font_color
    };
    let fps = if config.fps == 0 { 24 } else { config.fps };
    let vposition = if config.vposition <= 0.0 {
        DEFAULT_VPOSITION
    } else {
        config.vposition
    };

    let xml = match config.format {
        SubtitleFormat::SmpteXml | SubtitleFormat::Srt => {
            generate_smpte_ttml(&entries, lang, font_size, font_color, fps, vposition)
        }
        SubtitleFormat::InteropXml => {
            generate_interop_xml(&entries, lang, font_size, font_color, vposition)
        }
    };

    match std::fs::write(&config.output_file, xml) {
        Ok(()) => {
            tracing::info!(
                "Converted {} subtitle entries to {}",
                entries.len(),
                config.output_file.display()
            );
            0
        }
        Err(e) => {
            tracing::error!("Failed to write subtitle XML: {e}");
            -1
        }
    }
}

/// Burn subtitles into video frames using ffmpeg drawtext/subtitles filter.
pub fn burnin_subtitles(input_video: &Path, subtitle_file: &Path, output_video: &Path) -> i32 {
    let sub_path = subtitle_file.to_string_lossy();
    let filter = format!("subtitles='{}'", sub_path.replace('\'', "\\'"));

    let result = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input_video)
        .arg("-vf")
        .arg(&filter)
        .arg("-c:a")
        .arg("copy")
        .arg(output_video)
        .output();

    match result {
        Ok(o) if o.status.success() => {
            tracing::info!("Burned subtitles into {}", output_video.display());
            0
        }
        Ok(o) => {
            tracing::error!(
                "ffmpeg subtitle burn-in failed: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            -1
        }
        Err(e) => {
            tracing::error!("Failed to run ffmpeg: {e}");
            -1
        }
    }
}

struct SrtEntry {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

/// Parse SRT via the shared postkit parser, keeping raw millisecond timing so
/// each format below can render it at its own timecode rate.
fn parse_srt(content: &str) -> Vec<SrtEntry> {
    postkit::subtitle_retime::parse_srt(content)
        .into_iter()
        .filter(|c| !c.text.is_empty())
        .map(|c| SrtEntry {
            start_ms: c.start_ms,
            end_ms: c.end_ms,
            text: c.text,
        })
        .collect()
}

/// A subtitle cue in whole picture frames, used by reel splitting to filter and
/// rebase cues onto per-reel timelines.
#[derive(Debug, Clone)]
pub struct SubCue {
    pub start_frame: u64,
    pub end_frame: u64,
    pub text: String,
}

/// Frame count to ST 428-7 timecode "HH:MM:SS:FF" at `fps` frames/sec.
fn frames_to_dcst(total_frames: u64, fps: u32) -> String {
    let fps = fps.max(1) as u64;
    let frames = total_frames % fps;
    let secs = total_frames / fps;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60,
        frames
    )
}

/// Parse an SRT file into frame-based cues at `fps` (for reel splitting).
pub fn parse_srt_frames(path: &Path, fps: u32) -> Result<Vec<SubCue>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let fps64 = fps.max(1) as u64;
    Ok(parse_srt(&content)
        .into_iter()
        .map(|e| SubCue {
            start_frame: e.start_ms * fps64 / 1000,
            end_frame: e.end_ms * fps64 / 1000,
            text: e.text,
        })
        .collect())
}

/// Convert an SRT file to a DCST XML, shifting every cue later by `head_frames`.
/// Head padding moves the program start, so supplied SRT cues must slide by the
/// same offset to stay aligned with the picture. `head_frames == 0` is a plain
/// conversion. Returns the ancillary resources the timed-text MXF wraps
/// alongside the XML.
pub fn srt_to_shifted_dcst(
    srt: &Path,
    head_frames: u64,
    lang: &str,
    fps: u32,
    opts: &SubtitleOptions,
    out: &Path,
) -> Result<Vec<(PathBuf, [u8; 16])>, String> {
    let cues: Vec<SubCue> = parse_srt_frames(srt, fps)?
        .into_iter()
        .map(|c| SubCue {
            start_frame: c.start_frame + head_frames,
            end_frame: c.end_frame + head_frames,
            text: c.text,
        })
        .collect();
    write_dcst_frames(&cues, lang, fps, opts, out)
}

/// Write a reel's DCST from frame-based cues (already rebased to reel-local 0),
/// returning the ancillary resources the timed-text MXF wraps alongside it.
pub fn write_dcst_frames(
    cues: &[SubCue],
    lang: &str,
    fps: u32,
    opts: &SubtitleOptions,
    out: &Path,
) -> Result<Vec<(PathBuf, [u8; 16])>, String> {
    let styled = styled_from_frames(cues, fps);
    write_dcst_styled(&styled, lang, fps, opts, 0, out)
}

/// The cues a reel of a subtitled composition carries when none of its own fall
/// inside it: one space at the reel start, one second long. ST 428-7 has no
/// document without a cue in it, and a composition with subtitles needs a track
/// on every reel, so this is the placeholder DCP-o-matic writes for such a reel.
pub fn placeholder_cues(fps: u32) -> Vec<SubCue> {
    let rate = fps.max(1) as u64;
    vec![SubCue {
        start_frame: 0,
        end_frame: rate * PLACEHOLDER_CUE_SECONDS,
        text: PLACEHOLDER_CUE_TEXT.to_string(),
    }]
}

/// [`placeholder_cues`] for the callers that work in styled cues.
pub fn placeholder_styled_cues(fps: u32) -> Vec<StyledCue> {
    styled_from_frames(&placeholder_cues(fps), fps)
}

/// Frame-based cues as styled cues. The millisecond time rounds up, so the
/// renderer's own truncating conversion back to frames at the same rate lands on
/// the frame the cue came from.
fn styled_from_frames(cues: &[SubCue], fps: u32) -> Vec<StyledCue> {
    let fps64 = fps.max(1) as u64;
    let to_ms = |frames: u64| (frames * 1000).div_ceil(fps64);
    cues.iter()
        .map(|c| {
            StyledCue::text(
                to_ms(c.start_frame),
                to_ms(c.end_frame),
                vec![StyledRun::plain(&c.text)],
            )
        })
        .collect()
}

/// Milliseconds to Interop "HH:MM:SS.mmm".
fn ms_to_interop(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let millis = ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{millis:03}")
}

/// Render SRT entries to a loose ST 428-7 DCST XML. This is the conversion
/// command's output, which is a document on its own: nothing wraps it, so it
/// carries no embedded font and its `Font` names none.
fn generate_smpte_ttml(
    entries: &[SrtEntry],
    lang: &str,
    font_size: u32,
    font_color: &str,
    fps: u32,
    vposition: f64,
) -> String {
    let cues: Vec<StyledCue> = entries
        .iter()
        .map(|e| StyledCue::text(e.start_ms, e.end_ms, vec![StyledRun::plain(&e.text)]))
        .collect();
    let opts = SubtitleOptions {
        vposition: Some(vposition),
        appearance: TimedTextAppearance {
            font_size: Some(font_size),
            colour: Some(font_color.to_string()),
            ..TimedTextAppearance::default()
        },
        ..SubtitleOptions::default()
    };
    render_dcst_styled(&cues, lang, fps, &opts, None, &[], 0)
}

fn generate_interop_xml(
    entries: &[SrtEntry],
    lang: &str,
    font_size: u32,
    font_color: &str,
    vposition: f64,
) -> String {
    let sub_id = uuid::Uuid::new_v4();
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<DCSubtitle Version=\"1.0\">\n");
    xml.push_str(&format!("  <SubtitleID>{sub_id}</SubtitleID>\n"));
    xml.push_str("  <MovieTitle>Subtitles</MovieTitle>\n");
    xml.push_str("  <ReelNumber>1</ReelNumber>\n");
    xml.push_str(&format!("  <Language>{lang}</Language>\n"));
    xml.push_str(&format!(
        "  <Font Id=\"font1\" Color=\"{font_color}\" Size=\"{font_size}\" Effect=\"shadow\" EffectColor=\"000000\">\n"
    ));

    for (i, entry) in entries.iter().enumerate() {
        xml.push_str(&format!(
            "    <Subtitle SpotNumber=\"{}\" TimeIn=\"{}\" TimeOut=\"{}\" FadeUpTime=\"2\" FadeDownTime=\"2\">\n",
            i + 1,
            ms_to_interop(entry.start_ms),
            ms_to_interop(entry.end_ms),
        ));
        let lines: Vec<&str> = entry.text.split('\n').collect();
        for (j, line) in lines.iter().enumerate() {
            let vpos = line_vposition(vposition, lines.len(), j);
            xml.push_str(&format!(
                "      <Text Vposition=\"{vpos:.1}\" VAlign=\"bottom\" HAlign=\"center\">{}</Text>\n",
                postkit::packaging::escape_xml(line)
            ));
        }
        xml.push_str("    </Subtitle>\n");
    }

    xml.push_str("  </Font>\n");
    xml.push_str("</DCSubtitle>\n");
    xml
}

/// High-level convenience: convert an SRT file to DCP SMPTE XML. `vposition` is
/// the bottom-line percentage from the bottom of the screen (0 uses the default).
pub fn convert_srt_to_dcp_xml(
    input: &Path,
    output: &Path,
    language: &str,
    fps: u32,
    vposition: f64,
) -> Result<(), String> {
    let config = SubtitleConfig {
        input_file: input.to_path_buf(),
        output_file: output.to_path_buf(),
        format: SubtitleFormat::SmpteXml,
        language: language.to_string(),
        font_size: 42,
        font_color: "FFFFFFFF".to_string(),
        fps,
        vposition,
    };
    let code = import_subtitles(&config);
    if code == 0 {
        Ok(())
    } else {
        Err("Subtitle conversion failed".to_string())
    }
}

/// ARGB hex for a SMPTE subtitle Color/EffectColor (alpha first).
fn argb(c: Rgba) -> String {
    format!("{:02X}{:02X}{:02X}{:02X}", c.a, c.r, c.g, c.b)
}

fn halign_str(h: HAlign) -> &'static str {
    match h {
        HAlign::Left => "left",
        HAlign::Center => "center",
        HAlign::Right => "right",
    }
}

fn valign_str(v: VAlign) -> &'static str {
    match v {
        VAlign::Top => "top",
        VAlign::Middle => "center",
        VAlign::Bottom => "bottom",
    }
}

/// Default Vposition for an anchor: centred cues sit at 0, top/bottom 8% in.
fn default_base(valign: &str) -> f64 {
    if valign == "center" {
        0.0
    } else {
        DEFAULT_VPOSITION
    }
}

/// Resolved placement for a cue: (halign, valign, base Vposition).
fn placement(cue: &StyledCue, opts: &SubtitleOptions) -> (&'static str, &'static str, f64) {
    let halign = opts
        .halign
        .as_deref()
        .map(norm_halign)
        .or_else(|| cue.align.map(halign_str))
        .unwrap_or("center");
    let valign = opts
        .valign
        .as_deref()
        .map(norm_valign)
        .or_else(|| cue.valign.map(valign_str))
        .unwrap_or("bottom");
    let base = opts.vposition.unwrap_or_else(|| {
        // images carry a real SMPTE-style Vposition; text-cue vposition is not reliable
        if cue.image.is_some() {
            cue.vposition
                .map(|v| v as f64)
                .unwrap_or_else(|| default_base(valign))
        } else {
            default_base(valign)
        }
    });
    (halign, valign, base)
}

fn norm_halign(s: &str) -> &'static str {
    match s.to_lowercase().as_str() {
        "left" => "left",
        "right" => "right",
        _ => "center",
    }
}

fn norm_valign(s: &str) -> &'static str {
    match s.to_lowercase().as_str() {
        "top" => "top",
        "center" | "centre" | "middle" => "center",
        _ => "bottom",
    }
}

/// Vposition for line `j` of an `n`-line cue anchored at `valign`, base `base`.
/// Bottom stacks upward (last line at base), top grows downward, centre spreads
/// around the base.
fn stacked_vpos(valign: &str, base: f64, n: usize, j: usize) -> f64 {
    match valign {
        "top" => base + j as f64 * LINE_SPACING,
        "center" => base + ((n - 1) as f64 / 2.0 - j as f64) * LINE_SPACING,
        _ => base + (n - 1 - j) as f64 * LINE_SPACING,
    }
}

/// Split a cue's runs into lines (each a run list), breaking at '\n'.
fn cue_lines(cue: &StyledCue) -> Vec<Vec<StyledRun>> {
    let mut lines: Vec<Vec<StyledRun>> = vec![Vec::new()];
    for run in &cue.runs {
        let parts: Vec<&str> = run.text.split('\n').collect();
        for (k, part) in parts.iter().enumerate() {
            if k > 0 {
                lines.push(Vec::new());
            }
            if !part.is_empty() {
                lines.last_mut().unwrap().push(StyledRun {
                    text: part.to_string(),
                    ..run.clone()
                });
            }
        }
    }
    lines
}

/// Render one line's runs to DCST Text content, using inline `<dcst:Font>` spans
/// only where a run carries styling.
fn render_line(runs: &[StyledRun]) -> String {
    let plain = |r: &StyledRun| !r.italic && !r.bold && !r.underline && r.color.is_none();
    let mut s = String::new();
    for r in runs {
        let esc = postkit::packaging::escape_xml(&r.text);
        if plain(r) {
            s.push_str(&esc);
        } else {
            let mut attrs = String::new();
            if r.italic {
                attrs.push_str(" Italic=\"yes\"");
            }
            if r.bold {
                attrs.push_str(" Weight=\"bold\"");
            }
            if r.underline {
                attrs.push_str(" Underline=\"yes\"");
            }
            if let Some(c) = r.color {
                attrs.push_str(&format!(" Color=\"{}\"", argb(c)));
            }
            s.push_str(&format!("<dcst:Font{attrs}>{esc}</dcst:Font>"));
        }
    }
    s
}

/// Render styled cues to a ST 428-7 DCST XML honouring placement, styling, RTL
/// and 3D depth options, plus an embedded-font `LoadFont` and bitmap Image refs.
fn render_dcst_styled(
    cues: &[StyledCue],
    lang: &str,
    fps: u32,
    opts: &SubtitleOptions,
    font_ref: Option<[u8; 16]>,
    resources: &[(PathBuf, [u8; 16])],
    head_frames: u64,
) -> String {
    let sub_id = uuid::Uuid::new_v4();
    let (fade_up, fade_down) = opts.appearance.fades(fps);
    let z_attr = opts
        .zposition
        .map(|z| format!(" Zposition=\"{z}\""))
        .unwrap_or_default();
    let fps64 = fps.max(1) as u64;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(&format!(
        "<dcst:SubtitleReel xmlns:dcst=\"{DCST_NAMESPACE}\">\n"
    ));
    xml.push_str(&format!("  <dcst:Id>urn:uuid:{sub_id}</dcst:Id>\n"));
    xml.push_str("  <dcst:ContentTitleText>Subtitles</dcst:ContentTitleText>\n");
    xml.push_str("  <dcst:AnnotationText>Subtitles</dcst:AnnotationText>\n");
    xml.push_str(&format!(
        "  <dcst:IssueDate>{}</dcst:IssueDate>\n",
        chrono::Utc::now().format(DCST_ISSUE_DATE_FORMAT)
    ));
    xml.push_str("  <dcst:ReelNumber>1</dcst:ReelNumber>\n");
    xml.push_str(&format!("  <dcst:Language>{lang}</dcst:Language>\n"));
    xml.push_str(&format!("  <dcst:EditRate>{fps} 1</dcst:EditRate>\n"));
    xml.push_str(&format!("  <dcst:TimeCodeRate>{fps}</dcst:TimeCodeRate>\n"));
    xml.push_str(&format!(
        "  <dcst:StartTime>{DCST_START_TIME}</dcst:StartTime>\n"
    ));
    if let Some(id) = font_ref {
        xml.push_str(&format!(
            "  <dcst:LoadFont ID=\"{SUBTITLE_FONT_ID}\">urn:uuid:{}</dcst:LoadFont>\n",
            uuid::Uuid::from_bytes(id).hyphenated()
        ));
    }
    let font_id_attribute = match font_ref {
        Some(_) => format!(" ID=\"{SUBTITLE_FONT_ID}\""),
        None => String::new(),
    };
    xml.push_str("  <dcst:SubtitleList>\n");
    xml.push_str(&format!(
        "    <dcst:Font{font_id_attribute} {}>\n",
        opts.appearance.font_attributes()
    ));

    for (i, cue) in cues.iter().enumerate() {
        let tin = frames_to_dcst(cue.start_ms * fps64 / 1000 + head_frames, fps);
        let tout = frames_to_dcst(cue.end_ms * fps64 / 1000 + head_frames, fps);
        xml.push_str(&format!(
            "      <dcst:Subtitle SpotNumber=\"{}\" TimeIn=\"{tin}\" TimeOut=\"{tout}\" FadeUpTime=\"{fade_up}\" FadeDownTime=\"{fade_down}\">\n",
            i + 1,
        ));
        let (halign, valign, base) = placement(cue, opts);
        if let Some(img) = cue.image.as_ref() {
            let id = resources
                .iter()
                .find(|(p, _)| p == img)
                .map(|(_, id)| uuid::Uuid::from_bytes(*id).hyphenated().to_string())
                .unwrap_or_default();
            xml.push_str(&format!(
                "        <dcst:Image Vposition=\"{base:.1}\" Valign=\"{valign}\" Halign=\"{halign}\"{z_attr}>urn:uuid:{id}</dcst:Image>\n"
            ));
        } else {
            let lines: Vec<Vec<StyledRun>> = cue_lines(cue)
                .into_iter()
                .filter(|l| l.iter().any(|r| !r.text.is_empty()))
                .collect();
            let n = lines.len().max(1);
            for (j, line) in lines.iter().enumerate() {
                let vpos = stacked_vpos(valign, base, n, j);
                xml.push_str(&format!(
                    "        <dcst:Text Vposition=\"{vpos:.1}\" Valign=\"{valign}\" Halign=\"{halign}\"{z_attr}>{}</dcst:Text>\n",
                    render_line(line)
                ));
            }
        }
        xml.push_str("      </dcst:Subtitle>\n");
    }

    xml.push_str("    </dcst:Font>\n");
    xml.push_str("  </dcst:SubtitleList>\n");
    xml.push_str("</dcst:SubtitleReel>\n");
    xml
}

#[cfg(test)]
mod trim_tests {
    use super::*;

    const FPS: u32 = 24;

    fn cue(start_ms: u64, end_ms: u64, text: &str) -> StyledCue {
        StyledCue::text(start_ms, end_ms, vec![StyledRun::plain(text)])
    }

    // the picture moved, so the cues have to move with it: this is what makes
    // `--trim-start` keep subtitles in sync instead of sliding them a second late
    #[test]
    fn cues_move_with_the_picture_and_the_outsiders_go() {
        // keep source frames 24..72, i.e. 1000ms..3000ms at 24 fps
        let trim = SourceTrim {
            start_frames: 24,
            kept_frames: 48,
        };
        let cues = vec![
            cue(0, 500, "before"),
            cue(500, 1500, "straddles the head"),
            cue(1600, 2000, "inside"),
            cue(2500, 4000, "straddles the tail"),
            cue(3500, 4000, "after"),
        ];
        let moved = apply_source_trim(&cues, trim, FPS);

        let texts: Vec<&str> = moved.iter().map(|c| c.runs[0].text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["straddles the head", "inside", "straddles the tail"],
            "cues wholly outside the kept window are dropped"
        );
        assert_eq!(
            (moved[0].start_ms, moved[0].end_ms),
            (0, 500),
            "clamped to the head"
        );
        assert_eq!(
            (moved[1].start_ms, moved[1].end_ms),
            (600, 1000),
            "an inside cue slides back by the head trim"
        );
        assert_eq!(
            (moved[2].start_ms, moved[2].end_ms),
            (1500, 2000),
            "clamped to the tail"
        );
    }

    #[test]
    fn no_trim_leaves_every_cue_where_it_was() {
        let cues = vec![cue(0, 500, "a"), cue(9_000, 9_500, "b")];
        let same = apply_source_trim(&cues, SourceTrim::default(), FPS);
        assert_eq!(same.len(), 2);
        assert_eq!((same[1].start_ms, same[1].end_ms), (9_000, 9_500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reel of a subtitled composition that no cue falls into still carries a
    /// subtitle asset, and what it carries is one space from the reel start.
    #[test]
    fn a_reel_with_no_cues_writes_the_placeholder_cue() {
        let placeholder = placeholder_styled_cues(24);
        assert!(
            cues_have_text(&placeholder),
            "a space is text, so the reel embeds a font like any other"
        );

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("subtitle.xml");
        let opts = SubtitleOptions {
            font_path: Some(fixture_font()),
            ..Default::default()
        };
        let resources = write_dcst_styled(&placeholder, "de", 24, &opts, 0, &out).unwrap();
        assert_eq!(resources.len(), 1, "the font is embedded: {resources:?}");

        let xml = std::fs::read_to_string(&out).unwrap();
        assert!(
            xml.contains("TimeIn=\"00:00:00:00\" TimeOut=\"00:00:01:00\""),
            "one second from the reel start: {xml}"
        );
        assert!(
            xml.contains(
                "<dcst:Text Vposition=\"8.0\" Valign=\"bottom\" Halign=\"center\"> </dcst:Text>"
            ),
            "the space survives serialisation: {xml}"
        );
        assert_eq!(
            xml.matches("<dcst:Subtitle ").count(),
            1,
            "one cue and no more: {xml}"
        );
        assert!(
            xml.contains(&format!("<dcst:LoadFont ID=\"{SUBTITLE_FONT_ID}\">")),
            "a font is loaded like any text cue: {xml}"
        );
        for element in [
            "dcst:Id",
            "dcst:ContentTitleText",
            "dcst:IssueDate",
            "dcst:ReelNumber",
            "dcst:Language",
            "dcst:EditRate",
            "dcst:TimeCodeRate",
            "dcst:StartTime",
        ] {
            assert!(xml.contains(&format!("<{element}>")), "{element} in {xml}");
        }
        assert!(xml.contains("<dcst:Language>de</dcst:Language>"), "{xml}");
    }

    #[test]
    fn two_line_cue_anchors_at_bottom() {
        let entries = [SrtEntry {
            start_ms: 1000,
            end_ms: 4000,
            text: "line one\nline two".to_string(),
        }];
        let xml = generate_smpte_ttml(&entries, "en", 42, "FFFFFFFF", 24, DEFAULT_VPOSITION);
        // last line at 8%, the line above it at 15%, both anchored to the bottom
        assert!(
            xml.contains("Vposition=\"15.0\" Valign=\"bottom\""),
            "top line at 15%: {xml}"
        );
        assert!(
            xml.contains("Vposition=\"8.0\" Valign=\"bottom\""),
            "bottom line at 8%: {xml}"
        );
        assert!(
            !xml.contains("Vposition=\"85.0\""),
            "old top-anchored value gone"
        );
    }

    #[test]
    fn interop_two_line_cue_anchors_at_bottom() {
        let entries = [SrtEntry {
            start_ms: 1000,
            end_ms: 4000,
            text: "line one\nline two".to_string(),
        }];
        let xml = generate_interop_xml(&entries, "en", 42, "FFFFFFFF", DEFAULT_VPOSITION);
        assert!(
            xml.contains("Vposition=\"15.0\" VAlign=\"bottom\""),
            "{xml}"
        );
        assert!(xml.contains("Vposition=\"8.0\" VAlign=\"bottom\""), "{xml}");
    }

    #[test]
    fn custom_vposition_shifts_the_block() {
        let entries = [SrtEntry {
            start_ms: 0,
            end_ms: 1000,
            text: "solo".to_string(),
        }];
        let xml = generate_smpte_ttml(&entries, "en", 42, "FFFFFFFF", 24, 12.0);
        assert!(xml.contains("Vposition=\"12.0\""), "{xml}");
    }

    fn write(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    /// A font in the repo, so a test that only needs some font does not depend on
    /// what faces the machine running it carries.
    fn fixture_font() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/LiberationSans-Regular.ttf")
    }

    /// Convert `input` and hand back the DCST. A text track has to embed a font,
    /// so one is named here unless the test named its own.
    fn render(input: &std::path::Path, opts: &SubtitleOptions) -> String {
        let opts = SubtitleOptions {
            font_path: opts.font_path.clone().or_else(|| Some(fixture_font())),
            ..opts.clone()
        };
        let out = input.with_extension("out.xml");
        let prepared =
            prepare_subtitle_track(input, CueTiming::default(), "en", 24, &opts, &out).unwrap();
        let xml = std::fs::read_to_string(&prepared.dcst_path).unwrap();
        std::fs::remove_file(&out).ok();
        xml
    }

    const SRT2: &str = "1\n00:00:01,000 --> 00:00:04,000\nline one\nline two\n";

    #[test]
    fn styled_srt_default_matches_centered_bottom() {
        let dir = tempfile::tempdir().unwrap();
        let srt = write(dir.path(), "in.srt", SRT2);
        let xml = render(&srt, &SubtitleOptions::default());
        assert!(
            xml.contains("Vposition=\"15.0\" Valign=\"bottom\" Halign=\"center\""),
            "{xml}"
        );
        assert!(
            xml.contains("Vposition=\"8.0\" Valign=\"bottom\" Halign=\"center\""),
            "{xml}"
        );
    }

    #[test]
    fn the_default_font_line_is_what_the_track_has_always_carried() {
        let dir = tempfile::tempdir().unwrap();
        let srt = write(dir.path(), "in.srt", SRT2);
        let xml = render(&srt, &SubtitleOptions::default());
        assert!(
            xml.contains(
                "<dcst:Font ID=\"font1\" Color=\"FFFFFFFF\" Size=\"42\" Effect=\"shadow\" EffectColor=\"FF000000\">"
            ),
            "{xml}"
        );
        assert!(
            xml.contains("FadeUpTime=\"00:00:00:02\" FadeDownTime=\"00:00:00:02\""),
            "{xml}"
        );
    }

    #[test]
    fn a_named_appearance_reaches_the_font_line_and_the_fades() {
        let dir = tempfile::tempdir().unwrap();
        let srt = write(dir.path(), "in.srt", SRT2);
        let opts = SubtitleOptions {
            appearance: TimedTextAppearance::from_flags(
                Some(50),
                Some("FFFF00"),
                Some("outline"),
                Some("112233AA"),
                Some(200),
                Some(200),
            )
            .unwrap(),
            ..Default::default()
        };
        let xml = render(&srt, &opts);
        assert!(
            xml.contains(
                "<dcst:Font ID=\"font1\" Color=\"FFFFFF00\" Size=\"50\" Effect=\"border\" EffectColor=\"AA112233\">"
            ),
            "{xml}"
        );
        // 200 ms at 24 fps is 4.8 frames, so 5
        assert!(
            xml.contains("FadeUpTime=\"00:00:00:05\" FadeDownTime=\"00:00:00:05\""),
            "{xml}"
        );
    }

    #[test]
    fn a_bad_appearance_value_is_refused_under_its_flag() {
        let colour = TimedTextAppearance::from_flags(None, Some("nope"), None, None, None, None)
            .unwrap_err();
        assert!(colour.contains("--subtitle-colour"), "got: {colour}");
        let effect = TimedTextAppearance::from_flags(None, None, Some("glow"), None, None, None)
            .unwrap_err();
        assert!(effect.contains("--subtitle-effect"), "got: {effect}");
    }

    #[test]
    fn top_valign_grows_downward() {
        let dir = tempfile::tempdir().unwrap();
        let srt = write(dir.path(), "in.srt", SRT2);
        let opts = SubtitleOptions {
            valign: Some("top".into()),
            ..Default::default()
        };
        let xml = render(&srt, &opts);
        // first line at 8%, the next below it at 15%, both top-anchored
        assert!(xml.contains("Vposition=\"8.0\" Valign=\"top\""), "{xml}");
        assert!(xml.contains("Vposition=\"15.0\" Valign=\"top\""), "{xml}");
    }

    #[test]
    fn halign_and_zposition_are_emitted() {
        let dir = tempfile::tempdir().unwrap();
        let srt = write(
            dir.path(),
            "in.srt",
            "1\n00:00:01,000 --> 00:00:02,000\nhi\n",
        );
        let opts = SubtitleOptions {
            halign: Some("left".into()),
            zposition: Some(2.5),
            ..Default::default()
        };
        let xml = render(&srt, &opts);
        assert!(xml.contains("Halign=\"left\""), "{xml}");
        assert!(xml.contains("Zposition=\"2.5\""), "{xml}");
    }

    #[test]
    fn rtl_auto_reorders_hebrew_to_visual() {
        let dir = tempfile::tempdir().unwrap();
        // logical alef-bet-gimel should render gimel-bet-alef
        let srt = write(
            dir.path(),
            "he.srt",
            "1\n00:00:01,000 --> 00:00:02,000\n\u{05d0}\u{05d1}\u{05d2}\n",
        );
        let xml = render(&srt, &SubtitleOptions::default());
        assert!(
            xml.contains("\u{05d2}\u{05d1}\u{05d0}"),
            "visual order: {xml}"
        );
    }

    #[test]
    fn wrap_splits_long_lines() {
        let dir = tempfile::tempdir().unwrap();
        let srt = write(
            dir.path(),
            "w.srt",
            "1\n00:00:01,000 --> 00:00:02,000\naaa bbb ccc ddd eee\n",
        );
        let opts = SubtitleOptions {
            wrap_cols: Some(7),
            ..Default::default()
        };
        let xml = render(&srt, &opts);
        // wrapped into multiple Text lines, none over 7 chars
        let texts: Vec<&str> = xml.matches("<dcst:Text").collect();
        assert!(texts.len() >= 3, "wrapped into >=3 lines: {xml}");
    }

    const ASS: &str = "[V4+ Styles]\nFormat: Name, Italic, Alignment\nStyle: Def,0,2\n[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Def,plain {\\i1}slanted{\\i0}\n";

    #[test]
    fn ass_italic_run_becomes_inline_font() {
        let dir = tempfile::tempdir().unwrap();
        let ass = write(dir.path(), "in.ass", ASS);
        let xml = render(&ass, &SubtitleOptions::default());
        assert!(
            xml.contains("<dcst:Font Italic=\"yes\">slanted</dcst:Font>"),
            "inline italic run: {xml}"
        );
        assert!(xml.contains(">plain"), "plain run stays plain: {xml}");
    }

    #[test]
    fn interop_png_emits_image_ref_and_resource() {
        let dir = tempfile::tempdir().unwrap();
        let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0, 0, 0, 13]);
        std::fs::write(dir.path().join("s1.png"), png).unwrap();
        let xml_in = write(
            dir.path(),
            "subs.xml",
            "<DCSubtitle Version=\"1.0\"><Subtitle TimeIn=\"00:00:01:00\" TimeOut=\"00:00:04:00\"><Image VAlign=\"bottom\" HAlign=\"center\" VPosition=\"8\">s1.png</Image></Subtitle></DCSubtitle>",
        );
        let out = dir.path().join("out.xml");
        let prepared = prepare_subtitle_track(
            &xml_in,
            CueTiming::default(),
            "en",
            24,
            &SubtitleOptions::default(),
            &out,
        )
        .unwrap();
        let xml = std::fs::read_to_string(&prepared.dcst_path).unwrap();
        assert_eq!(prepared.resources.len(), 1, "one embedded png");
        let id = uuid::Uuid::from_bytes(prepared.resources[0].1)
            .hyphenated()
            .to_string();
        assert!(
            xml.contains(&format!("<dcst:Image Vposition=\"8.0\" Valign=\"bottom\" Halign=\"center\">urn:uuid:{id}</dcst:Image>")),
            "image references embedded resource: {xml}"
        );
    }

    #[test]
    fn reel_font_shares_one_asset_id_across_reels() {
        // dom#2533: a font used by cues in several reels is referenced by the
        // same asset id in each reel's subtitle XML.
        let font = (PathBuf::from("/x/f.ttf"), *uuid::Uuid::new_v4().as_bytes());
        let id = uuid::Uuid::from_bytes(font.1).hyphenated().to_string();
        let c1 = vec![StyledCue::text(0, 1000, vec![StyledRun::plain("a")])];
        let c2 = vec![StyledCue::text(0, 1000, vec![StyledRun::plain("b")])];
        let (x1, r1) = render_reel_dcst(&c1, "en", 24, &SubtitleOptions::default(), Some(&font));
        let (x2, _) = render_reel_dcst(&c2, "en", 24, &SubtitleOptions::default(), Some(&font));
        assert!(
            x1.contains(&format!("<dcst:LoadFont ID=\"font1\">urn:uuid:{id}")),
            "{x1}"
        );
        assert!(x2.contains(&format!("urn:uuid:{id}")), "{x2}");
        assert_eq!(r1[0].1, font.1, "resource keeps the shared id");
    }

    /// The four document-level rules dcpdoctor reads off a packaged DCST: one
    /// namespace on the root, a zero StartTime, an IssueDate with no timezone
    /// suffix, and a LoadFont introducing the id the Font names.
    #[test]
    fn the_packaged_document_carries_what_the_conformance_rules_read() {
        let dir = tempfile::tempdir().unwrap();
        let srt = write(dir.path(), "in.srt", SRT2);
        let xml = render(&srt, &SubtitleOptions::default());

        assert_eq!(
            xml.matches("xmlns").count(),
            1,
            "the root declares one namespace: {xml}"
        );
        assert!(
            xml.contains("<dcst:StartTime>00:00:00:00</dcst:StartTime>"),
            "{xml}"
        );
        let issue_date = xml
            .split("<dcst:IssueDate>")
            .nth(1)
            .and_then(|t| t.split("</dcst:IssueDate>").next())
            .expect("an IssueDate");
        assert_eq!(issue_date.len(), 19, "yyyy-mm-ddThh:mm:ss: {issue_date}");
        assert!(
            !issue_date.contains('+') && !issue_date.contains('.'),
            "no timezone and no fraction: {issue_date}"
        );
        assert!(
            xml.contains("<dcst:LoadFont ID=\"font1\">urn:uuid:"),
            "a track with no --subtitle-font still loads one: {xml}"
        );
        assert!(
            xml.find("<dcst:StartTime>").unwrap() < xml.find("<dcst:LoadFont").unwrap(),
            "StartTime precedes LoadFont: {xml}"
        );
    }

    /// The embedded font is the machine's, subset to the cues, and it is the
    /// resource the LoadFont urn names.
    #[test]
    fn a_track_with_no_named_font_embeds_a_system_one() {
        assert!(
            postkit::subtitle_raster::find_system_sans_font().is_some(),
            "this machine carries no system sans font"
        );
        let dir = tempfile::tempdir().unwrap();
        let srt = write(dir.path(), "in.srt", SRT2);
        let out = dir.path().join("out.xml");
        let prepared = prepare_subtitle_track(
            &srt,
            CueTiming::default(),
            "en",
            24,
            &SubtitleOptions::default(),
            &out,
        )
        .unwrap();
        assert_eq!(prepared.resources.len(), 1, "the font is the one resource");
        let (font_file, id) = &prepared.resources[0];
        assert!(font_file.exists(), "the staged font is on disk");
        let xml = std::fs::read_to_string(&prepared.dcst_path).unwrap();
        assert!(
            xml.contains(&format!(
                "<dcst:LoadFont ID=\"font1\">urn:uuid:{}</dcst:LoadFont>",
                uuid::Uuid::from_bytes(*id).hyphenated()
            )),
            "{xml}"
        );
    }

    /// A `Font` may only name a face a `LoadFont` introduced, and the loose
    /// conversion command embeds nothing, so its `Font` names nothing either.
    #[test]
    fn a_document_with_no_load_font_leaves_the_font_unnamed() {
        let entries = [SrtEntry {
            start_ms: 0,
            end_ms: 1000,
            text: "solo".to_string(),
        }];
        let xml = generate_smpte_ttml(&entries, "en", 42, "FFFFFFFF", 24, DEFAULT_VPOSITION);
        assert!(!xml.contains("LoadFont"), "{xml}");
        assert!(
            xml.contains("<dcst:Font Color=\"FFFFFFFF\""),
            "the Font carries styling and no ID: {xml}"
        );
    }

    /// The frame-based path shares the styled writer, which times cues in
    /// milliseconds. The conversion has to land back on the frame it came from,
    /// or every reel-split cue drifts a frame early.
    #[test]
    fn frame_cues_keep_their_frame_after_the_round_trip_through_milliseconds() {
        const FPS: u32 = 24;
        let cues: Vec<SubCue> = (0..FPS as u64 * 3)
            .map(|f| SubCue {
                start_frame: f,
                end_frame: f + 1,
                text: format!("cue {f}"),
            })
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("frames.xml");
        let opts = SubtitleOptions {
            font_path: Some(fixture_font()),
            ..Default::default()
        };
        write_dcst_styled(&styled_from_frames(&cues, FPS), "en", FPS, &opts, 0, &out).unwrap();
        let xml = std::fs::read_to_string(&out).unwrap();
        for cue in &cues {
            let time_in = frames_to_dcst(cue.start_frame, FPS);
            let time_out = frames_to_dcst(cue.end_frame, FPS);
            assert!(
                xml.contains(&format!("TimeIn=\"{time_in}\" TimeOut=\"{time_out}\"")),
                "frame {} kept its timecode: {xml}",
                cue.start_frame
            );
        }
    }

    #[test]
    fn detect_kind_by_extension_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let srt = write(dir.path(), "a.srt", "x");
        assert_eq!(detect_subtitle_kind(&srt).unwrap(), SubtitleInputKind::Srt);
        let smpte = write(dir.path(), "b.xml", "<dcst:SubtitleReel/>");
        assert_eq!(
            detect_subtitle_kind(&smpte).unwrap(),
            SubtitleInputKind::SmpteDcstPassthrough
        );
        let interop = write(
            dir.path(),
            "c.xml",
            "<DCSubtitle><Subtitle><Image>x.png</Image></Subtitle></DCSubtitle>",
        );
        assert_eq!(
            detect_subtitle_kind(&interop).unwrap(),
            SubtitleInputKind::InteropPng
        );
    }
}
