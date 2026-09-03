use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};

/// Default encode target in Mbit/s. Under DCI's 250 on purpose: rate allocation
/// lands a frame either side of the target, so 250 fails the peak bitrate check.
const DEFAULT_BANDWIDTH_MBPS: u32 = 230;

/// Panel frame rate keys and the CPL edit rate numerator each stands for. Every
/// rate the panel offers is integer, so the denominator is always 1.
const FRAME_RATES: [(&str, u32); 9] = [
    ("24", 24),
    ("25", 25),
    ("30", 30),
    ("48", 48),
    ("50", 50),
    ("60", 60),
    ("96", 96),
    ("100", 100),
    ("120", 120),
];
const FRAME_RATE_DENOMINATOR: u32 = 1;
const DEFAULT_FRAME_RATE: (&str, u32) = FRAME_RATES[0];

const DEFAULT_STANDARD: &str = "smpte";
const DEFAULT_RESOLUTION: &str = "2k-full";
const DEFAULT_COLOUR: &str = "xyz";
const DEFAULT_CONTENT_KIND: &str = "feature";
const DEFAULT_CHANNELS: &str = "5.1";
const DEFAULT_LANGUAGE: &str = "en";

// ─── Progress / Events ─────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct PipelineProgress {
    pub job_id: u64,
    pub stage: String,
    pub message: String,
    pub frame: u64,
    pub total_frames: u64,
    pub fps: f64,
    pub elapsed_secs: f64,
    pub percent: f64,
}

// ─── ISDCF naming ──────────────────────────────────────────────────────────

/// One certification rating as the panel collects it.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RatingInput {
    pub agency: String,
    pub label: String,
}

/// The naming fieldset plus the ISDCF naming setting. Every field is package
/// metadata in its own right, so it lands in the CPL whether or not the built
/// name replaces the title.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NamingMetadata {
    pub audio_language: Option<String>,
    pub studio: Option<String>,
    pub territory_type: Option<String>,
    /// Comma-separated, the way the panel's single field collects them.
    pub content_versions: Option<String>,
    pub ratings: Vec<RatingInput>,
    pub temp_version: bool,
    pub pre_release: bool,
    pub red_band: bool,
    pub two_d_version_of_three_d: bool,
    pub version_file: bool,
    pub isdcf_naming: bool,
}

/// Everything the ISDCF content title is built from, so the panel's preview and
/// the submitted job read the same facts.
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IsdcfNameRequest {
    pub title: String,
    pub standard: Option<String>,
    pub resolution: Option<String>,
    pub framerate: Option<String>,
    pub content_kind: Option<String>,
    pub audio_path: Option<String>,
    pub subtitle: Option<String>,
    pub subtitle_language: Option<String>,
    pub burn_subtitle: Option<String>,
    pub ccap: Option<String>,
    pub ccap_language: Option<String>,
    pub right_eye: Option<String>,
    pub atmos: Option<String>,
    pub facility: Option<String>,
    pub naming: NamingMetadata,
    pub source_width: Option<u32>,
    pub source_height: Option<u32>,
    pub crop_left: Option<u32>,
    pub crop_right: Option<u32>,
    pub crop_top: Option<u32>,
    pub crop_bottom: Option<u32>,
    pub rotate: Option<String>,
    #[serde(skip)]
    pub picture_raster: Option<(u32, u32)>,
}

const CONTENT_VERSION_SEPARATOR: char = ',';

fn standard_of(standard: &str) -> dcpwizard_core::Standard {
    match standard {
        "interop" => dcpwizard_core::Standard::Interop,
        _ => dcpwizard_core::Standard::Smpte,
    }
}

fn resolution_of(resolution: &str) -> dcpwizard_core::Resolution {
    if resolution.contains("4k") {
        dcpwizard_core::Resolution::FourK
    } else {
        dcpwizard_core::Resolution::TwoK
    }
}

fn resolution_of_raster(width: u32, height: u32) -> dcpwizard_core::Resolution {
    if width > dcpwizard_core::Resolution::TwoK.width()
        || height > dcpwizard_core::Resolution::TwoK.height()
    {
        dcpwizard_core::Resolution::FourK
    } else {
        dcpwizard_core::Resolution::TwoK
    }
}

fn request_picture_raster(request: &IsdcfNameRequest) -> Result<Option<(u32, u32)>, String> {
    if request.resolution.as_deref() != Some("auto") {
        return Ok(None);
    }
    let (Some(width), Some(height)) = (request.source_width, request.source_height) else {
        return Ok(None);
    };
    let options = dcpwizard_core::source_picture::SourcePictureOptions {
        crop: postkit::picture_processing::Crop {
            left: request.crop_left.unwrap_or(0),
            right: request.crop_right.unwrap_or(0),
            top: request.crop_top.unwrap_or(0),
            bottom: request.crop_bottom.unwrap_or(0),
        },
        rotation: postkit::picture_processing::parse_rotation(
            request.rotate.as_deref().unwrap_or_default(),
        )?,
        ..Default::default()
    };
    let resolved = dcpwizard_core::source_picture::resolve_picture(
        &options,
        Path::new("source"),
        width,
        height,
        &dcpwizard_core::source_picture::EncodeGeometry::default(),
        false,
    )?;
    Ok(Some((resolved.encode_width, resolved.encode_height)))
}

fn content_type_of(content_kind: &str) -> dcpwizard_core::ContentType {
    match content_kind {
        "trailer" => dcpwizard_core::ContentType::Trailer,
        "test" => dcpwizard_core::ContentType::Test,
        "short" => dcpwizard_core::ContentType::Short,
        "advertisement" => dcpwizard_core::ContentType::Advertisement,
        "episode" => dcpwizard_core::ContentType::Episode,
        _ => dcpwizard_core::ContentType::Feature,
    }
}

fn content_versions_of(spec: Option<&str>) -> Vec<String> {
    spec.unwrap_or_default()
        .split(CONTENT_VERSION_SEPARATOR)
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_string)
        .collect()
}

fn ratings_of(ratings: &[RatingInput]) -> Vec<dcpwizard_core::isdcf_name::Rating> {
    ratings
        .iter()
        .filter(|rating| !rating.agency.trim().is_empty() && !rating.label.trim().is_empty())
        .map(|rating| dcpwizard_core::isdcf_name::Rating {
            agency: rating.agency.trim().to_string(),
            label: rating.label.trim().to_string(),
        })
        .collect()
}

fn territory_type_of(
    territory_type: Option<&str>,
) -> Result<dcpwizard_core::isdcf_name::TerritoryType, String> {
    match territory_type.unwrap_or("specific") {
        "specific" => Ok(dcpwizard_core::isdcf_name::TerritoryType::Specific),
        "international-texted" => {
            Ok(dcpwizard_core::isdcf_name::TerritoryType::InternationalTexted)
        }
        "international-textless" => {
            Ok(dcpwizard_core::isdcf_name::TerritoryType::InternationalTextless)
        }
        other => Err(format!("Unknown territory type '{other}'")),
    }
}

/// The ISDCF content title for what the panel currently holds.
fn isdcf_name_for(request: &IsdcfNameRequest) -> Result<String, String> {
    let resolution = request.resolution.as_deref().unwrap_or(DEFAULT_RESOLUTION);
    let picture_raster = if resolution == "auto" {
        request.picture_raster.or(request_picture_raster(request)?)
    } else {
        None
    };
    let (container_width, container_height) =
        picture_raster.unwrap_or_else(|| container_of(resolution));
    let (frame_rate_num, frame_rate_den) =
        frame_rate_of(request.framerate.as_deref().unwrap_or(DEFAULT_FRAME_RATE.0));
    let some_path = |value: &Option<String>| {
        value
            .as_deref()
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
    };

    let config = dcpwizard_core::dcp::DcpConfig {
        title: request.title.clone(),
        standard: standard_of(request.standard.as_deref().unwrap_or(DEFAULT_STANDARD)),
        resolution: picture_raster
            .map(|(width, height)| resolution_of_raster(width, height))
            .unwrap_or_else(|| resolution_of(resolution)),
        content_type: content_type_of(
            request
                .content_kind
                .as_deref()
                .unwrap_or(DEFAULT_CONTENT_KIND),
        ),
        frame_rate_num,
        frame_rate_den,
        container_width,
        container_height,
        stereo_3d: some_path(&request.right_eye).is_some(),
        atmos_path: some_path(&request.atmos),
        subtitle_path: some_path(&request.subtitle),
        subtitle_language: request
            .subtitle_language
            .clone()
            .unwrap_or_else(|| DEFAULT_LANGUAGE.into()),
        ccap_path: some_path(&request.ccap),
        ccap_language: request
            .ccap_language
            .clone()
            .unwrap_or_else(|| DEFAULT_LANGUAGE.into()),
        facility: request.facility.clone().filter(|code| !code.is_empty()),
        audio_language: request
            .naming
            .audio_language
            .clone()
            .filter(|tag| !tag.is_empty()),
        ratings: ratings_of(&request.naming.ratings),
        content_versions: content_versions_of(request.naming.content_versions.as_deref()),
        ..Default::default()
    };

    let options = dcpwizard_core::isdcf_title::IsdcfNamingOptions {
        studio: request
            .naming
            .studio
            .clone()
            .filter(|code| !code.is_empty()),
        temp_version: request.naming.temp_version,
        pre_release: request.naming.pre_release,
        red_band: request.naming.red_band,
        two_d_version_of_three_d: request.naming.two_d_version_of_three_d,
        territory_type: territory_type_of(request.naming.territory_type.as_deref())?,
        date: None,
        version_file: request.naming.version_file,
    };

    // the panel has no accessibility channel fields, so the summary is the
    // selected WAV's channel count and nothing else
    let channel_count = match some_path(&request.audio_path) {
        Some(path) => postkit::wav_io::channel_count(&path)?,
        None => 0,
    };
    let sound = dcpwizard_core::isdcf_title::soundtrack_summary(channel_count, None, None);

    Ok(dcpwizard_core::isdcf_title::isdcf_title(
        &config,
        &options,
        &sound,
        some_path(&request.burn_subtitle).is_some(),
    ))
}

/// The output folder under its new name, when the panel derived it from the
/// title. A folder the user chose themselves is left alone.
fn renamed_output_dir(output_dir: &str, title: &str, name: &str) -> PathBuf {
    let output_path = PathBuf::from(output_dir);
    match output_path.file_name().and_then(|folder| folder.to_str()) {
        Some(folder) if folder == title => output_path.with_file_name(name),
        _ => output_path,
    }
}

/// The ISDCF content title the panel would build, for the live preview.
#[tauri::command]
pub async fn isdcf_name_preview(request: IsdcfNameRequest) -> Result<String, String> {
    isdcf_name_for(&request)
}

// ─── Job types ─────────────────────────────────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct JobConfig {
    pub(crate) id: u64,
    video_path: PathBuf,
    title: String,
    output_dir: PathBuf,
    audio_path: Option<String>,
    validate: bool,
    standard: String,
    resolution: String,
    framerate: String,
    bandwidth: u32,
    /// PSNR target in dB for the J2K encode, None when the encode allocates by
    /// compression ratio.
    #[serde(default)]
    quality_psnr: Option<f64>,
    colour: String,
    content_kind: String,
    encrypt: bool,
    key_out: Option<String>,
    channels: String,
    // right-eye video for a stereoscopic 3D DCP (main input is the left eye)
    right_eye: Option<String>,
    // dolby atmos / dcdata bitstream wrapped as a ST 429-18 aux track
    atmos: Option<String>,
    subtitle: Option<String>,
    subtitle_language: String,
    // subtitle file drawn into the picture during the encode. Registers no
    // timed-text track: burnt-in text is part of the image.
    burn_subtitle: Option<String>,
    burn_subtitle_font: Option<String>,
    // how the burnt-in text and the packaged track look
    burn_style: postkit::subtitle_raster::BurnStyleOverrides,
    subtitle_appearance: dcpwizard_core::subtitle::TimedTextAppearance,
    ccap: Option<String>,
    ccap_language: String,
    // loudness normalize spec (leqm=<db> or lufs=<value>) applied to the audio
    loudness_target: Option<String>,
    true_peak_ceiling: Option<f64>,
    // directory of mono channel WAVs (name_L.wav, name_Lfe.wav, ...) routed to
    // one interleaved WAV, replacing audio_path
    audio_channel_dir: Option<String>,
    audio_input_order: dcpwizard_core::mxf_wrap::AudioInputOrder,
    // how many channels the packaged sound track is filled to. None widens 5.1
    // to 16 and packages every other source at its own width
    audio_channels: Option<u32>,
    // sign-language video (ISDCF Doc 13) packed onto sound channel 15
    sign_language_video: Option<String>,
    sign_language_tag: Option<String>,
    pad_head: Option<String>,
    pad_tail: Option<String>,
    pad_color: Option<String>,
    // shift the sound against the picture, keeping the running time
    audio_delay_ms: i64,
    // head/tail trim of the source in frames, applied before any padding
    trim_start_frames: u64,
    trim_end_frames: u64,
    // how long a single-image input is held, in frames. Zero = not a still
    still_length_frames: u64,
    // crop, deinterlace, denoise, rotate, flip and container fill, applied to
    // the source while it decodes
    picture: dcpwizard_core::source_picture::SourcePictureOptions,
    // IN:LANE@GAIN mix matrix applied to the sound before anything else
    audio_map: Option<String>,
    // colour space the source carries, which decides the encoder transform
    source_colourspace: postkit::colour::ColourSpace,
    // stereo -> 5.1 upmix applied before loudness normalization
    upmix: Option<postkit::upmix::Upmixer>,
    reel_length_minutes: u32,
    // explicit reel boundaries in frames, from the panel's split timecodes
    reel_split_frames: Vec<u64>,
    split_chapters: bool,
    // one CPL per entry over shared essence; empty keeps the single-CPL path
    versions: Vec<dcpwizard_core::versions::VersionSpec>,
    // DCI HDR Addendum: stamp ST 2084 PQ / P3-D65 on the picture MXF
    hdr_dci: bool,
    // how the source reaches the encoder: display RGB, through an HDR-to-DCI
    // LUT, or already PQ. Nothing but display RGB gets the X'Y'Z' transform.
    source_colour: postkit::encode::SourceColour,
    // tone map an HDR source down to SDR with ffmpeg's generic transform
    allow_generic_hdr_tonemap: bool,
    // mastering facility for the composition metadata and the ISDCF name
    facility: Option<String>,
    // naming and metadata from the panel's fieldset
    naming: NamingMetadata,
    /// What ffprobe read from the source. The probe counts frames by decoding,
    /// so the check runs one and the build reads it back rather than paying twice.
    source: Option<postkit::probe::VideoInfo>,
    /// What the pre-build check found, carried through so the job log lists it
    /// without measuring the source a second time.
    hints: Vec<String>,
    /// Library items joined onto the build as reels before the feature's, in
    /// the order the panel put them in, and after it for the tail.
    head_items: Vec<dcpwizard_core::library::AttachedItem>,
    tail_items: Vec<dcpwizard_core::library::AttachedItem>,
}

fn apply_isdcf_name_to_job(
    job: &mut JobConfig,
    picture_raster: Option<(u32, u32)>,
) -> Result<(), String> {
    if !job.naming.isdcf_naming {
        return Ok(());
    }
    let request = IsdcfNameRequest {
        title: job.title.clone(),
        standard: Some(job.standard.clone()),
        resolution: Some(job.resolution.clone()),
        framerate: Some(job.framerate.clone()),
        content_kind: Some(job.content_kind.clone()),
        audio_path: job.audio_path.clone(),
        subtitle: job.subtitle.clone(),
        subtitle_language: Some(job.subtitle_language.clone()),
        burn_subtitle: job.burn_subtitle.clone(),
        ccap: job.ccap.clone(),
        ccap_language: Some(job.ccap_language.clone()),
        right_eye: job.right_eye.clone(),
        atmos: job.atmos.clone(),
        facility: job.facility.clone(),
        naming: job.naming.clone(),
        picture_raster,
        ..Default::default()
    };
    let name = isdcf_name_for(&request)?;
    job.output_dir =
        renamed_output_dir(job.output_dir.to_string_lossy().as_ref(), &job.title, &name);
    job.title = name;
    Ok(())
}

// ─── Queue state (managed by Tauri) ────────────────────────────────────────

impl postkit::gui_job_queue::GuiJob for JobConfig {
    fn id(&self) -> u64 {
        self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn output_dir(&self) -> &Path {
        &self.output_dir
    }
}

pub type JobQueue = postkit::gui_job_queue::GuiJobQueue<JobConfig>;

/// Where the Jobs panel keeps its queue. `DCPWIZARD_JOBS_FILE` is the daemon's
/// own jobs.jsonl and names a different file.
pub fn jobs_path() -> PathBuf {
    postkit::gui_job_queue::jobs_path("DCPWIZARD_GUI_JOBS_FILE", dcpwizard_core::store::data_dir())
}

/// Files a finished DCP always has at its root.
const DCP_ROOT_FILES: [&str; 2] = ["ASSETMAP.xml", "VOLINDEX.xml"];

fn holds_dcp(dir: &std::path::Path) -> bool {
    DCP_ROOT_FILES.iter().any(|name| dir.join(name).exists())
}

/// What a panel field holds, or None when the user left it empty.
fn filled(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|text| !text.is_empty())
}

/// A number typed into an appearance field, refused under the name of the
/// `create` flag it feeds.
fn parsed_field<T: std::str::FromStr>(
    flag: &str,
    value: &Option<String>,
) -> Result<Option<T>, String> {
    match filled(value) {
        Some(text) => text
            .parse()
            .map(Some)
            .map_err(|_| format!("{flag}: {text} is not a number")),
        None => Ok(None),
    }
}

/// A colour typed into an appearance field, refused under the name of the
/// `create` flag it feeds.
fn parsed_colour(
    flag: &str,
    value: &Option<String>,
) -> Result<Option<postkit::subtitle_formats::Rgba>, String> {
    match filled(value) {
        Some(text) => dcpwizard_core::subtitle::parse_colour_flag(flag, text).map(Some),
        None => Ok(None),
    }
}

/// What a submitted build came back with: the queued job, or the hints that
/// have to be shown before one is queued.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResult {
    pub job_id: Option<u64>,
    pub hints: Vec<String>,
}

// ─── Tauri commands ────────────────────────────────────────────────────────

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn submit_job(
    app: AppHandle,
    video_path: String,
    title: String,
    output_dir: String,
    audio_path: Option<String>,
    validate: Option<bool>,
    standard: Option<String>,
    resolution: Option<String>,
    framerate: Option<String>,
    bandwidth: Option<u32>,
    quality_psnr: Option<f64>,
    colour: Option<String>,
    content_kind: Option<String>,
    encrypt: Option<bool>,
    key_out: Option<String>,
    channels: Option<String>,
    right_eye: Option<String>,
    atmos: Option<String>,
    subtitle: Option<String>,
    subtitle_language: Option<String>,
    subtitle_font_size: Option<String>,
    subtitle_colour: Option<String>,
    subtitle_effect: Option<String>,
    subtitle_effect_colour: Option<String>,
    subtitle_fade_up: Option<String>,
    subtitle_fade_down: Option<String>,
    burn_subtitle: Option<String>,
    burn_subtitle_font: Option<String>,
    burn_font_size: Option<String>,
    burn_colour: Option<String>,
    burn_effect: Option<String>,
    burn_effect_colour: Option<String>,
    burn_outline_width: Option<String>,
    burn_line_height: Option<String>,
    burn_margin: Option<String>,
    burn_fade_up: Option<String>,
    burn_fade_down: Option<String>,
    ccap: Option<String>,
    ccap_language: Option<String>,
    loudness_target: Option<String>,
    true_peak_ceiling: Option<f64>,
    audio_channel_dir: Option<String>,
    audio_input_order: Option<String>,
    audio_channels: Option<u32>,
    audio_map: Option<String>,
    crop_left: Option<u32>,
    crop_right: Option<u32>,
    crop_top: Option<u32>,
    crop_bottom: Option<u32>,
    fill_crop: Option<bool>,
    deinterlace: Option<bool>,
    denoise: Option<bool>,
    rotate: Option<String>,
    flip: Option<String>,
    sign_language_video: Option<String>,
    sign_language_tag: Option<String>,
    pad_head: Option<String>,
    pad_tail: Option<String>,
    pad_color: Option<String>,
    audio_delay_ms: Option<i64>,
    trim_start: Option<String>,
    trim_end: Option<String>,
    still_length: Option<String>,
    source_colourspace: Option<String>,
    upmix: Option<String>,
    reel_length_minutes: Option<u32>,
    split_at: Option<String>,
    split_chapters: Option<bool>,
    versions: Option<String>,
    hdr_dci: Option<bool>,
    hdr_to_dci_lut: Option<String>,
    hdr_already_pq: Option<bool>,
    allow_generic_hdr_tonemap: Option<bool>,
    facility: Option<String>,
    naming: Option<NamingMetadata>,
    head_items: Option<Vec<String>>,
    tail_items: Option<Vec<String>>,
    hints_accepted: Option<bool>,
) -> Result<SubmitResult, String> {
    let queue = app.state::<JobQueue>();
    let id = queue.reserve_job_id();

    // Never encrypt without an explicit key destination.
    if encrypt.unwrap_or(false) && key_out.as_deref().unwrap_or("").is_empty() {
        return Err("Key Output File is required when encrypting".into());
    }

    // a name the library no longer holds, or media it has lost, fails here
    // rather than after the encode
    let library = dcpwizard_core::library::Library::open();
    let head_items =
        dcpwizard_core::library_reel::attach_by_name(&library, &head_items.unwrap_or_default())?;
    let tail_items =
        dcpwizard_core::library_reel::attach_by_name(&library, &tail_items.unwrap_or_default())?;

    let framerate = framerate.unwrap_or_else(|| DEFAULT_FRAME_RATE.0.into());
    let naming = naming.unwrap_or_default();
    let facility = facility.filter(|code| !code.is_empty());

    let audio_input_order = parse_audio_input_order(audio_input_order.as_deref())?;

    let sign_language_video = sign_language_video.filter(|s| !s.is_empty());
    let sign_language_tag = sign_language_tag.filter(|s| !s.is_empty());
    if sign_language_video.is_some() && sign_language_tag.is_none() {
        return Err("Sign Language tag is required with a sign-language video".into());
    }

    // reject bad pad specs here, before the encode, not after it.
    let pad_head = pad_head.filter(|s| !s.is_empty());
    let pad_tail = pad_tail.filter(|s| !s.is_empty());
    let pad_color = pad_color.filter(|s| !s.is_empty());
    let (fps_num, _) = frame_rate_of(&framerate);
    for spec in [pad_head.as_deref(), pad_tail.as_deref()]
        .into_iter()
        .flatten()
    {
        dcpwizard_core::pad::parse_pad_frames(spec, fps_num)?;
    }
    if let Some(spec) = pad_color.as_deref() {
        dcpwizard_core::pad::parse_pad_color(spec)?;
    }

    // trim and still specs share the pad syntax, and are parsed here for the same
    // reason: a bad spec must fail before the encode, not after it.
    let duration_frames = |spec: Option<&str>, label: &str| -> Result<u64, String> {
        match spec.map(str::trim).filter(|s| !s.is_empty()) {
            Some(spec) => dcpwizard_core::pad::parse_pad_frames(spec, fps_num)
                .map_err(|e| format!("{label}: {e}")),
            None => Ok(0),
        }
    };
    let trim_start_frames = duration_frames(trim_start.as_deref(), "Trim start")?;
    let trim_end_frames = duration_frames(trim_end.as_deref(), "Trim end")?;
    let still_length_frames = duration_frames(still_length.as_deref(), "Still length")?;

    let video = PathBuf::from(&video_path);
    let still_input = postkit::still::is_still_image(&video);
    if still_input && still_length_frames == 0 {
        return Err("The video is a single image and has no length: set a still length".into());
    }
    if !still_input && still_length_frames > 0 {
        return Err(
            "Still length applies to a single-image video; a video carries its own length".into(),
        );
    }
    if still_input && trim_start_frames + trim_end_frames > 0 {
        return Err(
            "A still is held for exactly its still length: shorten that instead of trimming".into(),
        );
    }

    let source_colourspace = dcpwizard_core::encode::parse_source_colourspace(
        source_colourspace.as_deref().unwrap_or("rec709"),
    )?;
    let xyz_route = dcpwizard_core::encode::xyz_route(source_colourspace)?;
    // a J2K directory is picture that is already encoded: no transform runs
    // over it, so a colour space here would be ignored
    if postkit::encode::detect_input_type(&video) == postkit::encode::InputType::J2kSequence {
        dcpwizard_core::encode::check_precompressed_colourspace(source_colourspace)?;
    }

    let upmix = parse_upmixer(upmix.as_deref())?;

    let (flip_horizontal, flip_vertical) =
        postkit::picture_processing::parse_flip(flip.as_deref().unwrap_or_default())?;
    let picture = dcpwizard_core::source_picture::SourcePictureOptions {
        crop: postkit::picture_processing::Crop {
            left: crop_left.unwrap_or(0),
            right: crop_right.unwrap_or(0),
            top: crop_top.unwrap_or(0),
            bottom: crop_bottom.unwrap_or(0),
        },
        fill_crop: fill_crop.unwrap_or(false),
        deinterlace: deinterlace.unwrap_or(false),
        denoise: denoise.unwrap_or(false),
        rotation: postkit::picture_processing::parse_rotation(
            rotate.as_deref().unwrap_or_default(),
        )?,
        flip_horizontal,
        flip_vertical,
        ..dcpwizard_core::source_picture::SourcePictureOptions::default()
    };
    if postkit::encode::detect_input_type(&video) == postkit::encode::InputType::J2kSequence {
        dcpwizard_core::source_picture::check_precompressed_picture(&picture)?;
    }
    let panel_resolution = resolution.as_deref().unwrap_or(DEFAULT_RESOLUTION);
    if picture.fill_crop && container_of(panel_resolution) == NO_CONTAINER {
        return Err(
            "Fill container needs a picture resolution to fill: choose one instead of Auto".into(),
        );
    }

    // the map places every channel by hand, and each of these places channels
    // its own way, so two of them would fight over the same lanes
    let audio_map = audio_map.filter(|spec| !spec.trim().is_empty());
    if audio_map.is_some() {
        let competing = [
            (
                audio_input_order == dcpwizard_core::mxf_wrap::AudioInputOrder::LrcLsRsLfe,
                "the six-channel input order",
            ),
            (upmix.is_some(), "the stereo upmix"),
            (
                audio_channel_dir
                    .as_deref()
                    .is_some_and(|dir| !dir.is_empty()),
                "a channel WAV directory",
            ),
        ];
        if let Some((_, name)) = competing.into_iter().find(|(set, _)| *set) {
            return Err(format!(
                "The audio mapping and {name} both decide which DCP lane each channel lands on: \
                 use one or the other"
            ));
        }
    }

    // reel splitting: length, timecodes and chapters are three ways to say the
    // same thing, so only one may be set (the CLI rejects the combos too).
    let reel_length_minutes = reel_length_minutes.unwrap_or(0);
    let split_at = split_at.filter(|s| !s.trim().is_empty());
    let split_chapters = split_chapters.unwrap_or(false);
    let split_sources = [reel_length_minutes > 0, split_at.is_some(), split_chapters]
        .into_iter()
        .filter(|set| *set)
        .count();
    if split_sources > 1 {
        return Err(
            "Choose one reel split: reel length, split timecodes, or split at chapters".into(),
        );
    }
    let reel_split_frames = match split_at.as_deref() {
        Some(spec) => parse_split_timecodes(spec, fps_num)?,
        None => Vec::new(),
    };

    let versions = load_version_specs(versions.as_deref())?;
    let subtitle = subtitle.filter(|s| !s.is_empty());
    let ccap = ccap.filter(|s| !s.is_empty());
    if !versions.is_empty() && (subtitle.is_some() || ccap.is_some()) {
        return Err(
            "A versions manifest carries its own subtitles and captions: clear the subtitle and CCAP fields".into(),
        );
    }
    // multi-CPL packages reel by length only, so refuse the split the packer
    // would drop on the floor.
    if !versions.is_empty() && (!reel_split_frames.is_empty() || split_chapters) {
        return Err(
            "A versions manifest splits reels by length only: clear the split timecodes and chapter split".into(),
        );
    }

    let subtitle_appearance = dcpwizard_core::subtitle::TimedTextAppearance::from_flags(
        parsed_field("--subtitle-font-size", &subtitle_font_size)?,
        filled(&subtitle_colour),
        filled(&subtitle_effect),
        filled(&subtitle_effect_colour),
        parsed_field("--subtitle-fade-up", &subtitle_fade_up)?,
        parsed_field("--subtitle-fade-down", &subtitle_fade_down)?,
    )?;
    let burn_style = postkit::subtitle_raster::BurnStyleOverrides {
        font_size_percent: parsed_field("--burn-font-size", &burn_font_size)?,
        colour: parsed_colour("--burn-colour", &burn_colour)?,
        effect: match filled(&burn_effect) {
            Some(text) => Some(dcpwizard_core::subtitle::parse_effect_flag(
                "--burn-effect",
                text,
            )?),
            None => None,
        },
        effect_colour: parsed_colour("--burn-effect-colour", &burn_effect_colour)?,
        outline_width_percent: parsed_field("--burn-outline-width", &burn_outline_width)?,
        line_height_ratio: parsed_field("--burn-line-height", &burn_line_height)?,
        margin_percent: parsed_field("--burn-margin", &burn_margin)?,
        x_scale: None,
        y_scale: None,
        fade_up_ms: parsed_field("--burn-fade-up", &burn_fade_up)?,
        fade_down_ms: parsed_field("--burn-fade-down", &burn_fade_down)?,
    };

    let burn_subtitle = burn_subtitle.filter(|s| !s.is_empty());
    let burn_subtitle_font = burn_subtitle_font.filter(|s| !s.is_empty());
    if subtitle.is_none()
        && subtitle_appearance != dcpwizard_core::subtitle::TimedTextAppearance::default()
    {
        return Err(
            "The subtitle appearance styles the timed-text track the subtitle field packages, and captions keep the default appearance: pick a subtitle file or clear those fields".into(),
        );
    }
    if burn_subtitle.is_none()
        && burn_style != postkit::subtitle_raster::BurnStyleOverrides::default()
    {
        return Err(
            "The burn-in appearance styles the text the burn-in field draws into the picture: pick a burn-in subtitle file or clear those fields".into(),
        );
    }
    if let Some(path) = burn_subtitle.as_deref() {
        if !versions.is_empty() {
            return Err(
                "A versions manifest carries its own subtitles: clear the burn-in field".into(),
            );
        }
        // parse the cue file and build the burn now, so a bad file or a missing
        // font fails here instead of part way through the encode
        let timed_text_paths: Vec<&Path> = [subtitle.as_deref(), ccap.as_deref()]
            .into_iter()
            .flatten()
            .map(Path::new)
            .collect();
        dcpwizard_core::subtitle::check_burn_supported(
            Path::new(path),
            &timed_text_paths,
            matches!(xyz_route, dcpwizard_core::encode::XyzRoute::AlreadyXyz)
                || hdr_already_pq.unwrap_or(false)
                || hdr_to_dci_lut.is_some(),
            postkit::encode::detect_input_type(&video) == postkit::encode::InputType::J2kSequence,
        )?;
        dcpwizard_core::subtitle::prepare_subtitle_burn(
            Path::new(path),
            burn_subtitle_font.as_deref().map(Path::new),
            postkit::encode::FrameRate::whole(fps_num),
            &burn_style,
        )?;
    }

    let right_eye = right_eye.filter(|s| !s.is_empty());
    let hdr_dci = hdr_dci.unwrap_or(false);
    let allow_generic_hdr_tonemap = allow_generic_hdr_tonemap.unwrap_or(false);
    // the generic tone map lands on Rec.709, so it and a wide-gamut source
    // colour space cannot both describe the frames the encoder gets
    if allow_generic_hdr_tonemap && !xyz_route.compressor_transform() {
        return Err(
            "The source colour space and the generic HDR tone map both decide what the encoder converts: set one or the other".into(),
        );
    }
    let hdr_source_colour = resolve_hdr(
        &HdrPanelOptions {
            dci: hdr_dci,
            lut: hdr_to_dci_lut,
            already_pq: hdr_already_pq.unwrap_or(false),
            allow_generic_tonemap: allow_generic_hdr_tonemap,
        },
        fps_num,
        bandwidth.unwrap_or(DEFAULT_BANDWIDTH_MBPS),
        right_eye.is_some(),
        reel_length_minutes > 0 || !reel_split_frames.is_empty() || split_chapters,
        !versions.is_empty(),
    )?;
    // the source colour space and the HDR options both answer "does the encoder
    // run its X'Y'Z' transform?", so only one of them may.
    let source_colour = match (
        xyz_route.compressor_transform(),
        hdr_source_colour.applies_xyz_transform(),
    ) {
        (true, _) => hdr_source_colour,
        (false, true) => xyz_route.source_colour(),
        (false, false) => {
            return Err(
                "The source colour space and the HDR options both decide the encoder's colour transform: set one or the other".into(),
            );
        }
    };

    let mut job = JobConfig {
        id,
        video_path: PathBuf::from(&video_path),
        title: title.clone(),
        output_dir: PathBuf::from(&output_dir),
        audio_path,
        validate: validate.unwrap_or(false),
        standard: standard.unwrap_or_else(|| DEFAULT_STANDARD.into()),
        resolution: resolution.unwrap_or_else(|| DEFAULT_RESOLUTION.into()),
        framerate,
        bandwidth: bandwidth.unwrap_or(DEFAULT_BANDWIDTH_MBPS),
        quality_psnr,
        colour: colour.unwrap_or_else(|| DEFAULT_COLOUR.into()),
        content_kind: content_kind.unwrap_or_else(|| DEFAULT_CONTENT_KIND.into()),
        encrypt: encrypt.unwrap_or(false),
        key_out: key_out.filter(|k| !k.is_empty()),
        channels: channels.unwrap_or_else(|| DEFAULT_CHANNELS.into()),
        right_eye,
        atmos: atmos.filter(|s| !s.is_empty()),
        subtitle,
        subtitle_language: subtitle_language.unwrap_or_else(|| DEFAULT_LANGUAGE.into()),
        burn_subtitle,
        burn_subtitle_font,
        burn_style,
        subtitle_appearance,
        ccap,
        ccap_language: ccap_language.unwrap_or_else(|| DEFAULT_LANGUAGE.into()),
        loudness_target: loudness_target.filter(|s| !s.is_empty()),
        true_peak_ceiling,
        audio_channel_dir: audio_channel_dir.filter(|s| !s.is_empty()),
        audio_input_order,
        audio_channels,
        audio_map,
        picture,
        sign_language_video,
        sign_language_tag,
        pad_head,
        pad_tail,
        pad_color,
        audio_delay_ms: audio_delay_ms.unwrap_or(0),
        trim_start_frames,
        trim_end_frames,
        still_length_frames,
        source_colourspace,
        upmix,
        reel_length_minutes,
        reel_split_frames,
        split_chapters,
        versions,
        hdr_dci,
        source_colour,
        allow_generic_hdr_tonemap,
        facility,
        naming,
        source: probe_job_source(&video, still_input),
        hints: Vec::new(),
        head_items,
        tail_items,
    };

    let (plan, planned_picture) = checked_job_plan(&job)?;
    apply_isdcf_name_to_job(&mut job, planned_picture.map(|picture| picture.raster))?;

    // packages are folders named by title, so a reused title lands in the old
    // package. refuse now, not after the encode.
    if holds_dcp(&job.output_dir) {
        return Err(format!(
            "Output folder already holds a DCP: {}. Use a new title or output folder, or delete the old package first.",
            job.output_dir.display()
        ));
    }
    if queue.is_building_into(&job.output_dir) {
        return Err(format!(
            "A build is already running into {}. Wait for it to finish or cancel it.",
            job.output_dir.display()
        ));
    }
    let hints: Vec<String> = dcpwizard_core::hints::gather_hints(&plan)
        .into_iter()
        .map(|hint| hint.text)
        .collect();
    // the pref lives in the panel, which says it has taken the hints by sending
    // hintsAccepted rather than by naming the pref here
    if !hints.is_empty() && hints_accepted != Some(true) {
        return Ok(SubmitResult {
            job_id: None,
            hints,
        });
    }
    let job = JobConfig {
        hints: hints.clone(),
        ..job
    };

    queue.submit(job);

    if !queue.has_running_job() {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            run_queue_worker(app2).await;
        });
    }

    Ok(SubmitResult {
        job_id: Some(id),
        hints,
    })
}

/// Read the source once for the pre-build check. A codestream directory has
/// nothing for ffprobe to read, so it is not offered one.
fn probe_job_source(
    video: &std::path::Path,
    still_input: bool,
) -> Option<postkit::probe::VideoInfo> {
    let codestreams =
        postkit::encode::detect_input_type(video) == postkit::encode::InputType::J2kSequence;
    (still_input || !codestreams)
        .then(|| dcpwizard_core::probe::probe_video(video))
        .flatten()
}

/// One description of the job for the checks and the hints, from the same
/// values the build itself runs on.
fn job_plan(job: &JobConfig) -> dcpwizard_core::preflight::CreatePlan {
    let (fps, _) = frame_rate_of(&job.framerate);
    let pad_frames = |spec: &Option<String>| -> u64 {
        spec.as_deref()
            .and_then(|spec| dcpwizard_core::pad::parse_pad_frames(spec, fps).ok())
            .unwrap_or(0)
    };
    let codestreams = postkit::encode::detect_input_type(&job.video_path)
        == postkit::encode::InputType::J2kSequence;
    dcpwizard_core::preflight::CreatePlan {
        picture: job.video_path.clone(),
        picture_kind: match (job.still_length_frames > 0, codestreams) {
            (true, _) => dcpwizard_core::preflight::PictureKind::Still,
            (_, true) => dcpwizard_core::preflight::PictureKind::Codestreams,
            _ => dcpwizard_core::preflight::PictureKind::Video,
        },
        source: job.source.clone(),
        still_frames: job.still_length_frames,
        fps,
        picture_options: job.picture.clone(),
        geometry: job_geometry(job),
        trim_start_frames: job.trim_start_frames,
        trim_end_frames: job.trim_end_frames,
        pad_head_frames: pad_frames(&job.pad_head),
        pad_tail_frames: pad_frames(&job.pad_tail),
        audio: job
            .audio_path
            .as_ref()
            .or(job.audio_channel_dir.as_ref())
            .map(PathBuf::from),
        audio_map: job.audio_map.clone(),
        upmix: job.upmix.is_some(),
        audio_channels: job.audio_channels,
        audio_language: job.naming.audio_language.clone(),
        subtitle: job.subtitle.as_ref().map(PathBuf::from),
        ccap: job.ccap.as_ref().map(PathBuf::from),
        burn_subtitle: job.burn_subtitle.as_ref().map(PathBuf::from),
        burn_subtitle_font: job.burn_subtitle_font.as_ref().map(PathBuf::from),
        burn_style: job.burn_style.clone(),
        source_colourspace: job.source_colourspace,
        frames_already_xyz: !job.source_colour.applies_xyz_transform(),
        atmos: job.atmos.as_ref().map(PathBuf::from),
        // the panel places no markers, so a composition gets the default pair
        markers: Vec::new(),
        standard: standard_of(&job.standard),
        content_type: content_type_of(&job.content_kind),
        encrypt: job.encrypt,
        hdr_dci: job.hdr_dci,
        video_bit_rate_mbps: job.bandwidth,
        right_eye: job.right_eye.as_ref().map(PathBuf::from),
        four_k: job.resolution.contains("4k"),
        reel_length_minutes: job.reel_length_minutes,
        reel_split_frames: job.reel_split_frames.clone(),
        library_items: job.head_items.len() + job.tail_items.len(),
    }
}

fn checked_job_plan(
    job: &JobConfig,
) -> Result<
    (
        dcpwizard_core::preflight::CreatePlan,
        Option<dcpwizard_core::preflight::PlannedPicture>,
    ),
    String,
> {
    let mut plan = job_plan(job);
    let planned_picture = dcpwizard_core::preflight::plan_picture(&plan)?;
    if let Some(picture) = planned_picture {
        plan.four_k = resolution_of_raster(picture.raster.0, picture.raster.1)
            == dcpwizard_core::Resolution::FourK;
    }
    dcpwizard_core::preflight::check_before_encode(&plan)?;
    Ok((plan, planned_picture))
}

// ─── Delivery profiles ─────────────────────────────────────────────────────

/// A delivery profile as panel control values. A field is null when the profile
/// carries nothing the panel can express, and the panel leaves that control
/// alone.
#[derive(Clone, Serialize)]
pub struct ProfilePanelSettings {
    pub name: String,
    pub description: String,
    pub standard: Option<String>,
    pub resolution: Option<String>,
    pub framerate: String,
    pub bandwidth: u32,
    pub content_kind: String,
}

fn profile_panel_settings(profile: &dcpwizard_core::profiles::Profile) -> ProfilePanelSettings {
    let standard = profile.standard.to_lowercase();
    ProfilePanelSettings {
        name: profile.name.clone(),
        description: profile.description.clone(),
        standard: matches!(standard.as_str(), "smpte" | "interop").then_some(standard),
        resolution: resolution_key_of(profile.resolution_width, profile.resolution_height)
            .map(str::to_string),
        framerate: profile.frame_rate.to_string(),
        bandwidth: profile.bitrate_mbps,
        content_kind: profile.content_kind.clone(),
    }
}

fn profile_panel_list() -> Vec<ProfilePanelSettings> {
    dcpwizard_core::profiles::all_profiles()
        .iter()
        .map(profile_panel_settings)
        .collect()
}

#[tauri::command]
pub async fn list_profiles() -> Vec<ProfilePanelSettings> {
    profile_panel_list()
}

/// The black borders a source carries, and what the picture plan around them
/// does, for the panel's auto-crop button.
#[derive(Serialize)]
pub struct DetectedCrop {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
    pub description: String,
}

/// Measure the black borders around `video_path`'s content.
#[tauri::command]
pub async fn detect_source_crop(
    video_path: String,
    threshold: Option<f32>,
    resolution: Option<String>,
) -> Result<DetectedCrop, String> {
    let source = PathBuf::from(&video_path);
    let info = dcpwizard_core::probe::probe_video(&source)
        .ok_or_else(|| format!("cannot read the size of {video_path}"))?;
    let resolved = dcpwizard_core::source_picture::resolve_picture(
        &dcpwizard_core::source_picture::SourcePictureOptions {
            auto_crop: true,
            auto_crop_threshold: threshold
                .unwrap_or(postkit::picture_processing::DEFAULT_AUTO_CROP_THRESHOLD),
            ..dcpwizard_core::source_picture::SourcePictureOptions::default()
        },
        &source,
        info.width,
        info.height,
        &geometry_for_resolution(resolution.as_deref().unwrap_or(DEFAULT_RESOLUTION)),
        postkit::encode::detect_input_type(&source) == postkit::encode::InputType::ImageSequence,
    )?;
    Ok(DetectedCrop {
        left: resolved.processing.crop.left,
        right: resolved.processing.crop.right,
        top: resolved.processing.crop.top,
        bottom: resolved.processing.crop.bottom,
        description: resolved.plan.describe(),
    })
}

/// Where the preview's SRT copies of the timed text are written, inside the
/// app's cache folder.
const PREVIEW_SUBTITLE_DIRECTORY: &str = "preview-subtitles";

/// A subtitle file the preview player can render, converting the job's timed
/// text to SRT when mpv cannot read it as it stands. `source_path` is a subtitle
/// file the job packages or a built DCP directory, and `track` picks which
/// timed-text track of a DCP is read.
#[tauri::command]
pub async fn subtitle_file_for_preview(
    app: AppHandle,
    source_path: String,
    track: dcpwizard_core::subtitle_extract::PackagedTrack,
    fps: u32,
) -> Result<String, String> {
    let work_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("no cache folder to write the preview subtitles into: {e}"))?
        .join(PREVIEW_SUBTITLE_DIRECTORY);
    let playable = dcpwizard_core::subtitle_preview::playable_subtitle_file(
        &PathBuf::from(&source_path),
        track,
        fps,
        &work_dir,
    )?;
    Ok(playable.to_string_lossy().into_owned())
}

/// The audio mapping grid the panel draws: one row per source channel, one
/// column per DCP lane.
#[derive(Serialize)]
pub struct AudioMapPanel {
    pub channels: usize,
    pub lanes: Vec<String>,
}

/// How many channels the chosen WAV carries, and the lanes a map may name.
#[tauri::command]
pub async fn probe_audio_map(audio_path: String) -> Result<AudioMapPanel, String> {
    Ok(AudioMapPanel {
        channels: postkit::wav_io::channel_count(std::path::Path::new(&audio_path))?,
        lanes: dcpwizard_core::audio_map::DCP_LANE_NAMES
            .iter()
            .map(|lane| lane.to_string())
            .collect(),
    })
}

#[derive(Serialize)]
pub struct DiskSpace {
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub percent_free: f64,
}

/// Free space on the volume holding `path`.
#[tauri::command]
pub async fn disk_space(path: String) -> Result<DiskSpace, String> {
    // the output folder is only created once the build starts, so report the
    // volume of the nearest folder that does exist
    let mut dir = PathBuf::from(&path);
    while !dir.exists() {
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Err(format!("no existing folder above {path}")),
        }
    }
    let (free, total) = postkit::free_space::volume_bytes(&dir)
        .map_err(|e| format!("Could not read free space: {e}"))?;
    Ok(DiskSpace {
        free_bytes: free,
        total_bytes: total,
        percent_free: if total == 0 {
            0.0
        } else {
            free as f64 * 100.0 / total as f64
        },
    })
}

/// Give a built DCP a new content title without re-encoding: the CPL is
/// rewritten with a new composition id and the essence is left alone. The folder
/// is renamed too when it is still named after the old title. Returns the
/// package path, which changes when the folder is renamed.
#[tauri::command]
pub async fn retitle_dcp(path: String, title: String) -> Result<String, String> {
    let dir = PathBuf::from(&path);
    if !holds_dcp(&dir) {
        return Err(format!("{path} does not hold a DCP"));
    }
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("Enter a new title".into());
    }
    let old_title = dcpwizard_core::multi_cpl::list_cpls(&dir)
        .first()
        .map(|cpl| cpl.content_title.clone())
        .ok_or_else(|| format!("No CPL found in {path}"))?;

    let config = dcpwizard_core::edit::EditConfig {
        input: dir.clone(),
        title: Some(title.clone()),
        ..Default::default()
    };
    if dcpwizard_core::edit::edit_dcp(&config) != 0 {
        return Err(format!(
            "Could not retitle {path}. Encrypted packages are refused: every KDM is bound to the CPL id, and a retitle mints a new one."
        ));
    }

    let folder_is_named_after_the_title =
        dir.file_name().and_then(|n| n.to_str()) == Some(&old_title);
    let title_works_as_a_folder_name =
        !title.contains(std::path::MAIN_SEPARATOR) && !title.contains('/');
    if !folder_is_named_after_the_title || !title_works_as_a_folder_name {
        return Ok(path);
    }
    let renamed = dir.with_file_name(&title);
    if renamed.exists() {
        return Ok(path);
    }
    std::fs::rename(&dir, &renamed)
        .map_err(|e| format!("Retitled, but could not rename the folder: {e}"))?;
    Ok(renamed.to_string_lossy().into_owned())
}

/// Delete a built DCP folder and everything in it. Refuses any folder that is
/// not a DCP, so a stale recent entry cannot take out a folder of source media.
#[tauri::command]
pub async fn delete_dcp(app: AppHandle, path: String) -> Result<(), String> {
    let dir = PathBuf::from(&path);
    if !dir.exists() {
        return Err(format!("{path} no longer exists"));
    }
    if !holds_dcp(&dir) {
        return Err(format!("{path} does not hold a DCP, refusing to delete it"));
    }
    let queue = app.state::<JobQueue>();
    if queue.is_building_into(&dir) {
        return Err(format!(
            "A build is writing into {path}. Cancel it before deleting."
        ));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("Could not delete {path}: {e}"))
}

#[tauri::command]
pub async fn cancel_job(app: AppHandle, job_id: u64) -> Result<(), String> {
    app.state::<JobQueue>().cancel(job_id);
    Ok(())
}

#[tauri::command]
pub async fn pause_job(app: AppHandle) -> Result<(), String> {
    app.state::<JobQueue>().pause();
    Ok(())
}

#[tauri::command]
pub async fn resume_job(app: AppHandle) -> Result<(), String> {
    app.state::<JobQueue>().resume();
    Ok(())
}

#[tauri::command]
pub async fn list_jobs(app: AppHandle) -> Vec<postkit::gui_job_queue::JobInfo> {
    app.state::<JobQueue>().snapshot()
}

// ─── Version File (supplemental DCP) ───────────────────────────────────────

// One reel replacement from the GUI. Empty strings mean "reference the OV".
#[derive(Deserialize)]
pub struct VfReplacementInput {
    reel_number: u32,
    picture: Option<String>,
    sound: Option<String>,
}

#[tauri::command]
pub async fn create_vf(
    ov_dir: String,
    output_dir: String,
    title: Option<String>,
    replacements: Vec<VfReplacementInput>,
) -> Result<String, String> {
    let path_opt = |s: Option<String>| s.filter(|p| !p.is_empty()).map(PathBuf::from);
    let replacement_reels: Vec<dcpwizard_core::vf::ReplacementReel> = replacements
        .into_iter()
        .map(|r| dcpwizard_core::vf::ReplacementReel {
            reel_number: r.reel_number,
            picture: path_opt(r.picture),
            sound: path_opt(r.sound),
            subtitle: None,
            ccap: None,
        })
        .collect();

    if !replacement_reels
        .iter()
        .any(|r| r.picture.is_some() || r.sound.is_some())
    {
        return Err("Add at least one replacement reel with a picture or sound".into());
    }

    let config = dcpwizard_core::vf::VfConfig {
        ov_dir: PathBuf::from(&ov_dir),
        vf_dir: PathBuf::from(&output_dir),
        title: title.unwrap_or_default(),
        replacement_reels,
        subtitle_language: String::new(),
        subtitle_opts: dcpwizard_core::subtitle::SubtitleOptions::default(),
        signer: None,
    };

    // create_vf does blocking IO (mxf wrap, hashing), keep it off the async runtime.
    let code = tokio::task::spawn_blocking(move || dcpwizard_core::vf::create_vf(&config))
        .await
        .map_err(|e| format!("VF task panicked: {e}"))?;

    if code == 0 {
        Ok(format!("Created Version File DCP at {output_dir}"))
    } else {
        Err(format!(
            "VF creation failed (rc={code}); see log for details"
        ))
    }
}

// ─── Queue worker ──────────────────────────────────────────────────────────

async fn run_queue_worker(app: AppHandle) {
    loop {
        let job = app.state::<JobQueue>().take_next();

        let Some(job) = job else {
            app.state::<JobQueue>().clear_current();
            break;
        };

        app.state::<JobQueue>().start(&job);

        let result = tokio::task::spawn_blocking({
            let app = app.clone();
            let job = job.clone();
            move || run_job(&app, &job)
        })
        .await;

        let queue = app.state::<JobQueue>();
        match result {
            Ok(Ok(_)) => {
                queue.finish(&job, postkit::gui_job_queue::StoredJobState::Done, "");
                emit_progress(&app, job.id, "done", "Complete", 0, 0, 0.0, 0.0, 100.0);
            }
            Ok(Err(e)) => {
                let cancelled = queue.is_cancelled();
                let state = if cancelled {
                    postkit::gui_job_queue::StoredJobState::Cancelled
                } else {
                    postkit::gui_job_queue::StoredJobState::Failed
                };
                queue.finish(&job, state, &e);
                let stage = if cancelled { "cancelled" } else { "error" };
                emit_progress(&app, job.id, stage, &e, 0, 0, 0.0, 0.0, 0.0);
            }
            // a panic leaves no error event, so the panel would wait forever
            Err(e) => {
                queue.finish(
                    &job,
                    postkit::gui_job_queue::StoredJobState::Failed,
                    &format!("Build panicked: {e}"),
                );
                emit_progress(
                    &app,
                    job.id,
                    "error",
                    &format!("Build panicked: {e}"),
                    0,
                    0,
                    0.0,
                    0.0,
                    0.0,
                );
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

// ─── Job execution ─────────────────────────────────────────────────────────

fn log_to(log_file: &Arc<Mutex<Option<std::fs::File>>>, msg: &str) {
    eprintln!("[pipeline] {msg}");
    if let Some(f) = log_file.lock().unwrap().as_mut() {
        let _ = writeln!(f, "{msg}");
    }
}

const SECONDS_PER_MINUTE: u64 = 60;

/// One `[TIMING]` line for the job log, sitting alongside the `[ENCODE]` and
/// `[PACKAGE]` lines the same stage writes.
fn format_stage_timing(stage: &str, duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    format!(
        "[TIMING] {stage} took {}m{}s",
        seconds / SECONDS_PER_MINUTE,
        seconds % SECONDS_PER_MINUTE
    )
}

/// The `[TIMING]` line naming where the time inside an encode went, or None
/// when nothing was measured, which is a still or a J2K sequence.
fn format_encode_breakdown(progress: &postkit::pipeline::PipelineProgress) -> Option<String> {
    let measured = progress.decode_wait_secs > 0.0
        || progress.prepare_secs > 0.0
        || progress.encode_secs > 0.0
        || progress.write_secs > 0.0;
    measured.then(|| format!("[TIMING] encode breakdown: {}", progress.phase_breakdown()))
}

fn parse_audio_input_order(
    value: Option<&str>,
) -> Result<dcpwizard_core::mxf_wrap::AudioInputOrder, String> {
    match value.unwrap_or("dcp") {
        "dcp" => Ok(dcpwizard_core::mxf_wrap::AudioInputOrder::Canonical51),
        "lrc-ls-rs-lfe" => Ok(dcpwizard_core::mxf_wrap::AudioInputOrder::LrcLsRsLfe),
        other => Err(format!("Unknown audio input order: {other}")),
    }
}

/// Panel resolution keys and the CPL container they stand for. scope/flat/full
/// are distinct containers, not just 2K vs 4K.
const RESOLUTION_CONTAINERS: [(&str, u32, u32); 6] = [
    ("2k-scope", 2048, 858),
    ("2k-flat", 1998, 1080),
    ("2k-full", 2048, 1080),
    ("4k-scope", 4096, 1716),
    ("4k-flat", 3996, 2160),
    ("4k-full", 4096, 2160),
];

/// What `container_of` answers for a resolution key that names no container,
/// which is the panel's "Auto (from source)".
const NO_CONTAINER: (u32, u32) = (0, 0);

/// The DCI rasters a fitted picture is centred on. The container is masked out
/// of one of these, so a scope package is 2048x858 inside a 2048x1080 frame.
const TWO_K_RASTER: (u32, u32) = (2048, 1080);
const FOUR_K_RASTER: (u32, u32) = (4096, 2160);

fn container_of(resolution: &str) -> (u32, u32) {
    RESOLUTION_CONTAINERS
        .iter()
        .find(|(key, _, _)| *key == resolution)
        .map(|(_, width, height)| (*width, *height))
        .unwrap_or(NO_CONTAINER)
}

/// The rasters a job's picture has to land on. Choosing a container in the panel
/// is choosing the DCI raster around it, so the picture is always scaled onto
/// that raster; the container fill only decides whether the source is cropped to
/// the container's aspect or letterboxed inside it.
fn job_geometry(job: &JobConfig) -> dcpwizard_core::source_picture::EncodeGeometry {
    geometry_for_resolution(&job.resolution)
}

fn geometry_for_resolution(resolution: &str) -> dcpwizard_core::source_picture::EncodeGeometry {
    let container = container_of(resolution);
    let raster = if resolution.contains("4k") {
        FOUR_K_RASTER
    } else {
        TWO_K_RASTER
    };
    dcpwizard_core::source_picture::EncodeGeometry {
        forced_raster: (container != NO_CONTAINER).then_some(raster),
        container: (container != NO_CONTAINER).then_some(container),
    }
}

/// The picture processing a job asks for, planned against the source it decodes.
fn resolve_job_picture(
    job: &JobConfig,
    source: &std::path::Path,
) -> Result<dcpwizard_core::source_picture::ResolvedPicture, String> {
    let info = dcpwizard_core::probe::probe_video(source)
        .ok_or_else(|| format!("cannot read the size of {}", source.display()))?;
    dcpwizard_core::source_picture::resolve_picture(
        &job.picture,
        source,
        info.width,
        info.height,
        &job_geometry(job),
        postkit::encode::detect_input_type(source) == postkit::encode::InputType::ImageSequence,
    )
}

fn resolution_key_of(width: u32, height: u32) -> Option<&'static str> {
    RESOLUTION_CONTAINERS
        .iter()
        .find(|(_, w, h)| *w == width && *h == height)
        .map(|(key, _, _)| *key)
}

fn parse_upmixer(value: Option<&str>) -> Result<Option<postkit::upmix::Upmixer>, String> {
    match value.unwrap_or("") {
        "" | "none" => Ok(None),
        "a" => Ok(Some(postkit::upmix::Upmixer::A)),
        "b" => Ok(Some(postkit::upmix::Upmixer::B)),
        other => Err(format!("Unknown upmix variant '{other}' (use a or b)")),
    }
}

/// The create panel's HDR controls, before validation.
struct HdrPanelOptions {
    dci: bool,
    lut: Option<String>,
    already_pq: bool,
    allow_generic_tonemap: bool,
}

/// Resolve the HDR controls into the source colour the encoder gets, rejecting
/// the combinations the CLI rejects before an encode starts. A DCI HDR package
/// only leaves here with a source that never sees the X'Y'Z' transform, so PQ
/// signaling can never end up over transformed frames.
fn resolve_hdr(
    panel: &HdrPanelOptions,
    frame_rate: u32,
    bandwidth: u32,
    stereoscopic: bool,
    splits_reels: bool,
    versions: bool,
) -> Result<postkit::encode::SourceColour, String> {
    use dcpwizard_core::hdr;
    use postkit::encode::SourceColour;

    let lut = panel
        .lut
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let source_paths = [lut.is_some(), panel.already_pq, panel.allow_generic_tonemap]
        .into_iter()
        .filter(|set| *set)
        .count();
    if source_paths > 1 {
        return Err(
            "Choose one HDR source path: an HDR-to-DCI LUT, an already-PQ source, or generic tone mapping".into(),
        );
    }

    let source_colour = match (lut, panel.already_pq) {
        (Some(path), _) => {
            let path = PathBuf::from(path);
            if !path.is_file() {
                return Err(format!("HDR-to-DCI LUT not found: {}", path.display()));
            }
            SourceColour::DciLut(path)
        }
        (None, true) => SourceColour::AlreadyPq,
        (None, false) => SourceColour::DisplayRgb,
    };

    if !panel.dci {
        return Ok(source_colour);
    }
    if source_colour == SourceColour::DisplayRgb {
        return Err(
            "DCI HDR needs the source path to PQ: choose an HDR-to-DCI LUT or mark the source already PQ".into(),
        );
    }
    if bandwidth > hdr::HDR_MAX_MBPS {
        return Err(format!(
            "DCI HDR caps the codestream at {} bytes/frame ({} Mbit/s at {frame_rate} fps): lower the bandwidth from {bandwidth}",
            hdr::hdr_codestream_byte_cap(frame_rate),
            hdr::HDR_MAX_MBPS
        ));
    }
    if stereoscopic {
        return Err("DCI HDR is not supported for stereoscopic (3D) DCPs".into());
    }
    if splits_reels {
        return Err("DCI HDR is not supported with reel splitting".into());
    }
    if versions {
        return Err("DCI HDR is not supported with a versions manifest".into());
    }
    Ok(source_colour)
}

/// What a detected HDR source needs before the J2K encode.
#[derive(Debug, PartialEq, Eq)]
enum HdrSourceStep {
    EncodeDirectly,
    TonemapToSdr,
}

/// Mirrors the CLI: an HDR source reaches the encoder through its HDR-to-DCI LUT
/// or as already-PQ essence, and anything else needs the generic tone map
/// opt-in. The tone map lands on SDR, so it is unreachable for a PQ source.
fn plan_hdr_source(
    hdr_type: postkit::dolby_vision::HdrType,
    source_colour: &postkit::encode::SourceColour,
    allow_generic_tonemap: bool,
) -> Result<HdrSourceStep, String> {
    use postkit::dolby_vision::HdrType;
    use postkit::encode::SourceColour;

    // a wide-gamut source is display RGB too, so an HDR file labelled that way
    // still has to name how it reaches DCI
    let display_rgb = matches!(
        source_colour,
        SourceColour::DisplayRgb | SourceColour::DisplayRgbIn(_)
    );
    if hdr_type == HdrType::Sdr || !display_rgb {
        return Ok(HdrSourceStep::EncodeDirectly);
    }
    if allow_generic_tonemap {
        return Ok(HdrSourceStep::TonemapToSdr);
    }
    Err(format!(
        "Source is {hdr_type:?}: choose an HDR-to-DCI LUT, mark it already PQ, or allow generic tone mapping"
    ))
}

/// Hand the encoder a source it can compress honestly: the file itself, or the
/// generic tone map's SDR conversion when the panel opted into it.
fn prepare_hdr_input(
    source: &std::path::Path,
    job: &JobConfig,
    tonemapped: PathBuf,
    log: impl Fn(&str),
) -> Result<PathBuf, String> {
    let hdr_type = dcpwizard_core::dolby_vision::detect_hdr_type(source);
    match plan_hdr_source(hdr_type, &job.source_colour, job.allow_generic_hdr_tonemap)? {
        HdrSourceStep::EncodeDirectly => Ok(source.to_path_buf()),
        HdrSourceStep::TonemapToSdr => {
            log(&format!(
                "[ENCODE] Generic FFmpeg tone map of a {hdr_type:?} source. It is not a delivery transform."
            ));
            if let Some(parent) = tonemapped.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let code = dcpwizard_core::dolby_vision::convert_hdr(
                source,
                postkit::dolby_vision::HdrType::Sdr,
                &tonemapped,
            );
            if code != 0 {
                return Err(format!("HDR tone mapping failed (rc={code})"));
            }
            Ok(tonemapped)
        }
    }
}

fn parse_split_timecodes(spec: &str, fps: u32) -> Result<Vec<u64>, String> {
    let mut frames = Vec::new();
    for timecode in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        frames.push(dcpwizard_core::reel::parse_timecode(timecode, fps)?);
    }
    if frames.is_empty() {
        return Err("Split timecodes need at least one HH:MM:SS or HH:MM:SS:FF entry".into());
    }
    Ok(frames)
}

/// Read and validate a versions manifest. An absent or empty path keeps the
/// single-CPL path.
fn load_version_specs(
    path: Option<&str>,
) -> Result<Vec<dcpwizard_core::versions::VersionSpec>, String> {
    match path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => dcpwizard_core::versions::load_versions(std::path::Path::new(p)),
        None => Ok(Vec::new()),
    }
}

/// Reel boundaries in frames: the panel's timecodes, or the source's chapter
/// marks when the panel asked to split at chapters.
fn resolve_reel_splits(job: &JobConfig, fps: u32) -> Result<Vec<u64>, String> {
    if !job.split_chapters {
        return Ok(job.reel_split_frames.clone());
    }
    let probe = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_chapters"])
        .arg(&job.video_path)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    if !probe.status.success() {
        return Err("ffprobe failed to read chapters".into());
    }
    dcpwizard_core::reel::parse_chapter_starts(&String::from_utf8_lossy(&probe.stdout), fps)
}

// Frame rate drives both the J2K encode (video demux rate) and the CPL.
fn frame_rate_of(framerate: &str) -> (u32, u32) {
    let numerator = FRAME_RATES
        .iter()
        .find(|(key, _)| *key == framerate)
        .map(|(_, rate)| *rate)
        .unwrap_or(DEFAULT_FRAME_RATE.1);
    (numerator, FRAME_RATE_DENOMINATOR)
}

fn count_frames(j2k_dir: &std::path::Path) -> u64 {
    std::fs::read_dir(j2k_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .count() as u64
        })
        .unwrap_or(0)
}

/// Create-time audio processing: filename channel routing from a directory of
/// mono WAVs, then stereo-to-5.1 upmix, then the picture/sound delay, then
/// loudness normalization, then the 23.976-to-24 pull-up the conform asks for.
/// Same order as the CLI create path. Intermediates go under `<output>/audio_work`.
fn prepare_audio(
    job: &JobConfig,
    conform: dcpwizard_core::hfr::SourceConform,
    output: &std::path::Path,
    log: impl Fn(&str),
) -> Result<Option<PathBuf>, String> {
    let work_dir = output.join("audio_work");
    let mut audio_path = job
        .audio_path
        .as_ref()
        .filter(|a| !a.is_empty())
        .map(PathBuf::from);

    // no sound named: the picture source's own track is the sound, and enters
    // here so everything below applies to it as it would to a named WAV
    if audio_path.is_none() && job.audio_channel_dir.is_none() {
        log("[AUDIO] No sound file named: extracting the source's own audio");
        if let Some(extracted) =
            dcpwizard_core::audio_fallback::extract_embedded_audio(&job.video_path, &work_dir)?
        {
            log("[AUDIO] Using the source's own audio");
            audio_path = Some(extracted);
        }
    }

    // the map places every channel by hand, so it runs before anything that
    // moves channels for it
    if let (Some(spec), Some(input)) = (job.audio_map.as_deref(), &audio_path) {
        std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
        let mapped = work_dir.join("mapped.wav");
        let applied = dcpwizard_core::audio_map::apply_audio_map(spec, input, &mapped)?;
        log(&format!(
            "[AUDIO] Map: {} channels to {} over {} frames{}",
            applied.report.input_channels,
            applied.report.output_channels,
            applied.report.frames,
            if applied.pure_routing {
                ", bit-exact routing"
            } else {
                ""
            }
        ));
        if applied.report.clipped_samples > 0 {
            log(&format!(
                "[AUDIO] Map clipped {} sample(s): lower the cell gains",
                applied.report.clipped_samples
            ));
        }
        audio_path = Some(mapped);
    }

    if let Some(dir) = job.audio_channel_dir.as_deref() {
        std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
        let routed = work_dir.join("routed.wav");
        audio_path = Some(dcpwizard_core::audio_route::route_directory(
            std::path::Path::new(dir),
            &routed,
        )?);
        log("[AUDIO] Routed channel WAVs from the input directory by filename");
    }

    if let (Some(variant), Some(input)) = (job.upmix, &audio_path) {
        std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("upmix.wav");
        postkit::upmix::upmix_wav(variant, input, &out).map_err(|e| e.to_string())?;
        log("[AUDIO] Upmixed stereo to 5.1");
        audio_path = Some(out);
    }

    if let (true, Some(input)) = (job.audio_delay_ms != 0, &audio_path) {
        std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("delayed.wav");
        audio_path = Some(dcpwizard_core::audio_adjust::apply_delay(
            input,
            &out,
            job.audio_delay_ms,
        )?);
        log(&format!(
            "[AUDIO] Delayed the sound by {}ms against the picture",
            job.audio_delay_ms
        ));
    }

    if let (Some(spec), Some(input)) = (job.loudness_target.as_deref(), &audio_path) {
        let target = dcpwizard_core::loudness::parse_loudness_target(spec)?;
        let ceiling = job
            .true_peak_ceiling
            .unwrap_or(dcpwizard_core::loudness::DEFAULT_TRUE_PEAK_CEILING_DBTP);
        std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("loudness.wav");
        let plan = dcpwizard_core::loudness::adjust_loudness(input, &out, target, ceiling)
            .map_err(|e| e.to_string())?;
        log(&format!(
            "[AUDIO] loudness {:.1} -> {:.1} dB (gain {:+.2} dB, peak {:.2} dBTP)",
            plan.measured_db, plan.target_db, plan.gain_db, plan.resulting_true_peak_dbtp
        ));
        audio_path = Some(out);
    }

    if let (true, Some(input)) = (conform.audio_pull_up, &audio_path) {
        std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
        let out = work_dir.join("pullup.wav");
        dcpwizard_core::hfr::audio_pull_up(input, &out)?;
        log("[AUDIO] Applied 23.976-to-24 audio pull-up");
        audio_path = Some(out);
    }

    Ok(audio_path)
}

/// The burn a job asks for, rebuilt from the cue file. `submit_job` already
/// proved the file parses, so a failure here is a file that changed underneath.
fn job_subtitle_burn(
    job: &JobConfig,
    fps: postkit::encode::FrameRate,
) -> Result<Option<std::sync::Arc<postkit::subtitle_raster::SubtitleBurn>>, String> {
    let Some(path) = job.burn_subtitle.as_deref() else {
        return Ok(None);
    };
    dcpwizard_core::subtitle::prepare_subtitle_burn(
        Path::new(path),
        job.burn_subtitle_font.as_deref().map(Path::new),
        fps,
        &job.burn_style,
    )
    .map(Some)
}

/// Build a still's frame directory: one encode, linked for every frame of the
/// hold. Shaped as an [`postkit::pipeline::EncodeResult`] so the rest of the job
/// does not care which kind of input it got.
fn encode_still(
    job: &JobConfig,
    output: &std::path::Path,
    fps: postkit::encode::FrameRate,
    picture: &dcpwizard_core::source_picture::ResolvedPicture,
    log: impl Fn(&str),
) -> Result<postkit::pipeline::EncodeResult, String> {
    let started = std::time::Instant::now();
    let j2k_dir = output.join("j2k");
    let route = dcpwizard_core::encode::xyz_route(job.source_colourspace)?;
    postkit::still::build_still_frames(&postkit::still::StillHold {
        image: &job.video_path,
        frames: job.still_length_frames,
        fps,
        width: picture.encode_width,
        height: picture.encode_height,
        filters: &picture.plan.filters,
        apply_xyz_transform: route.compressor_transform(),
        rsiz: postkit::encode::default_rsiz(),
        colour_transform: route.frame_transform()?,
        burn: job_subtitle_burn(job, fps)?,
        out_dir: &j2k_dir,
    })?;
    log(&format!(
        "[ENCODE] Still held for {} frame(s) at {}x{}",
        job.still_length_frames, picture.encode_width, picture.encode_height
    ));
    Ok(postkit::pipeline::EncodeResult {
        j2k_dir,
        frames_encoded: job.still_length_frames,
        elapsed_secs: started.elapsed().as_secs_f64(),
        picture_findings: postkit::picture_findings::PictureFindings::default(),
    })
}

/// The window the encode is given so only the kept frames are ever compressed,
/// or None when nothing is trimmed, the picture is not encoded here, or the
/// source was never probed for a length.
fn job_encode_window(
    job: &JobConfig,
    encode_input: &std::path::Path,
) -> Result<Option<postkit::encode::FrameRange>, String> {
    if job.trim_start_frames + job.trim_end_frames == 0 {
        return Ok(None);
    }
    // without a probed length the kept frames are counted off the encode, the
    // way they always were
    let Some(total) = job
        .source
        .as_ref()
        .map(|info| u64::from(info.total_frames))
        .filter(|total| *total > 0)
    else {
        return Ok(None);
    };
    let kept =
        dcpwizard_core::trim::kept_frames(total, job.trim_start_frames, job.trim_end_frames)?;
    Ok(dcpwizard_core::trim::encode_window(
        encode_input,
        job.trim_start_frames,
        kept,
    ))
}

/// Trim the sound to the kept window, and the encoded frames too when `window`
/// says the encoder was not given it. Hands both back unchanged when no trim was
/// asked for.
fn apply_trim(
    job: &JobConfig,
    j2k_dir: &std::path::Path,
    output: &std::path::Path,
    audio: Option<PathBuf>,
    fps: u32,
    window: Option<postkit::encode::FrameRange>,
) -> Result<(PathBuf, Option<PathBuf>), String> {
    if job.trim_start_frames + job.trim_end_frames == 0 {
        return Ok((j2k_dir.to_path_buf(), audio));
    }
    let (picture, kept) = match window {
        Some(window) => (j2k_dir.to_path_buf(), window.frame_count),
        None => {
            let trimmed = output.join("j2k_trimmed");
            let kept = dcpwizard_core::trim::link_trimmed_frames(
                j2k_dir,
                job.trim_start_frames,
                job.trim_end_frames,
                &trimmed,
            )?;
            (trimmed, kept)
        }
    };
    let audio = match audio {
        Some(input) => {
            let out = output.join("audio_work").join("trimmed.wav");
            std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
            dcpwizard_core::trim::trim_wav(&input, job.trim_start_frames, kept, fps, &out)?;
            Some(out)
        }
        None => None,
    };
    Ok((picture, audio))
}

fn build_dcp_config(
    job: &JobConfig,
    j2k_dir: PathBuf,
    right_eye_dir: Option<PathBuf>,
    audio_path: Option<PathBuf>,
    sign_language_main_channels: Option<u32>,
    reel_split_frames: Vec<u64>,
) -> dcpwizard_core::dcp::DcpConfig {
    let standard = standard_of(&job.standard);
    let (container_width, container_height) = container_of(&job.resolution);
    let resolution = if container_width == 0 && container_height == 0 {
        dcpwizard_core::cpl::picture_geometry(&j2k_dir, 0, 0)
            .map(|geometry| resolution_of_raster(geometry.stored_width, geometry.stored_height))
            .unwrap_or_else(|_| resolution_of(&job.resolution))
    } else {
        resolution_of(&job.resolution)
    };
    let content_type = content_type_of(&job.content_kind);

    let (frame_rate_num, frame_rate_den) = frame_rate_of(&job.framerate);

    // after a trim, j2k_dir holds exactly the frames that survived it, which is
    // the window the timed text has to be clamped to
    let kept_after_trim = if job.trim_start_frames + job.trim_end_frames > 0 {
        dcpwizard_core::trim::frame_count(&j2k_dir)
    } else {
        0
    };

    dcpwizard_core::dcp::DcpConfig {
        title: job.title.clone(),
        standard,
        resolution,
        container_width,
        container_height,
        content_type,
        output_dir: job.output_dir.clone(),
        frame_rate_num,
        frame_rate_den,
        max_bitrate_mbps: job.bandwidth,
        encrypt: job.encrypt,
        key_out: job.key_out.as_ref().map(PathBuf::from),
        stereo_3d: right_eye_dir.is_some(),
        right_eye_dir,
        j2k_dir: Some(j2k_dir),
        audio_path,
        audio_input_order: job.audio_input_order,
        audio_channels: job.audio_channels,
        atmos_path: job.atmos.as_ref().map(PathBuf::from),
        subtitle_path: job.subtitle.as_ref().map(PathBuf::from),
        subtitle_language: job.subtitle_language.clone(),
        subtitle_opts: dcpwizard_core::subtitle::SubtitleOptions {
            appearance: job.subtitle_appearance.clone(),
            ..Default::default()
        },
        ccap_path: job.ccap.as_ref().map(PathBuf::from),
        ccap_language: job.ccap_language.clone(),
        pad_head: job.pad_head.clone(),
        pad_tail: job.pad_tail.clone(),
        pad_color: job.pad_color.clone(),
        source_trim: dcpwizard_core::subtitle::SourceTrim {
            start_frames: job.trim_start_frames,
            kept_frames: kept_after_trim,
        },
        reel_length_minutes: job.reel_length_minutes,
        reel_split_frames,
        sign_language_lang: job.sign_language_tag.clone(),
        sign_language_main_channels,
        hdr_dci: job.hdr_dci,
        facility: job.facility.clone(),
        audio_language: job
            .naming
            .audio_language
            .clone()
            .filter(|tag| !tag.is_empty()),
        ratings: ratings_of(&job.naming.ratings),
        content_versions: content_versions_of(job.naming.content_versions.as_deref()),
        head_items: job.head_items.clone(),
        tail_items: job.tail_items.clone(),
        ..Default::default()
    }
}

fn run_job(app: &AppHandle, job: &JobConfig) -> Result<String, String> {
    let job_started = Instant::now();
    let queue = app.state::<JobQueue>();
    let cancel = queue.cancel_flag();
    let pause = queue.pause_flag();

    let output = &job.output_dir;
    let log_path = output.join("dcpwizard.log");
    let log_file: Arc<Mutex<Option<std::fs::File>>> =
        Arc::new(Mutex::new(std::fs::File::create(&log_path).ok()));

    log_to(&log_file, "=== DCP Wizard Pipeline ===");
    log_to(&log_file, &format!("Job ID: {}", job.id));
    log_to(&log_file, &format!("Title: {}", job.title));
    log_to(&log_file, &format!("Input: {}", job.video_path.display()));
    log_to(&log_file, &format!("Output: {}", output.display()));
    log_to(
        &log_file,
        &format!(
            "Started: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        ),
    );

    let (fps_num, fps_den) = frame_rate_of(&job.framerate);
    let encode_fps = postkit::encode::FrameRate::new(fps_num, fps_den);
    let conform = job
        .source
        .as_ref()
        .map(|info| dcpwizard_core::hfr::conform_source_to_dcp(info.fps_num, info.fps_den, fps_num))
        .unwrap_or_default();
    let preflight_started = Instant::now();

    // reel boundaries before the encode: a source with no chapter marks should
    // fail now, not after an hour of J2K.
    let reel_split_frames = resolve_reel_splits(job, fps_num)?;
    if !reel_split_frames.is_empty() {
        log_to(
            &log_file,
            &format!("[PACKAGE] Reel boundaries: {reel_split_frames:?}"),
        );
    }

    // Map the target bandwidth (Mbps) to a J2K compression ratio. Only honoured
    // for video input; image/J2K sequences fall back to the encoder default. A 3D
    // job encodes both eyes with this ratio, so the halving is part of it.
    let compression_ratio = job
        .source
        .as_ref()
        .map(|info| {
            dcpwizard_core::encode::video_compression_ratio(
                info.width,
                info.height,
                fps_num,
                Some(job.bandwidth),
                job.right_eye.is_some(),
            )
        })
        .unwrap_or(dcpwizard_core::encode::DEFAULT_COMPRESSION_RATIO);

    // HDR source handling before the encode: the LUT and already-PQ paths reach
    // the encoder untransformed, everything else needs the tone map opt-in.
    let encode_input = prepare_hdr_input(
        &job.video_path,
        job,
        output.join("hdr_tonemap.mov"),
        |msg| log_to(&log_file, msg),
    )?;

    // the crop and the fit are planned before the encode, so a source they
    // cannot land on the raster fails now rather than after an hour of J2K
    let resolved_picture = resolve_job_picture(job, &encode_input)?;
    log_to(
        &log_file,
        &format!("[ENCODE] Picture: {}", resolved_picture.plan.describe()),
    );
    for hint in &job.hints {
        log_to(&log_file, &format!("[HINT] {hint}"));
    }

    log_to(
        &log_file,
        &format_stage_timing("preflight", preflight_started.elapsed()),
    );

    // only the kept frames are compressed, so a trim never pays for the encoder
    // time it then throws away. Both eyes take the same window.
    let encode_window = job_encode_window(job, &encode_input)?;
    if let Some(window) = encode_window {
        log_to(
            &log_file,
            &format!(
                "[ENCODE] Encoding frames {}..{} of the source",
                window.first_frame,
                window.end_frame()
            ),
        );
    }

    let dci_codestream_byte_cap = if job.hdr_dci {
        dcpwizard_core::hdr::hdr_codestream_byte_cap(fps_num)
    } else {
        postkit::j2k::dci_codestream_byte_cap(fps_num)
    };
    // under a PSNR target the bandwidth is a ceiling per frame rather than what
    // the allocation aims at
    let codestream_byte_cap = match job.quality_psnr {
        Some(_) => dci_codestream_byte_cap.min(dcpwizard_core::encode::video_codestream_byte_cap(
            fps_num,
            job.bandwidth,
            job.right_eye.is_some(),
        )),
        None => dci_codestream_byte_cap,
    };
    if let Some(db) = job.quality_psnr {
        log_to(
            &log_file,
            &format!(
                "[ENCODE] PSNR {db} dB (bandwidth {} Mbit/s, at most {codestream_byte_cap} bytes a frame)",
                job.bandwidth
            ),
        );
    }

    let encode_options = postkit::pipeline::EncodeRunOptions {
        compression_ratio,
        quality_psnr: job.quality_psnr,
        fps: encode_fps,
        read_source_at: conform.read_source_at,
        frame_range: encode_window,
        source_colour: job.source_colour.clone(),
        codestream_byte_cap: Some(codestream_byte_cap),
        subtitle_burn: job_subtitle_burn(job, encode_fps)?,
        picture: resolved_picture.processing.clone(),
        rsiz: postkit::encode::default_rsiz(),
    };

    // the picture MXF is written as the frames finish where the job allows it, so
    // packaging no longer reads the whole J2K directory back to wrap it
    let overlap_refusal = dcpwizard_core::overlapped_picture::overlap_refusal(
        &dcpwizard_core::overlapped_picture::PictureSource {
            input_type: postkit::encode::detect_input_type(&encode_input),
            still_hold: job.still_length_frames > 0,
        },
        &dcpwizard_core::overlapped_picture::PackageShape {
            stereoscopic: job.right_eye.is_some(),
            pads: job.pad_head.is_some() || job.pad_tail.is_some(),
            splits_reels: job.reel_length_minutes > 0 || !reel_split_frames.is_empty(),
            multiple_versions: !job.versions.is_empty(),
            encrypts: job.encrypt,
        },
    );
    let wrap_target = match overlap_refusal {
        Some(reason) => {
            log_to(
                &log_file,
                &format!("[ENCODE] Wrapping the picture MXF after the encode: {reason}"),
            );
            None
        }
        None => Some(dcpwizard_core::overlapped_picture::PictureWrapTarget {
            dcp_dir: job.output_dir.clone(),
            fps: fps_num,
            hdr_dci: job.hdr_dci,
        }),
    };

    // Encode using shared pipeline. A still never reaches it: it is one encode
    // whose codestream is linked for every frame of the hold.
    let job_id = job.id;
    let app_ref = app.clone();
    let log_ref = log_file.clone();
    let encode_breakdown: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let encode_breakdown_ref = encode_breakdown.clone();
    let encode_started = Instant::now();
    let mut picture_mxf = None;
    let encode_result = if job.still_length_frames > 0 {
        encode_still(job, output, encode_fps, &resolved_picture, |msg| {
            log_to(&log_ref, msg)
        })?
    } else {
        let on_progress = |p: &postkit::pipeline::PipelineProgress| {
            emit_progress(
                &app_ref,
                job_id,
                &p.stage,
                &p.message,
                p.frame,
                p.total_frames,
                p.fps,
                p.elapsed_secs,
                p.percent,
            );
            if let Some(line) = format_encode_breakdown(p) {
                *encode_breakdown_ref.lock().unwrap() = Some(line);
            }
        };
        let on_log = |msg: &str| log_to(&log_ref, msg);
        match wrap_target {
            Some(target) => {
                let (encode, wrapped) =
                    dcpwizard_core::overlapped_picture::encode_and_wrap_picture(
                        &encode_input,
                        output,
                        &encode_options,
                        target,
                        &cancel,
                        &pause,
                        on_progress,
                        on_log,
                    )?;
                log_to(
                    &log_file,
                    &format!(
                        "[ENCODE] Picture MXF written during the encode: {} ({} frames)",
                        wrapped.mxf_name(),
                        wrapped.duration
                    ),
                );
                picture_mxf = Some(wrapped);
                encode
            }
            None => postkit::pipeline::run_encode_with_options(
                &encode_input,
                output,
                &encode_options,
                &cancel,
                &pause,
                on_progress,
                on_log,
            )?,
        }
    };
    for finding in encode_result.picture_findings.describe(encode_fps.as_f64()) {
        log_to(&log_file, &format!("[ENCODE] {finding}"));
    }

    // Stereoscopic 3D: encode the right eye into its own subdir at the same
    // ratio/fps (the main input is the left eye).
    let right_eye_dir = if let Some(re) = job.right_eye.as_deref() {
        log_to(&log_file, &format!("[ENCODE] Right eye: {re}"));
        let re_out = output.join("right");
        let re_input = prepare_hdr_input(
            std::path::Path::new(re),
            job,
            re_out.join("hdr_tonemap.mov"),
            |msg| log_to(&log_file, msg),
        )?;
        let log_ref = log_file.clone();
        let re_result = postkit::pipeline::run_encode_with_options(
            &re_input,
            &re_out,
            &encode_options,
            &cancel,
            &pause,
            |_p| {},
            |msg| log_to(&log_ref, msg),
        )?;
        for finding in re_result.picture_findings.describe(encode_fps.as_f64()) {
            log_to(&log_file, &format!("[ENCODE] right eye: {finding}"));
        }
        Some(re_result.j2k_dir)
    } else {
        None
    };

    log_to(
        &log_file,
        &format_stage_timing("encode", encode_started.elapsed()),
    );
    if let Some(breakdown) = encode_breakdown.lock().unwrap().as_deref() {
        log_to(&log_file, breakdown);
    }

    emit_progress(
        app,
        job.id,
        "audio",
        "Preparing the sound track...",
        0,
        0,
        0.0,
        0.0,
        99.0,
    );
    let audio_started = Instant::now();
    let audio_path = prepare_audio(job, conform, output, |msg| log_to(&log_file, msg))?;
    log_to(
        &log_file,
        &format_stage_timing("audio", audio_started.elapsed()),
    );

    // trim before sign language, which is packed to cover the picture the
    // package actually carries
    let (j2k_dir, audio_path) = apply_trim(
        job,
        &encode_result.j2k_dir,
        output,
        audio_path,
        fps_num,
        encode_window,
    )
    .map_err(|e| {
        log_to(&log_file, &format!("[TRIM] {e}"));
        e
    })?;

    // sign-language video (ISDCF Doc 13): pack VP9 onto channel 15, replacing
    // the sound track with the combined 16-channel WAV.
    let (audio_path, sign_language_main_channels) = match job.sign_language_video.as_deref() {
        Some(video) => {
            log_to(&log_file, &format!("[AUDIO] Sign language: {video}"));
            let frames = count_frames(&j2k_dir);
            let combined = output.join("slvs_sound.wav");
            let main_channels = dcpwizard_core::sign_language::build_slvs_sound(
                std::path::Path::new(video),
                audio_path.as_deref(),
                frames,
                fps_num,
                &combined,
            )?;
            (Some(combined), Some(main_channels))
        }
        None => (audio_path, None),
    };

    // Package DCP
    emit_progress(
        app,
        job.id,
        "package",
        "Creating DCP...",
        0,
        0,
        0.0,
        0.0,
        99.0,
    );
    log_to(&log_file, "[PACKAGE] Creating DCP...");
    let package_started = Instant::now();

    let mut config = build_dcp_config(
        job,
        j2k_dir,
        right_eye_dir,
        audio_path,
        sign_language_main_channels,
        reel_split_frames,
    );
    config.picture_mxf = picture_mxf;

    let rc = if job.versions.is_empty() {
        dcpwizard_core::dcp::create_dcp(&config)
    } else {
        log_to(
            &log_file,
            &format!(
                "[PACKAGE] {} versions over shared essence",
                job.versions.len()
            ),
        );
        dcpwizard_core::versions::create_versioned_dcp(&config, &job.versions)
    };
    if rc != 0 {
        log_to(&log_file, &format!("[PACKAGE] FAILED (rc={rc})"));
        return Err(format!(
            "DCP packaging failed (rc={rc}), see {}",
            log_path.display()
        ));
    }
    log_to(&log_file, "[PACKAGE] Done");
    log_to(
        &log_file,
        &format_stage_timing("package", package_started.elapsed()),
    );

    // Optional validation
    if job.validate {
        emit_progress(
            app,
            job.id,
            "validate",
            "Validating DCP...",
            0,
            0,
            0.0,
            0.0,
            99.5,
        );
        log_to(&log_file, "[VALIDATE] Running validation...");
        let validate_started = Instant::now();

        let result = dcpwizard_core::verify::verify_dcp(&job.output_dir);

        for err in &result.errors {
            log_to(&log_file, &format!("[VALIDATE] ERROR: {err}"));
        }
        for warn in &result.warnings {
            log_to(&log_file, &format!("[VALIDATE] WARNING: {warn}"));
        }

        let _ = app.emit(
            "validation-result",
            serde_json::json!({
                "job_id": job.id,
                "valid": result.valid,
                "errors": result.errors,
                "warnings": result.warnings,
                "info": result.info,
            }),
        );

        let summary = if result.valid {
            "DCP is valid ✓".to_string()
        } else {
            format!(
                "Validation: {} error(s), {} warning(s)",
                result.errors.len(),
                result.warnings.len()
            )
        };
        log_to(&log_file, &format!("[VALIDATE] {summary}"));
        log_to(
            &log_file,
            &format_stage_timing("validate", validate_started.elapsed()),
        );
        emit_progress(app, job.id, "validate", &summary, 0, 0, 0.0, 0.0, 100.0);
    }

    log_to(
        &log_file,
        &format_stage_timing("total", job_started.elapsed()),
    );

    log_to(
        &log_file,
        &format!(
            "=== Pipeline finished in {:.1}s ===",
            encode_result.elapsed_secs
        ),
    );
    Ok(format!("DCP created in {:.1}s", encode_result.elapsed_secs))
}

// ─── Helpers ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    app: &AppHandle,
    job_id: u64,
    stage: &str,
    message: &str,
    frame: u64,
    total_frames: u64,
    fps: f64,
    elapsed_secs: f64,
    percent: f64,
) {
    let _ = app.emit(
        "pipeline-progress",
        PipelineProgress {
            job_id,
            stage: stage.to_string(),
            message: message.to_string(),
            frame,
            total_frames,
            fps,
            elapsed_secs,
            percent,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcpwizard_core::hfr::SourceConform;
    use dcpwizard_core::mxf_wrap::AudioInputOrder;
    use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
    use std::time::Duration;

    #[test]
    fn stage_timing_reads_as_minutes_and_seconds() {
        assert_eq!(
            format_stage_timing("encode", Duration::from_secs(192)),
            "[TIMING] encode took 3m12s"
        );
        assert_eq!(
            format_stage_timing("package", Duration::from_millis(1900)),
            "[TIMING] package took 0m1s"
        );
        assert_eq!(
            format_stage_timing("total", Duration::from_secs(3600)),
            "[TIMING] total took 60m0s"
        );
    }

    #[test]
    fn the_encode_breakdown_is_one_timing_line_and_absent_when_nothing_was_measured() {
        let measured = postkit::pipeline::PipelineProgress {
            stage: "encode".into(),
            message: "Frame 100/200".into(),
            frame: 100,
            total_frames: 200,
            fps: 12.0,
            elapsed_secs: 300.0,
            percent: 50.0,
            decode_wait_secs: 12.0,
            prepare_secs: 30.4,
            encode_secs: 250.0,
            write_secs: 7.6,
        };
        assert_eq!(
            format_encode_breakdown(&measured).as_deref(),
            Some(
                "[TIMING] encode breakdown: decoder wait 12s, frame prep 30s, j2k 4m10s, write 8s"
            )
        );

        let unmeasured = postkit::pipeline::PipelineProgress {
            decode_wait_secs: 0.0,
            prepare_secs: 0.0,
            encode_secs: 0.0,
            write_secs: 0.0,
            ..measured
        };
        assert_eq!(format_encode_breakdown(&unmeasured), None);
    }

    fn test_job() -> JobConfig {
        JobConfig {
            id: 1,
            video_path: PathBuf::from("/in/movie.mov"),
            title: "Test".into(),
            output_dir: PathBuf::from("/out"),
            audio_path: None,
            audio_map: None,
            picture: dcpwizard_core::source_picture::SourcePictureOptions::default(),
            validate: false,
            standard: "smpte".into(),
            resolution: "2k-flat".into(),
            framerate: "24".into(),
            bandwidth: 250,
            quality_psnr: None,
            colour: "xyz".into(),
            content_kind: "feature".into(),
            encrypt: false,
            key_out: None,
            channels: "5.1".into(),
            right_eye: None,
            atmos: None,
            subtitle: None,
            subtitle_language: "en".into(),
            burn_subtitle: None,
            burn_subtitle_font: None,
            burn_style: postkit::subtitle_raster::BurnStyleOverrides::default(),
            subtitle_appearance: dcpwizard_core::subtitle::TimedTextAppearance::default(),
            ccap: None,
            ccap_language: "en".into(),
            loudness_target: None,
            true_peak_ceiling: None,
            audio_channel_dir: None,
            audio_input_order: AudioInputOrder::Canonical51,
            audio_channels: None,
            sign_language_video: None,
            sign_language_tag: None,
            pad_head: None,
            pad_tail: None,
            pad_color: None,
            audio_delay_ms: 0,
            trim_start_frames: 0,
            trim_end_frames: 0,
            still_length_frames: 0,
            source_colourspace: postkit::colour::ColourSpace::Rec709,
            upmix: None,
            reel_length_minutes: 0,
            reel_split_frames: Vec::new(),
            split_chapters: false,
            versions: Vec::new(),
            hdr_dci: false,
            source_colour: postkit::encode::SourceColour::DisplayRgb,
            allow_generic_hdr_tonemap: false,
            facility: None,
            naming: NamingMetadata::default(),
            source: None,
            hints: Vec::new(),
            head_items: Vec::new(),
            tail_items: Vec::new(),
        }
    }

    fn attached(name: &str) -> dcpwizard_core::library::AttachedItem {
        dcpwizard_core::library::AttachedItem {
            item: dcpwizard_core::library::LibraryItem {
                name: name.into(),
                kind: dcpwizard_core::library::LibraryItemKind::HeadIdent,
                file: format!("{name}.mov"),
                seconds: 8.0,
                width: 1920,
                height: 1080,
                has_audio: true,
            },
            media: PathBuf::from(format!("/library/media/{name}.mov")),
        }
    }

    #[test]
    fn the_panels_library_items_reach_the_build_in_order() {
        let job = JobConfig {
            head_items: vec![attached("Ident"), attached("Rating")],
            tail_items: vec![attached("Anti Piracy")],
            ..test_job()
        };
        let config = build_dcp_config(
            &job,
            PathBuf::from("/out/j2k"),
            None,
            None,
            None,
            Vec::new(),
        );
        let head: Vec<&str> = config
            .head_items
            .iter()
            .map(|a| a.item.name.as_str())
            .collect();
        assert_eq!(head, ["Ident", "Rating"]);
        assert_eq!(config.tail_items.len(), 1);
        assert_eq!(config.tail_items[0].item.name, "Anti Piracy");
        // and the pre-build check counts them, so the refusals fire before the encode
        assert_eq!(job_plan(&job).library_items, 3);
    }

    #[test]
    fn the_isdcf_name_takes_over_a_folder_derived_from_the_title() {
        assert_eq!(
            renamed_output_dir(
                "/out/My Film",
                "My Film",
                "MyFilm_TST-1_F_EN-XX_20_2K_SMPTE_OV"
            ),
            PathBuf::from("/out/MyFilm_TST-1_F_EN-XX_20_2K_SMPTE_OV")
        );
    }

    #[test]
    fn a_folder_the_user_chose_keeps_its_name() {
        assert_eq!(
            renamed_output_dir(
                "/out/deliveries",
                "My Film",
                "MyFilm_TST-1_F_EN-XX_20_2K_SMPTE_OV"
            ),
            PathBuf::from("/out/deliveries")
        );
    }

    #[test]
    fn the_panel_builds_the_name_from_what_it_holds() {
        let request = IsdcfNameRequest {
            title: "My Film".into(),
            standard: Some("smpte".into()),
            resolution: Some("2k-flat".into()),
            framerate: Some("24".into()),
            content_kind: Some("test".into()),
            facility: Some("PPF".into()),
            naming: NamingMetadata {
                audio_language: Some("en".into()),
                content_versions: Some("Final Cut, ".into()),
                isdcf_naming: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let name = isdcf_name_for(&request).unwrap();
        assert!(name.starts_with("MyFilm_TST-1_F_EN-XX_MOS_2K_"), "{name}");
        assert!(name.ends_with("_PPF_SMPTE_OV"), "{name}");
    }

    #[test]
    fn the_auto_name_preview_uses_the_cropped_source_raster() {
        let request = IsdcfNameRequest {
            title: "My Film".into(),
            resolution: Some("auto".into()),
            source_width: Some(4096),
            source_height: Some(2160),
            crop_top: Some(222),
            crop_bottom: Some(222),
            naming: NamingMetadata {
                isdcf_naming: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let name = isdcf_name_for(&request).unwrap();
        assert!(name.contains("_S_"), "{name}");
        assert!(name.contains("_4K_"), "{name}");
    }

    #[test]
    fn the_submitted_auto_name_uses_the_planned_raster() {
        let mut job = JobConfig {
            title: "My Film".into(),
            output_dir: PathBuf::from("/out/My Film"),
            resolution: "auto".into(),
            naming: NamingMetadata {
                isdcf_naming: true,
                ..Default::default()
            },
            ..test_job()
        };
        apply_isdcf_name_to_job(&mut job, Some((4096, 1716))).unwrap();
        assert!(job.title.contains("_S_"), "{}", job.title);
        assert!(job.title.contains("_4K_"), "{}", job.title);
        assert_eq!(job.output_dir, PathBuf::from("/out").join(&job.title));
    }

    #[test]
    fn auto_four_k_uses_the_four_k_frame_rate_limit() {
        let job = JobConfig {
            resolution: "auto".into(),
            framerate: "48".into(),
            source: Some(postkit::probe::VideoInfo {
                width: 4096,
                height: 2160,
                fps_num: 48,
                fps_den: 1,
                has_audio: false,
                total_frames: 48,
            }),
            ..test_job()
        };
        let error = checked_job_plan(&job).expect_err("48 fps 4K must fail");
        assert!(error.contains("4K"), "{error}");
    }

    #[test]
    fn an_unknown_territory_type_is_refused() {
        let request = IsdcfNameRequest {
            title: "My Film".into(),
            naming: NamingMetadata {
                territory_type: Some("worldwide".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(isdcf_name_for(&request).is_err());
    }

    fn hdr_panel() -> HdrPanelOptions {
        HdrPanelOptions {
            dci: true,
            lut: None,
            already_pq: true,
            allow_generic_tonemap: false,
        }
    }

    fn write_mono(path: &std::path::Path, value: i32, frames: usize) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 24,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).unwrap();
        for _ in 0..frames {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn only_a_folder_holding_a_dcp_counts_as_one() {
        // delete_dcp refuses anything this rejects, so a recent entry pointing
        // at a folder of source media cannot delete it.
        let dir = tempfile::tempdir().unwrap();
        assert!(!holds_dcp(dir.path()));
        std::fs::write(dir.path().join("movie.mov"), b"x").unwrap();
        assert!(!holds_dcp(dir.path()));
        std::fs::write(dir.path().join("ASSETMAP.xml"), b"x").unwrap();
        assert!(holds_dcp(dir.path()));
    }

    #[test]
    fn audio_input_order_maps_to_core() {
        assert_eq!(
            parse_audio_input_order(Some("lrc-ls-rs-lfe")).unwrap(),
            AudioInputOrder::LrcLsRsLfe
        );
        assert_eq!(
            parse_audio_input_order(Some("dcp")).unwrap(),
            AudioInputOrder::Canonical51
        );
        assert_eq!(
            parse_audio_input_order(None).unwrap(),
            AudioInputOrder::Canonical51
        );
        assert!(parse_audio_input_order(Some("smpte")).is_err());
    }

    #[test]
    fn channel_directory_is_routed_to_one_interleaved_wav() {
        let dir = tempfile::tempdir().unwrap();
        let channels = dir.path().join("channels");
        std::fs::create_dir_all(&channels).unwrap();
        let full_scale = 1i32 << 23;
        write_mono(&channels.join("mix_L.wav"), full_scale / 10, 64);
        write_mono(&channels.join("mix_R.wav"), full_scale / 5, 64);
        write_mono(&channels.join("mix_C.wav"), full_scale / 4, 64);
        write_mono(&channels.join("mix_Lfe.wav"), full_scale / 3, 64);
        write_mono(&channels.join("mix_Ls.wav"), full_scale / 2, 64);
        write_mono(&channels.join("mix_Rs.wav"), (full_scale / 3) * 2, 64);

        let mut job = test_job();
        job.audio_channel_dir = Some(channels.to_string_lossy().into_owned());

        let routed = prepare_audio(&job, SourceConform::default(), dir.path(), |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(routed, dir.path().join("audio_work").join("routed.wav"));

        let mut reader = WavReader::open(&routed).unwrap();
        assert_eq!(reader.spec().channels, 6);
        let samples: Vec<i32> = reader.samples::<i32>().map(|s| s.unwrap()).collect();
        assert_eq!(
            &samples[..6],
            &[
                full_scale / 10,
                full_scale / 5,
                full_scale / 4,
                full_scale / 3,
                full_scale / 2,
                (full_scale / 3) * 2,
            ]
        );
    }

    /// A short clip with a sine track, false when ffmpeg cannot build one.
    fn clip_with_sound(path: &std::path::Path) -> bool {
        std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=24",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-ac",
                "2",
                "-frames:v",
                "24",
                "-shortest",
            ])
            .arg(path)
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
            && path.exists()
    }

    #[test]
    fn the_sources_own_audio_becomes_the_sound_when_none_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("movie.mp4");
        if !clip_with_sound(&clip) {
            eprintln!("skipping: ffmpeg could not build the source clip");
            return;
        }

        let mut job = test_job();
        job.video_path = clip;
        job.upmix = Some(postkit::upmix::Upmixer::A);

        let lines = std::sync::Mutex::new(Vec::new());
        let prepared = prepare_audio(&job, SourceConform::default(), dir.path(), |msg| {
            lines.lock().unwrap().push(msg.to_string())
        })
        .unwrap()
        .expect("the source's own audio");
        let lines = lines.into_inner().unwrap();

        // the extracted sound goes through the rest of the chain, so the upmix
        // that follows it is what the job actually packages
        assert_eq!(prepared, dir.path().join("audio_work").join("upmix.wav"));
        let reader = WavReader::open(&prepared).unwrap();
        assert_eq!(reader.spec().channels, 6, "the upmix ran on the extraction");
        assert!(
            lines.iter().any(|line| line.contains("source's own audio")),
            "{lines:?}"
        );
    }

    #[test]
    fn audio_file_is_untouched_without_routing_or_loudness() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("sound.wav");
        write_mono(&wav, 1234, 16);

        let mut job = test_job();
        job.audio_path = Some(wav.to_string_lossy().into_owned());

        let prepared = prepare_audio(&job, SourceConform::default(), dir.path(), |_| {}).unwrap();
        assert_eq!(prepared, Some(wav));
        assert!(!dir.path().join("audio_work").exists());
    }

    #[test]
    fn the_audio_delay_reaches_the_sound_and_keeps_its_length() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("sound.wav");
        write_mono(&wav, 1234, 48_000);

        let mut job = test_job();
        job.audio_path = Some(wav.to_string_lossy().into_owned());
        job.audio_delay_ms = 100;

        let delayed = prepare_audio(&job, SourceConform::default(), dir.path(), |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(delayed, dir.path().join("audio_work").join("delayed.wav"));
        let reader = WavReader::open(&delayed).unwrap();
        assert_eq!(
            reader.duration(),
            48_000,
            "a delay must not change the running time"
        );
    }

    #[test]
    fn the_conform_pulls_the_sound_up_by_one_part_in_1001() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("sound.wav");
        write_mono(&wav, 1234, 48_000);

        let mut job = test_job();
        job.audio_path = Some(wav.to_string_lossy().into_owned());
        let conform = dcpwizard_core::hfr::conform_source_to_dcp(24_000, 1_001, 24);

        let pulled_up = prepare_audio(&job, conform, dir.path(), |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(pulled_up, dir.path().join("audio_work").join("pullup.wav"));
        let reader = WavReader::open(&pulled_up).unwrap();
        let expected = 48_000 * 1_000 / 1_001;
        assert!(
            reader.duration().abs_diff(expected) <= 1,
            "{} samples, expected about {expected}",
            reader.duration()
        );
    }

    #[test]
    fn a_trim_reaches_the_frames_the_sound_and_the_cue_window() {
        let dir = tempfile::tempdir().unwrap();
        let j2k = dir.path().join("j2k");
        std::fs::create_dir_all(&j2k).unwrap();
        for i in 0..48u64 {
            std::fs::write(j2k.join(format!("frame_{i:08}.j2c")), [i as u8]).unwrap();
        }
        let wav = dir.path().join("sound.wav");
        write_mono(&wav, 1234, 96_000); // 48 frames at 24 fps

        let mut job = test_job();
        job.trim_start_frames = 12;
        job.trim_end_frames = 12;

        let (trimmed_dir, trimmed_audio) =
            apply_trim(&job, &j2k, dir.path(), Some(wav), 24, None).unwrap();
        assert_eq!(dcpwizard_core::trim::frame_count(&trimmed_dir), 24);
        let reader = WavReader::open(trimmed_audio.unwrap()).unwrap();
        assert_eq!(reader.duration(), 48_000, "24 frames at 24 fps");

        let config = build_dcp_config(&job, trimmed_dir, None, None, None, Vec::new());
        assert_eq!(
            config.source_trim,
            dcpwizard_core::subtitle::SourceTrim {
                start_frames: 12,
                kept_frames: 24,
            },
            "subtitles are clamped to the frames that survived"
        );
    }

    #[test]
    fn no_trim_leaves_the_frames_and_the_cue_window_alone() {
        let dir = tempfile::tempdir().unwrap();
        let j2k = dir.path().join("j2k");
        std::fs::create_dir_all(&j2k).unwrap();
        std::fs::write(j2k.join("frame_00000000.j2c"), [0u8]).unwrap();

        let job = test_job();
        let (same, audio) = apply_trim(&job, &j2k, dir.path(), None, 24, None).unwrap();
        assert_eq!(same, j2k, "an untrimmed job must not relink a thing");
        assert!(audio.is_none());
        assert_eq!(
            build_dcp_config(&job, j2k, None, None, None, Vec::new()).source_trim,
            dcpwizard_core::subtitle::SourceTrim::default()
        );
    }

    #[test]
    fn a_windowed_encode_leaves_the_frames_where_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let j2k = dir.path().join("j2k");
        std::fs::create_dir_all(&j2k).unwrap();
        for i in 0..24u64 {
            std::fs::write(j2k.join(format!("frame_{i:08}.j2c")), [i as u8]).unwrap();
        }
        let wav = dir.path().join("sound.wav");
        write_mono(&wav, 1234, 96_000);

        let mut job = test_job();
        job.trim_start_frames = 12;
        job.trim_end_frames = 12;
        let window = postkit::encode::FrameRange {
            first_frame: 12,
            frame_count: 24,
        };

        let (picture, audio) =
            apply_trim(&job, &j2k, dir.path(), Some(wav), 24, Some(window)).unwrap();
        assert_eq!(
            picture, j2k,
            "the encode already wrote only the kept frames"
        );
        assert!(
            !dir.path().join("j2k_trimmed").exists(),
            "a windowed encode must not relink a second copy of the picture"
        );
        let reader = WavReader::open(audio.unwrap()).unwrap();
        assert_eq!(reader.duration(), 48_000, "24 frames at 24 fps");
    }

    #[test]
    fn a_probed_video_is_windowed_and_a_codestream_directory_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut job = test_job();
        job.trim_start_frames = 12;
        job.trim_end_frames = 12;
        job.source = Some(postkit::probe::VideoInfo {
            width: 1920,
            height: 1080,
            fps_num: 24,
            fps_den: 1,
            has_audio: true,
            total_frames: 48,
        });

        let window = job_encode_window(&job, std::path::Path::new("/in/movie.mov"))
            .unwrap()
            .expect("a probed video is encoded, so it takes a window");
        assert_eq!(window.first_frame, 12);
        assert_eq!(window.frame_count, 24);

        let codestreams = dir.path().join("j2k");
        std::fs::create_dir_all(&codestreams).unwrap();
        std::fs::write(codestreams.join("frame_00000000.j2c"), [0u8]).unwrap();
        assert!(job_encode_window(&job, &codestreams).unwrap().is_none());

        job.source = None;
        assert!(
            job_encode_window(&job, std::path::Path::new("/in/movie.mov"))
                .unwrap()
                .is_none(),
            "no probed length leaves the count to the relink"
        );
    }

    #[test]
    fn panel_options_reach_the_core_config() {
        let mut job = test_job();
        job.audio_input_order = AudioInputOrder::LrcLsRsLfe;
        job.pad_head = Some("48f".into());
        job.pad_tail = Some("2s".into());
        job.pad_color = Some("#101010".into());
        job.sign_language_tag = Some("sgn-ase".into());
        job.sign_language_video = Some("/in/signer.mov".into());

        let config = build_dcp_config(
            &job,
            PathBuf::from("/out/j2k"),
            None,
            Some(PathBuf::from("/out/slvs_sound.wav")),
            Some(6),
            Vec::new(),
        );

        assert_eq!(config.audio_input_order, AudioInputOrder::LrcLsRsLfe);
        assert_eq!(config.pad_head.as_deref(), Some("48f"));
        assert_eq!(config.pad_tail.as_deref(), Some("2s"));
        assert_eq!(config.pad_color.as_deref(), Some("#101010"));
        assert_eq!(config.sign_language_lang.as_deref(), Some("sgn-ase"));
        assert_eq!(config.sign_language_main_channels, Some(6));
        assert_eq!(
            config.audio_path,
            Some(PathBuf::from("/out/slvs_sound.wav"))
        );
    }

    #[test]
    fn omitted_options_leave_the_core_defaults() {
        let config = build_dcp_config(
            &test_job(),
            PathBuf::from("/out/j2k"),
            None,
            None,
            None,
            Vec::new(),
        );

        assert_eq!(config.audio_input_order, AudioInputOrder::Canonical51);
        assert_eq!(config.pad_head, None);
        assert_eq!(config.pad_tail, None);
        assert_eq!(config.pad_color, None);
        assert_eq!(config.sign_language_lang, None);
        assert_eq!(config.sign_language_main_channels, None);
        assert_eq!(config.container_width, 1998);
        assert_eq!(config.frame_rate_num, 24);
        assert_eq!(config.reel_length_minutes, 0);
        assert!(config.reel_split_frames.is_empty());
    }

    #[test]
    fn upmix_variant_parses() {
        assert!(matches!(
            parse_upmixer(Some("a")).unwrap(),
            Some(postkit::upmix::Upmixer::A)
        ));
        assert!(matches!(
            parse_upmixer(Some("b")).unwrap(),
            Some(postkit::upmix::Upmixer::B)
        ));
        assert!(parse_upmixer(Some("none")).unwrap().is_none());
        assert!(parse_upmixer(None).unwrap().is_none());
        assert!(parse_upmixer(Some("c")).is_err());
    }

    #[test]
    fn upmix_turns_a_stereo_file_into_five_one() {
        let dir = tempfile::tempdir().unwrap();
        let stereo = dir.path().join("stereo.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 24,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&stereo, spec).unwrap();
        for frame in 0..4800 {
            let value = ((frame % 48) - 24) * 10_000;
            writer.write_sample(value).unwrap();
            writer.write_sample(-value).unwrap();
        }
        writer.finalize().unwrap();

        let mut job = test_job();
        job.audio_path = Some(stereo.to_string_lossy().into_owned());
        job.upmix = Some(postkit::upmix::Upmixer::B);

        let upmixed = prepare_audio(&job, SourceConform::default(), dir.path(), |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(upmixed, dir.path().join("audio_work").join("upmix.wav"));
        let reader = WavReader::open(&upmixed).unwrap();
        assert_eq!(reader.spec().channels, 6);
    }

    #[test]
    fn split_timecodes_become_reel_frames() {
        assert_eq!(
            parse_split_timecodes("00:00:10, 00:01:00:12", 24).unwrap(),
            vec![240, 1452]
        );
        assert!(parse_split_timecodes("10 minutes", 24).is_err());
        assert!(parse_split_timecodes(" , ", 24).is_err());
    }

    #[test]
    fn reel_splitting_reaches_the_core_config() {
        let mut job = test_job();
        job.reel_length_minutes = 20;

        let config = build_dcp_config(
            &job,
            PathBuf::from("/out/j2k"),
            None,
            None,
            None,
            parse_split_timecodes("00:00:10", 24).unwrap(),
        );

        assert_eq!(config.reel_length_minutes, 20);
        assert_eq!(config.reel_split_frames, vec![240]);
    }

    #[test]
    fn versions_manifest_is_loaded_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        let subtitle = dir.path().join("fr.srt");
        std::fs::write(&subtitle, "1\n00:00:00,000 --> 00:00:01,000\nbonjour\n").unwrap();
        let manifest = dir.path().join("versions.json");
        std::fs::write(
            &manifest,
            serde_json::json!([
                { "title": "Feature OV" },
                { "title": "Feature FR", "subtitle": subtitle, "subtitle_language": "fr" },
            ])
            .to_string(),
        )
        .unwrap();

        let specs = load_version_specs(Some(manifest.to_str().unwrap())).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[1].title, "Feature FR");
        assert_eq!(specs[1].subtitle.as_deref(), Some(subtitle.as_path()));

        assert!(load_version_specs(None).unwrap().is_empty());
        assert!(load_version_specs(Some("")).unwrap().is_empty());

        let missing = dir.path().join("missing.json");
        std::fs::write(
            &missing,
            r#"[{"title": "Gone", "subtitle": "/no/such/file.srt"}]"#,
        )
        .unwrap();
        assert!(load_version_specs(Some(missing.to_str().unwrap())).is_err());
    }

    #[test]
    fn dci_hdr_needs_a_pq_source() {
        let panel = HdrPanelOptions {
            dci: true,
            lut: None,
            already_pq: false,
            allow_generic_tonemap: false,
        };
        let error = resolve_hdr(&panel, 24, 250, false, false, false).unwrap_err();
        assert!(error.contains("already PQ"), "{error}");

        assert_eq!(
            resolve_hdr(&hdr_panel(), 24, 250, false, false, false).unwrap(),
            postkit::encode::SourceColour::AlreadyPq
        );
    }

    #[test]
    fn an_hdr_to_dci_lut_becomes_the_source_colour() {
        let dir = tempfile::tempdir().unwrap();
        let lut = dir.path().join("hdr_to_dci.cube");
        std::fs::write(&lut, "LUT_3D_SIZE 2\n").unwrap();

        let panel = HdrPanelOptions {
            dci: true,
            lut: Some(lut.to_string_lossy().into_owned()),
            already_pq: false,
            allow_generic_tonemap: false,
        };
        assert_eq!(
            resolve_hdr(&panel, 24, 250, false, false, false).unwrap(),
            postkit::encode::SourceColour::DciLut(lut)
        );

        let missing = HdrPanelOptions {
            lut: Some(dir.path().join("gone.cube").to_string_lossy().into_owned()),
            ..panel
        };
        assert!(resolve_hdr(&missing, 24, 250, false, false, false)
            .unwrap_err()
            .contains("LUT not found"));
    }

    #[test]
    fn dci_hdr_rejects_the_combinations_the_cli_rejects() {
        let over_cap = resolve_hdr(&hdr_panel(), 24, 500, false, false, false).unwrap_err();
        assert!(over_cap.contains("2343750 bytes/frame"), "{over_cap}");

        assert!(resolve_hdr(&hdr_panel(), 24, 250, true, false, false).is_err());
        assert!(resolve_hdr(&hdr_panel(), 24, 250, false, true, false).is_err());
        assert!(resolve_hdr(&hdr_panel(), 24, 250, false, false, true).is_err());

        let two_paths = HdrPanelOptions {
            allow_generic_tonemap: true,
            ..hdr_panel()
        };
        assert!(resolve_hdr(&two_paths, 24, 250, false, false, false)
            .unwrap_err()
            .contains("Choose one HDR source path"));
    }

    #[test]
    fn a_plain_job_keeps_display_rgb() {
        let panel = HdrPanelOptions {
            dci: false,
            lut: None,
            already_pq: false,
            allow_generic_tonemap: false,
        };
        assert_eq!(
            resolve_hdr(&panel, 24, 250, true, true, true).unwrap(),
            postkit::encode::SourceColour::DisplayRgb
        );
    }

    #[test]
    fn an_hdr_source_needs_a_path_to_dci() {
        use postkit::dolby_vision::HdrType;
        use postkit::encode::SourceColour;

        assert_eq!(
            plan_hdr_source(HdrType::Sdr, &SourceColour::DisplayRgb, false).unwrap(),
            HdrSourceStep::EncodeDirectly
        );
        assert_eq!(
            plan_hdr_source(HdrType::Hdr10, &SourceColour::AlreadyPq, false).unwrap(),
            HdrSourceStep::EncodeDirectly
        );
        assert_eq!(
            plan_hdr_source(
                HdrType::Hlg,
                &SourceColour::DciLut(PathBuf::from("/luts/hdr_to_dci.cube")),
                false,
            )
            .unwrap(),
            HdrSourceStep::EncodeDirectly
        );
        assert_eq!(
            plan_hdr_source(HdrType::Hdr10, &SourceColour::DisplayRgb, true).unwrap(),
            HdrSourceStep::TonemapToSdr
        );
        assert!(plan_hdr_source(HdrType::Hdr10, &SourceColour::DisplayRgb, false).is_err());
        // a P3 source is display RGB, so an HDR file labelled P3 must still say
        // how it reaches DCI instead of being matrixed straight through
        assert!(plan_hdr_source(
            HdrType::Hdr10,
            &SourceColour::DisplayRgbIn(postkit::colour::ColourSpace::P3),
            false,
        )
        .is_err());
    }

    #[test]
    fn dci_hdr_reaches_the_core_config() {
        let mut job = test_job();
        job.hdr_dci = true;
        job.source_colour = postkit::encode::SourceColour::AlreadyPq;

        let config = build_dcp_config(
            &job,
            PathBuf::from("/out/j2k"),
            None,
            None,
            None,
            Vec::new(),
        );
        assert!(config.hdr_dci);

        let plain = build_dcp_config(
            &test_job(),
            PathBuf::from("/out/j2k"),
            None,
            None,
            None,
            Vec::new(),
        );
        assert!(!plain.hdr_dci);
    }

    #[test]
    fn profiles_map_to_panel_controls() {
        let profiles = profile_panel_list();
        assert_eq!(profiles.len(), 4);

        let four_k = profiles.iter().find(|p| p.name == "cinema_4k").unwrap();
        assert_eq!(four_k.resolution.as_deref(), Some("4k-full"));
        assert_eq!(four_k.framerate, "24");
        assert_eq!(four_k.bandwidth, 500);
        assert_eq!(four_k.content_kind, "feature");
        assert_eq!(four_k.standard.as_deref(), Some("smpte"));

        let trailer = profiles.iter().find(|p| p.name == "trailer").unwrap();
        assert_eq!(trailer.resolution.as_deref(), Some("2k-scope"));
        assert_eq!(trailer.content_kind, "trailer");

        // the panel controls a profile names must drive the same containers the
        // job path maps back to.
        for profile in &profiles {
            let key = profile.resolution.as_deref().unwrap();
            let core = dcpwizard_core::profiles::get_profile(&profile.name).unwrap();
            assert_eq!(
                container_of(key),
                (core.resolution_width, core.resolution_height)
            );
        }
    }

    #[test]
    fn a_queued_job_comes_back_from_the_jobs_file() {
        use postkit::gui_job_queue::{load, record, StoredJobState};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("gui-jobs.jsonl");

        let mut queued = test_job();
        queued.id = 4;
        queued.title = "Restored".into();
        queued.upmix = Some(postkit::upmix::Upmixer::A);
        queued.burn_style.colour = Some(postkit::subtitle_formats::Rgba {
            r: 1,
            g: 2,
            b: 3,
            a: 4,
        });
        queued.burn_style.effect = Some(postkit::subtitle_raster::BurnEffect::Outline);
        queued.source = Some(postkit::probe::VideoInfo {
            width: 1920,
            height: 1080,
            fps_num: 24,
            fps_den: 1,
            has_audio: true,
            total_frames: 100,
        });

        record(&path, StoredJobState::Queued, "", &queued);

        let queue = JobQueue::new(path.clone());
        assert_eq!(queue.load_jobs_file(), 0);

        let restored = queue.take_next().unwrap();
        assert_eq!(restored.id, 4);
        assert_eq!(restored.title, "Restored");
        assert_eq!(restored.burn_style, queued.burn_style);
        assert_eq!(restored.source.as_ref().unwrap().width, 1920);
        assert!(matches!(restored.upmix, Some(postkit::upmix::Upmixer::A)));

        let saved = load::<JobConfig>(&path);
        assert_eq!(saved.jobs.len(), 1);
        assert_eq!(saved.jobs[0].state, StoredJobState::Queued);
    }
}
