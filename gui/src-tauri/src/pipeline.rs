use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

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

#[derive(Clone, Serialize)]
pub struct JobInfo {
    pub id: u64,
    pub title: String,
    pub status: String,
    pub percent: f64,
}

// ─── Job types ─────────────────────────────────────────────────────────────

#[derive(Clone)]
#[allow(dead_code)]
struct JobConfig {
    id: u64,
    video_path: PathBuf,
    title: String,
    output_dir: PathBuf,
    audio_path: Option<String>,
    validate: bool,
    standard: String,
    resolution: String,
    framerate: String,
    bandwidth: u32,
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
    ccap: Option<String>,
    ccap_language: String,
    // loudness normalize spec (leqm=<db> or lufs=<value>) applied to the audio
    loudness_target: Option<String>,
    true_peak_ceiling: Option<f64>,
    // directory of mono channel WAVs (name_L.wav, name_Lfe.wav, ...) routed to
    // one interleaved WAV, replacing audio_path
    audio_channel_dir: Option<String>,
    audio_input_order: dcpwizard_core::mxf_wrap::AudioInputOrder,
    // sign-language video (ISDCF Doc 13) packed onto sound channel 15
    sign_language_video: Option<String>,
    sign_language_tag: Option<String>,
    pad_head: Option<String>,
    pad_tail: Option<String>,
    pad_color: Option<String>,
    // stereo -> 5.1 upmix applied before loudness normalization
    upmix: Option<postkit::upmix::Upmixer>,
    reel_length_minutes: u32,
    // explicit reel boundaries in frames, from the panel's split timecodes
    reel_split_frames: Vec<u64>,
    split_chapters: bool,
    // one CPL per entry over shared essence; empty keeps the single-CPL path
    versions: Vec<dcpwizard_core::versions::VersionSpec>,
}

// ─── Queue state (managed by Tauri) ────────────────────────────────────────

pub struct JobQueue {
    queue: Mutex<VecDeque<JobConfig>>,
    next_id: AtomicU64,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    current_id: AtomicU64,
    current_title: Mutex<String>,
    current_status: Mutex<String>,
}

impl JobQueue {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            current_id: AtomicU64::new(0),
            current_title: Mutex::new(String::new()),
            current_status: Mutex::new(String::new()),
        }
    }
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
    colour: Option<String>,
    content_kind: Option<String>,
    encrypt: Option<bool>,
    key_out: Option<String>,
    channels: Option<String>,
    right_eye: Option<String>,
    atmos: Option<String>,
    subtitle: Option<String>,
    subtitle_language: Option<String>,
    ccap: Option<String>,
    ccap_language: Option<String>,
    loudness_target: Option<String>,
    true_peak_ceiling: Option<f64>,
    audio_channel_dir: Option<String>,
    audio_input_order: Option<String>,
    sign_language_video: Option<String>,
    sign_language_tag: Option<String>,
    pad_head: Option<String>,
    pad_tail: Option<String>,
    pad_color: Option<String>,
    upmix: Option<String>,
    reel_length_minutes: Option<u32>,
    split_at: Option<String>,
    split_chapters: Option<bool>,
    versions: Option<String>,
) -> Result<u64, String> {
    let queue = app.state::<JobQueue>();
    let id = queue.next_id.fetch_add(1, Ordering::Relaxed);

    // Never encrypt without an explicit key destination.
    if encrypt.unwrap_or(false) && key_out.as_deref().unwrap_or("").is_empty() {
        return Err("Key Output File is required when encrypting".into());
    }

    let framerate = framerate.unwrap_or_else(|| "24".into());
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

    let upmix = parse_upmixer(upmix.as_deref())?;

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

    let job = JobConfig {
        id,
        video_path: PathBuf::from(&video_path),
        title: title.clone(),
        output_dir: PathBuf::from(&output_dir),
        audio_path,
        validate: validate.unwrap_or(false),
        standard: standard.unwrap_or_else(|| "smpte".into()),
        resolution: resolution.unwrap_or_else(|| "2k-full".into()),
        framerate,
        bandwidth: bandwidth.unwrap_or(250),
        colour: colour.unwrap_or_else(|| "xyz".into()),
        content_kind: content_kind.unwrap_or_else(|| "feature".into()),
        encrypt: encrypt.unwrap_or(false),
        key_out: key_out.filter(|k| !k.is_empty()),
        channels: channels.unwrap_or_else(|| "5.1".into()),
        right_eye: right_eye.filter(|s| !s.is_empty()),
        atmos: atmos.filter(|s| !s.is_empty()),
        subtitle,
        subtitle_language: subtitle_language.unwrap_or_else(|| "en".into()),
        ccap,
        ccap_language: ccap_language.unwrap_or_else(|| "en".into()),
        loudness_target: loudness_target.filter(|s| !s.is_empty()),
        true_peak_ceiling,
        audio_channel_dir: audio_channel_dir.filter(|s| !s.is_empty()),
        audio_input_order,
        sign_language_video,
        sign_language_tag,
        pad_head,
        pad_tail,
        pad_color,
        upmix,
        reel_length_minutes,
        reel_split_frames,
        split_chapters,
        versions,
    };

    {
        let mut q = queue.queue.lock().unwrap();
        q.push_back(job);
    }

    if queue.current_id.load(Ordering::Relaxed) == 0 {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            run_queue_worker(app2).await;
        });
    }

    Ok(id)
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

#[tauri::command]
pub async fn cancel_job(app: AppHandle, job_id: u64) -> Result<(), String> {
    let queue = app.state::<JobQueue>();
    if queue.current_id.load(Ordering::Relaxed) == job_id {
        queue.cancel.store(true, Ordering::Relaxed);
        return Ok(());
    }
    let mut q = queue.queue.lock().unwrap();
    q.retain(|j| j.id != job_id);
    Ok(())
}

#[tauri::command]
pub async fn pause_job(app: AppHandle) -> Result<(), String> {
    let queue = app.state::<JobQueue>();
    queue.pause.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn resume_job(app: AppHandle) -> Result<(), String> {
    let queue = app.state::<JobQueue>();
    queue.pause.store(false, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn list_jobs(app: AppHandle) -> Vec<JobInfo> {
    let queue = app.state::<JobQueue>();
    let mut jobs = Vec::new();

    let current_id = queue.current_id.load(Ordering::Relaxed);
    if current_id > 0 {
        let title = queue.current_title.lock().unwrap().clone();
        let status = queue.current_status.lock().unwrap().clone();
        jobs.push(JobInfo {
            id: current_id,
            title,
            status,
            percent: 0.0,
        });
    }

    let q = queue.queue.lock().unwrap();
    for job in q.iter() {
        jobs.push(JobInfo {
            id: job.id,
            title: job.title.clone(),
            status: "queued".to_string(),
            percent: 0.0,
        });
    }
    jobs
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
        let job = {
            let queue = app.state::<JobQueue>();
            let mut q = queue.queue.lock().unwrap();
            q.pop_front()
        };

        let Some(job) = job else {
            let queue = app.state::<JobQueue>();
            queue.current_id.store(0, Ordering::Relaxed);
            break;
        };

        {
            let queue = app.state::<JobQueue>();
            queue.current_id.store(job.id, Ordering::Relaxed);
            *queue.current_title.lock().unwrap() = job.title.clone();
            *queue.current_status.lock().unwrap() = "running".to_string();
            queue.cancel.store(false, Ordering::Relaxed);
            queue.pause.store(false, Ordering::Relaxed);
        }

        let result = tokio::task::spawn_blocking({
            let app = app.clone();
            let job = job.clone();
            move || run_job(&app, &job)
        })
        .await;

        let queue = app.state::<JobQueue>();
        match result {
            Ok(Ok(_)) => {
                *queue.current_status.lock().unwrap() = "done".to_string();
                emit_progress(&app, job.id, "done", "Complete", 0, 0, 0.0, 0.0, 100.0);
            }
            Ok(Err(e)) => {
                let status = if queue.cancel.load(Ordering::Relaxed) {
                    "cancelled".to_string()
                } else {
                    format!("failed: {e}")
                };
                *queue.current_status.lock().unwrap() = status;
                emit_progress(&app, job.id, "error", &e, 0, 0, 0.0, 0.0, 0.0);
            }
            Err(e) => {
                *queue.current_status.lock().unwrap() = format!("panic: {e}");
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

fn container_of(resolution: &str) -> (u32, u32) {
    RESOLUTION_CONTAINERS
        .iter()
        .find(|(key, _, _)| *key == resolution)
        .map(|(_, width, height)| (*width, *height))
        .unwrap_or((0, 0))
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
    match framerate {
        "25" => (25, 1),
        "30" => (30, 1),
        "48" => (48, 1),
        "50" => (50, 1),
        "60" => (60, 1),
        "96" => (96, 1),
        "100" => (100, 1),
        "120" => (120, 1),
        _ => (24, 1),
    }
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
/// mono WAVs, then stereo-to-5.1 upmix, then loudness normalization. Same order
/// as the CLI create path. Intermediates go under `<output>/audio_work`.
fn prepare_audio(
    job: &JobConfig,
    output: &std::path::Path,
    log: impl Fn(&str),
) -> Result<Option<PathBuf>, String> {
    let work_dir = output.join("audio_work");
    let mut audio_path = job
        .audio_path
        .as_ref()
        .filter(|a| !a.is_empty())
        .map(PathBuf::from);

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

    Ok(audio_path)
}

fn build_dcp_config(
    job: &JobConfig,
    j2k_dir: PathBuf,
    right_eye_dir: Option<PathBuf>,
    audio_path: Option<PathBuf>,
    sign_language_main_channels: Option<u32>,
    reel_split_frames: Vec<u64>,
) -> dcpwizard_core::dcp::DcpConfig {
    let standard = match job.standard.as_str() {
        "interop" => dcpwizard_core::Standard::Interop,
        _ => dcpwizard_core::Standard::Smpte,
    };

    let resolution = if job.resolution.contains("4k") {
        dcpwizard_core::Resolution::FourK
    } else {
        dcpwizard_core::Resolution::TwoK
    };
    let (container_width, container_height) = container_of(&job.resolution);

    let content_type = match job.content_kind.as_str() {
        "trailer" => dcpwizard_core::ContentType::Trailer,
        "test" => dcpwizard_core::ContentType::Test,
        "short" => dcpwizard_core::ContentType::Short,
        "advertisement" => dcpwizard_core::ContentType::Advertisement,
        "episode" => dcpwizard_core::ContentType::Episode,
        _ => dcpwizard_core::ContentType::Feature,
    };

    let (frame_rate_num, frame_rate_den) = frame_rate_of(&job.framerate);

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
        atmos_path: job.atmos.as_ref().map(PathBuf::from),
        subtitle_path: job.subtitle.as_ref().map(PathBuf::from),
        subtitle_language: job.subtitle_language.clone(),
        ccap_path: job.ccap.as_ref().map(PathBuf::from),
        ccap_language: job.ccap_language.clone(),
        pad_head: job.pad_head.clone(),
        pad_tail: job.pad_tail.clone(),
        pad_color: job.pad_color.clone(),
        reel_length_minutes: job.reel_length_minutes,
        reel_split_frames,
        sign_language_lang: job.sign_language_tag.clone(),
        sign_language_main_channels,
        ..Default::default()
    }
}

fn run_job(app: &AppHandle, job: &JobConfig) -> Result<String, String> {
    let queue = app.state::<JobQueue>();
    let cancel = queue.cancel.clone();
    let pause = queue.pause.clone();

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

    let (fps_num, _) = frame_rate_of(&job.framerate);

    // reel boundaries before the encode: a source with no chapter marks should
    // fail now, not after an hour of J2K.
    let reel_split_frames = resolve_reel_splits(job, fps_num)?;
    if !reel_split_frames.is_empty() {
        log_to(
            &log_file,
            &format!("[PACKAGE] Reel boundaries: {reel_split_frames:?}"),
        );
    }

    // Map the target bandwidth (Mbps) to a J2K compression ratio, matching the
    // CLI convention (raw = w*h*36 bits/frame). Only honoured for video input;
    // image/J2K sequences fall back to the encoder default.
    let compression_ratio = dcpwizard_core::probe::probe_video(&job.video_path)
        .map(|info| {
            let fps = (fps_num as f64).max(1.0);
            let raw_bits = info.width as f64 * info.height as f64 * 36.0;
            let target_bits = (job.bandwidth as f64 * 1_000_000.0) / fps;
            (raw_bits / target_bits).max(1.0)
        })
        .unwrap_or(10.0);

    // Encode using shared pipeline
    let job_id = job.id;
    let app_ref = app.clone();
    let log_ref = log_file.clone();
    let encode_result = postkit::pipeline::run_encode_with_ratio(
        &job.video_path,
        output,
        compression_ratio,
        fps_num,
        &cancel,
        &pause,
        |p| {
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
        },
        |msg| log_to(&log_ref, msg),
    )?;

    // Stereoscopic 3D: encode the right eye into its own subdir at the same
    // ratio/fps (the main input is the left eye).
    let right_eye_dir = if let Some(re) = job.right_eye.as_deref() {
        log_to(&log_file, &format!("[ENCODE] Right eye: {re}"));
        let re_out = output.join("right");
        let log_ref = log_file.clone();
        let re_result = postkit::pipeline::run_encode_with_ratio(
            std::path::Path::new(re),
            &re_out,
            compression_ratio,
            fps_num,
            &cancel,
            &pause,
            |_p| {},
            |msg| log_to(&log_ref, msg),
        )?;
        Some(re_result.j2k_dir)
    } else {
        None
    };

    let audio_path = prepare_audio(job, output, |msg| log_to(&log_file, msg))?;

    // sign-language video (ISDCF Doc 13): pack VP9 onto channel 15, replacing
    // the sound track with the combined 16-channel WAV.
    let (audio_path, sign_language_main_channels) = match job.sign_language_video.as_deref() {
        Some(video) => {
            log_to(&log_file, &format!("[AUDIO] Sign language: {video}"));
            let frames = if encode_result.frames_encoded > 0 {
                encode_result.frames_encoded
            } else {
                count_frames(&encode_result.j2k_dir)
            };
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

    let config = build_dcp_config(
        job,
        encode_result.j2k_dir.clone(),
        right_eye_dir,
        audio_path,
        sign_language_main_channels,
        reel_split_frames,
    );

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
        return Err(format!("DCP packaging failed (rc={rc})"));
    }
    log_to(&log_file, "[PACKAGE] Done");

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
        emit_progress(app, job.id, "validate", &summary, 0, 0, 0.0, 0.0, 100.0);
    }

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
    use dcpwizard_core::mxf_wrap::AudioInputOrder;
    use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

    fn test_job() -> JobConfig {
        JobConfig {
            id: 1,
            video_path: PathBuf::from("/in/movie.mov"),
            title: "Test".into(),
            output_dir: PathBuf::from("/out"),
            audio_path: None,
            validate: false,
            standard: "smpte".into(),
            resolution: "2k-flat".into(),
            framerate: "24".into(),
            bandwidth: 250,
            colour: "xyz".into(),
            content_kind: "feature".into(),
            encrypt: false,
            key_out: None,
            channels: "5.1".into(),
            right_eye: None,
            atmos: None,
            subtitle: None,
            subtitle_language: "en".into(),
            ccap: None,
            ccap_language: "en".into(),
            loudness_target: None,
            true_peak_ceiling: None,
            audio_channel_dir: None,
            audio_input_order: AudioInputOrder::Canonical51,
            sign_language_video: None,
            sign_language_tag: None,
            pad_head: None,
            pad_tail: None,
            pad_color: None,
            upmix: None,
            reel_length_minutes: 0,
            reel_split_frames: Vec::new(),
            split_chapters: false,
            versions: Vec::new(),
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

        let routed = prepare_audio(&job, dir.path(), |_| {}).unwrap().unwrap();
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

    #[test]
    fn audio_file_is_untouched_without_routing_or_loudness() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("sound.wav");
        write_mono(&wav, 1234, 16);

        let mut job = test_job();
        job.audio_path = Some(wav.to_string_lossy().into_owned());

        let prepared = prepare_audio(&job, dir.path(), |_| {}).unwrap();
        assert_eq!(prepared, Some(wav));
        assert!(!dir.path().join("audio_work").exists());
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

        let upmixed = prepare_audio(&job, dir.path(), |_| {}).unwrap().unwrap();
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
}
