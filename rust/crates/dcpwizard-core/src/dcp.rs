use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// DCP creation configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DcpConfig {
    pub title: String,
    pub standard: crate::Standard,
    pub resolution: crate::Resolution,
    pub content_type: crate::ContentType,
    pub frame_rate_num: u32,
    pub frame_rate_den: u32,
    pub max_bitrate_mbps: u32,
    pub encrypt: bool,
    /// Where to write the content keys when `encrypt` is set. Required for
    /// encryption; keys are never written next to the DCP by default.
    pub key_out: Option<PathBuf>,
    pub stereo_3d: bool,
    /// Container the picture is masked to (e.g. 2048x858 scope, 1998x1080 flat),
    /// declared as the CPL's active area. Zero means the whole coded raster is
    /// active. It can never exceed the raster the encoder produced.
    pub container_width: u32,
    pub container_height: u32,
    pub output_dir: PathBuf,
    /// Directory containing J2K frames to wrap into picture MXF
    pub j2k_dir: Option<PathBuf>,
    /// Audio WAV file to wrap into sound MXF
    pub audio_path: Option<PathBuf>,
    /// Declared channel order for a six-channel input WAV. DCPwizard never
    /// guesses this order.
    pub audio_input_order: crate::mxf_wrap::AudioInputOrder,
    /// How many channels the packaged sound track carries, filled with silence
    /// past what the content holds. None widens a 5.1 source to 16 and packages
    /// every other source at its own width.
    #[serde(default)]
    pub audio_channels: Option<u32>,
    /// SRT subtitle file to convert, or supplied SMPTE timed-text XML to wrap as
    /// a subtitle track.
    pub subtitle_path: Option<PathBuf>,
    /// Subtitle language code (default "en").
    pub subtitle_language: String,
    /// Placement / RTL / wrap / font options for the subtitle conversion. Ignored
    /// for a supplied SMPTE DCST XML (wrapped unchanged).
    #[serde(default)]
    pub subtitle_opts: crate::subtitle::SubtitleOptions,
    /// Closed-caption (ST 429-12) input, wrapped as timed text with a
    /// ST 429-12 ClosedCaption CPL role. Distinct from the open `subtitle_path`; same
    /// input formats (SRT/styled -> DCST, or a supplied SMPTE DCST passthrough).
    #[serde(default)]
    pub ccap_path: Option<PathBuf>,
    /// Closed-caption language code (default "en").
    #[serde(default)]
    pub ccap_language: String,
    /// Split the DCP into reels of at most this many minutes each. Zero (default)
    /// keeps the single-reel path.
    pub reel_length_minutes: u32,
    /// Right-eye J2K frames for a stereoscopic (ST 429-10) DCP; the main `j2k_dir`
    /// is the left eye. Present implies a 3D package.
    pub right_eye_dir: Option<PathBuf>,
    /// Dolby Atmos / DCData bitstream to wrap as a ST 429-18 auxiliary track.
    pub atmos_path: Option<PathBuf>,
    /// Sound channel index carrying the Hearing Impaired (HI) accessibility track.
    pub hi_channel: Option<u32>,
    /// Sound channel index carrying the Visually Impaired (VI-N) narration track.
    pub vi_channel: Option<u32>,
    /// Black-frame + silence padding prepended at the head of the program. A
    /// duration with a unit: frames (`48f`) or seconds (`2s`). Head padding
    /// shifts the program, so supplied SRT subtitles are re-timed by this offset.
    pub pad_head: Option<String>,
    /// Black-frame + silence padding appended at the tail of the program. Same
    /// syntax as `pad_head`.
    pub pad_tail: Option<String>,
    /// Head/tail trim the caller already applied to the picture frames and the
    /// sound. Timed text still carries the source's timing, so it is re-timed
    /// here; trim is applied before `pad_head`, never after.
    #[serde(default)]
    pub source_trim: crate::subtitle::SourceTrim,
    /// Background/pad colour as `#RRGGBB` sRGB. Absent = black. Applied to the
    /// head/tail pad frames (run through the DCDM transform before J2K encoding).
    #[serde(default)]
    pub pad_color: Option<String>,
    /// Explicit reel-split boundaries as frame numbers (from --split-at timecodes
    /// or --split-chapters). Empty = no explicit split. Mutually exclusive with
    /// `reel_length_minutes`.
    #[serde(default)]
    pub reel_split_frames: Vec<u64>,
    /// RFC 5646 sign-language tag (ISDCF Doc 13). When set, the sound track's
    /// channel 15 is labelled SLVS and the CPL carries the SignLanguageVideo
    /// ExtensionMetadata. `audio_path` must already be the combined 16-channel
    /// WAV with the packed VP9 program on channel 15 (see `sign_language`).
    #[serde(default)]
    pub sign_language_lang: Option<String>,
    /// Leading main-audio channel count under an SLVS track, for the SLVS MCA
    /// config's soundfield layout (0 = SLVS-only, no leading soundfield).
    #[serde(default)]
    pub sign_language_main_channels: Option<u32>,
    /// RFC 5646 language the main soundtrack is spoken in. Names the audio in an
    /// ISDCF content title.
    #[serde(default)]
    pub audio_language: Option<String>,
    /// Certification ratings, written into the CPL's RatingList.
    #[serde(default)]
    pub ratings: Vec<crate::isdcf_name::Rating>,
    /// Content versions. The first is the CPL's ContentVersion LabelText.
    #[serde(default)]
    pub content_versions: Vec<String>,
    /// ST 429-16 composition identity, written into the CompositionMetadataAsset.
    #[serde(default)]
    pub release_territory: Option<String>,
    #[serde(default)]
    pub version_number: Option<u32>,
    #[serde(default)]
    pub chain: Option<String>,
    #[serde(default)]
    pub distributor: Option<String>,
    #[serde(default)]
    pub facility: Option<String>,
    #[serde(default)]
    pub luminance: Option<crate::cpl::Luminance>,
    /// DCI HDR Addendum: wrap the picture MXF with TransferCharacteristic=ST 2084
    /// (PQ) and P3-D65 colour primaries. Source must already be PQ/DCI.
    #[serde(default)]
    pub hdr_dci: bool,
    /// Signer for the CPL and PKL ds:Signature. None leaves them unsigned.
    #[serde(default)]
    pub signer: Option<crate::package_signature::PackageSigner>,
    /// Composition markers as `LABEL=timecode` specs (e.g. `FFEC=00:58:12:03`),
    /// written into the CPL as the first reel's MainMarkers asset. Empty gives
    /// the default FFOC/LFOC pair.
    #[serde(default)]
    pub markers: Vec<String>,
}

/// Validate custom container dimensions against the resolution bounds.
///
/// Both dims must be positive, even, and fit within the container for the chosen
/// resolution: 2048x1080 (2K) or 4096x2160 (4K).
pub fn validate_container_dims(width: u32, height: u32, is_4k: bool) -> Result<(), String> {
    let (max_w, max_h) = if is_4k { (4096, 2160) } else { (2048, 1080) };
    if width == 0 || height == 0 {
        return Err("container dimensions must be positive".into());
    }
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(format!(
            "container dimensions {width}x{height} must both be even"
        ));
    }
    if width > max_w || height > max_h {
        return Err(format!(
            "container dimensions {width}x{height} exceed the {} container {max_w}x{max_h}",
            if is_4k { "4K" } else { "2K" }
        ));
    }
    Ok(())
}

/// Dolby Atmos IAB bitstream data-essence UL, as used in real Atmos DCP AuxData.
const ATMOS_DATA_TYPE_UL: &str = "urn:smpte:ul:060e2b34.04010105.0e090604.00000000";

/// Wrap a timed-text input into an MXF: SRT/styled formats are converted to
/// DCST (cues moved onto the packaged `timing`, fonts/PNGs embedded), a supplied
/// SMPTE DCST is wrapped unchanged. Returns the track duration, or None on failure
/// (already logged). Used for the closed-caption track; the open subtitle path
/// stays inline in create_dcp.
fn wrap_timed_text_track(
    wrap: crate::mxf_wrap::MxfWrapConfig,
    lang: &str,
    timing: crate::subtitle::CueTiming,
    opts: &crate::subtitle::SubtitleOptions,
) -> Option<u64> {
    // temp DCST and staged resources land next to the output MXF
    let work_dir = wrap
        .output_mxf
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let kind = match crate::subtitle::detect_subtitle_kind(&wrap.input_path) {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("{e}");
            return None;
        }
    };
    if kind == crate::subtitle::SubtitleInputKind::SmpteDcstPassthrough {
        return match crate::mxf_wrap::wrap_mxf_result(&wrap) {
            Some(t) => Some(t.duration),
            None => {
                tracing::error!("Failed to wrap timed-text MXF");
                None
            }
        };
    }
    let dcst_path = wrap.output_mxf.with_extension("dcst.xml");
    let prepared = match crate::subtitle::prepare_subtitle_track(
        &wrap.input_path,
        timing,
        lang,
        wrap.frame_rate,
        opts,
        &dcst_path,
    ) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("closed-caption conversion failed: {e}");
            return None;
        }
    };
    let wrapped = crate::mxf_wrap::wrap_timed_text_resources(
        &prepared.dcst_path,
        &prepared.resources,
        &wrap.output_mxf,
        wrap.frame_rate,
        wrap.asset_uuid,
        None,
        wrap.encryption,
    );
    let _ = std::fs::remove_file(&dcst_path);
    for (p, _) in &prepared.resources {
        if p.starts_with(work_dir) {
            let _ = std::fs::remove_file(p);
        }
    }
    match wrapped {
        Some(t) => Some(t.duration),
        None => {
            tracing::error!("Failed to wrap timed-text MXF");
            None
        }
    }
}

/// Coarse stage progress + cooperative cancellation for a running create. The
/// job queue implements this so a create reports real per-stage progress and can
/// be cancelled between stages; the plain `create_dcp` path uses a no-op sink.
/// create_dcp wraps a pre-encoded J2K dir, so progress is stage-based (wrap /
/// package), not per-frame: the per-frame encode pipeline (postkit::pipeline)
/// runs earlier, outside this path.
pub trait ProgressSink {
    fn stage(&self, percent: u32, message: &str);
    fn cancelled(&self) -> bool;
}

struct NoProgress;
impl ProgressSink for NoProgress {
    fn stage(&self, _percent: u32, _message: &str) {}
    fn cancelled(&self) -> bool {
        false
    }
}

/// Create a complete DCP from the given configuration.
///
/// This orchestrates the full DCP creation pipeline:
/// 1. Wrap J2K frames and audio into MXF
/// 2. Generate CPL, PKL, ASSETMAP
/// 3. Optionally encrypt
pub fn create_dcp(config: &DcpConfig) -> i32 {
    create_dcp_with_progress(config, &NoProgress)
}

/// As [`create_dcp`], reporting coarse stage progress and honouring cooperative
/// cancellation through `progress`. Returns -2 when cancelled between stages.
pub fn create_dcp_with_progress(config: &DcpConfig, progress: &dyn ProgressSink) -> i32 {
    if progress.cancelled() {
        return -2;
    }
    progress.stage(5, "starting");
    tracing::info!(
        "Creating DCP: {} ({})",
        config.title,
        if config.standard == crate::Standard::Smpte {
            "SMPTE"
        } else {
            "Interop"
        }
    );

    let Some(j2k_dir) = config.j2k_dir.as_ref() else {
        tracing::error!("A J2K input directory is required");
        return -1;
    };
    if !j2k_dir.is_dir() {
        tracing::error!("J2K input directory does not exist: {}", j2k_dir.display());
        return -1;
    }

    // Reject a bad signer before anything is written, so signing cannot fail
    // once the CPL is on disk and leave a half-signed package.
    if let Some(signer) = config.signer.as_ref()
        && let Err(e) = signer.check_usable()
    {
        tracing::error!("{e}");
        return -1;
    }

    if let Err(e) = std::fs::create_dir_all(&config.output_dir) {
        tracing::error!("Failed to create output directory: {e}");
        return -1;
    }

    // Fail early if the essence won't fit: the wrapped MXFs are ~the size of the
    // J2K frames plus audio/atmos, so check that against the output filesystem.
    let mut required = crate::free_space::path_size(j2k_dir);
    if let Some(dir) = config.right_eye_dir.as_ref() {
        required += crate::free_space::path_size(dir);
    }
    for extra in [config.audio_path.as_ref(), config.atmos_path.as_ref()]
        .into_iter()
        .flatten()
    {
        required += crate::free_space::path_size(extra);
    }
    if let Err(e) = crate::free_space::check_destination_space(&config.output_dir, required) {
        tracing::error!("{e}");
        return -1;
    }

    // Fail before doing any work if we'd have nowhere safe to put the keys.
    if config.encrypt && config.key_out.is_none() {
        tracing::error!(
            "--key-out is required when encrypting; keys are never written next to the DCP"
        );
        return -1;
    }
    let fps = if config.frame_rate_num > 0 {
        config.frame_rate_num
    } else {
        24
    };

    // reject an illegal frame rate / resolution combo before wrapping anything
    let is_4k = config.resolution == crate::Resolution::FourK;
    if let Err(e) =
        crate::hfr::validate_fps_resolution(fps, is_4k, config.standard == crate::Standard::Smpte)
    {
        tracing::error!("{e}");
        return -1;
    }

    let stereoscopic = config.right_eye_dir.is_some();

    // ── Head/tail padding: parse durations and reject unsound combinations ──
    let head_frames = match config.pad_head.as_deref() {
        Some(spec) => match crate::pad::parse_pad_frames(spec, fps) {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("--pad-head: {e}");
                return -1;
            }
        },
        None => 0,
    };
    let tail_frames = match config.pad_tail.as_deref() {
        Some(spec) => match crate::pad::parse_pad_frames(spec, fps) {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("--pad-tail: {e}");
                return -1;
            }
        },
        None => 0,
    };
    // background/pad colour (default black); parse before any encoding
    let pad_rgb = match config.pad_color.as_deref() {
        Some(spec) => match crate::pad::parse_pad_color(spec) {
            Ok(rgb) => rgb,
            Err(e) => {
                tracing::error!("--pad-color: {e}");
                return -1;
            }
        },
        None => [0, 0, 0],
    };

    let cue_timing = crate::subtitle::CueTiming {
        trim: config.source_trim,
        pad_head_frames: head_frames,
    };
    // supplied SMPTE XML carries authored timing we will not rewrite; every
    // parsed format (SRT/ASS/PAC/... ) is regenerated so it can be re-timed.
    let supplied_xml = |path: &Option<PathBuf>| {
        path.as_ref()
            .filter(|p| p.exists())
            .map(|p| {
                matches!(
                    crate::subtitle::detect_subtitle_kind(p),
                    Ok(crate::subtitle::SubtitleInputKind::SmpteDcstPassthrough)
                )
            })
            .unwrap_or(false)
    };
    let supplied_subtitle_xml = supplied_xml(&config.subtitle_path);
    if config.source_trim.is_active() && (supplied_subtitle_xml || supplied_xml(&config.ccap_path))
    {
        tracing::error!(
            "trimming cannot re-time supplied SMPTE timed-text XML; supply SRT or another parsable \
             format, or trim nothing"
        );
        return -1;
    }

    let padding = head_frames + tail_frames > 0;
    if padding {
        if config.reel_length_minutes > 0 || !config.reel_split_frames.is_empty() {
            tracing::error!(
                "head/tail padding is not supported with reel splitting (--reel-length / --split-at)"
            );
            return -1;
        }
        if stereoscopic {
            tracing::error!("head/tail padding is not supported with stereoscopic 3D");
            return -1;
        }
        if config.atmos_path.is_some() {
            tracing::error!(
                "head/tail padding is not supported with Atmos: the auxiliary track cannot be re-timed soundly this pass"
            );
            return -1;
        }
        if head_frames > 0 && supplied_subtitle_xml {
            tracing::error!(
                "head padding cannot re-time supplied SMPTE subtitle XML; supply SRT to shift, or pad only the tail"
            );
            return -1;
        }
    }

    let prepared_audio = match config.audio_path.as_ref().filter(|path| path.exists()) {
        Some(path) => {
            let output = config
                .output_dir
                .join(format!(".dcpwizard_audio_{}.wav", uuid::Uuid::new_v4()));
            match crate::mxf_wrap::prepare_packaged_channels(
                path,
                &output,
                config.audio_input_order,
                config.audio_channels,
            ) {
                Ok(true) => Some(output),
                Ok(false) => None,
                Err(e) => {
                    tracing::error!("audio preparation failed: {e}");
                    return -1;
                }
            }
        }
        None => None,
    };

    // multi-reel path is opt-in; the single-reel path below is unchanged
    if config.reel_length_minutes > 0 || !config.reel_split_frames.is_empty() {
        if let Err(e) =
            crate::preflight::check_reel_split_support(&crate::preflight::ReelSplitContent {
                splitting: true,
                stereo_3d: stereoscopic,
                atmos: config.atmos_path.is_some(),
                hdr_dci: config.hdr_dci,
                markers: !config.markers.is_empty(),
            })
        {
            tracing::error!("{e}");
            return -1;
        }
        let mut reel_config = config.clone();
        if let Some(path) = prepared_audio.as_ref() {
            reel_config.audio_path = Some(path.clone());
        }
        progress.stage(10, "assembling reels");
        let code = crate::reel::create_multi_reel_dcp(&reel_config, fps);
        if let Some(path) = prepared_audio {
            let _ = std::fs::remove_file(path);
        }
        if code == 0 {
            progress.stage(100, "done");
        }
        return code;
    }

    if progress.cancelled() {
        return -2;
    }
    progress.stage(15, "wrapping picture");
    // ── Wrap picture MXF ──────────────────────────────────────────────
    let picture_uuid = uuid::Uuid::new_v4();
    let picture_mxf_name = format!("picture_{picture_uuid}.mxf");
    let picture_mxf_path = config.output_dir.join(&picture_mxf_name);
    // set from the (left-eye) frame count in the wrap block below
    let picture_duration: u64;

    // Mint content keys up front so the essence is encrypted at wrap time and
    // the PKL/ASSETMAP hashes below are taken from the final encrypted files.
    let picture_key = match crate::versions::mint_key(
        config,
        crate::encrypt::KeyType::Mdik,
        &picture_uuid.to_string(),
    ) {
        Ok(k) => k,
        Err(()) => return -1,
    };

    {
        let left_frames = crate::reel::collect_frames(j2k_dir);
        let content_count = left_frames.len() as u64;
        if content_count == 0 {
            tracing::error!("J2K input directory contains no codestreams");
            return -1;
        }

        let encryption = picture_key
            .as_ref()
            .map(|k| postkit::mxf_wrap::MxfEncryption {
                content_key: k.key,
                key_id: k.key_id,
            });

        if padding {
            // encode one black frame at the content's pixel dimensions, then repeat
            // its codestream for every padded frame (frame-wrapped MXF reuses it)
            let (bw, bh) = match crate::pad::read_j2k_dimensions(&left_frames[0]) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("{e}");
                    return -1;
                }
            };
            let black = config
                .output_dir
                .join(format!(".dcpwizard_pad_{picture_uuid}.j2c"));
            if let Err(e) = crate::pad::generate_solid_frame(bw, bh, fps, pad_rgb, &black) {
                tracing::error!("{e}");
                return -1;
            }
            let mut files =
                Vec::with_capacity(head_frames as usize + left_frames.len() + tail_frames as usize);
            files.extend(std::iter::repeat_n(black.clone(), head_frames as usize));
            files.extend(left_frames.iter().cloned());
            files.extend(std::iter::repeat_n(black.clone(), tail_frames as usize));
            picture_duration = files.len() as u64;
            let wrapped = if config.hdr_dci {
                crate::mxf_wrap::wrap_j2k_hdr_files(
                    files,
                    &picture_mxf_path,
                    fps,
                    encryption,
                    Some(*picture_uuid.as_bytes()),
                )
            } else {
                crate::mxf_wrap::wrap_mxf_files(
                    files,
                    &picture_mxf_path,
                    crate::mxf_wrap::MxfType::J2kPicture,
                    fps,
                    encryption,
                    None,
                    Some(*picture_uuid.as_bytes()),
                )
            };
            let _ = std::fs::remove_file(&black);
            if wrapped.is_none() {
                tracing::error!("Failed to wrap padded picture MXF");
                return -1;
            }
            tracing::info!(
                "Picture MXF: {picture_mxf_name} ({picture_duration} frames: {head_frames} head + {content_count} content + {tail_frames} tail)"
            );
        } else if stereoscopic {
            if config.hdr_dci {
                tracing::error!("--hdr-dci is not supported for stereoscopic (3D) DCPs");
                return -1;
            }
            picture_duration = content_count;
            // left eye is j2k_dir, right eye its own dir; both must match frame counts
            let right_dir = config.right_eye_dir.as_ref().unwrap();
            let right_frames = crate::reel::collect_frames(right_dir);
            if right_frames.len() as u64 != picture_duration {
                tracing::error!(
                    "3D eye frame count mismatch: left={picture_duration}, right={}",
                    right_frames.len()
                );
                return -1;
            }
            // ST 429-10 legal 3D rates: the essence runs at 2x the edit rate, so
            // the composition edit rate must itself be a legal DCP rate.
            if crate::mxf_wrap::wrap_stereoscopic_files(
                left_frames,
                right_frames,
                &picture_mxf_path,
                fps,
                encryption,
                Some(*picture_uuid.as_bytes()),
            )
            .is_none()
            {
                tracing::error!("Failed to wrap stereoscopic picture MXF");
                return -1;
            }
            tracing::info!(
                "Stereoscopic picture MXF: {picture_mxf_name} ({picture_duration} frame pairs)"
            );
        } else if config.hdr_dci {
            picture_duration = content_count;
            if crate::mxf_wrap::wrap_j2k_hdr_files(
                crate::reel::collect_frames(j2k_dir),
                &picture_mxf_path,
                fps,
                encryption,
                Some(*picture_uuid.as_bytes()),
            )
            .is_none()
            {
                tracing::error!("Failed to wrap DCI HDR picture MXF");
                return -1;
            }
            tracing::info!("Picture MXF: {picture_mxf_name} ({picture_duration} frames, DCI HDR)");
        } else {
            picture_duration = content_count;
            let wrap_config = crate::mxf_wrap::MxfWrapConfig {
                input_path: j2k_dir.clone(),
                output_mxf: picture_mxf_path.clone(),
                mxf_type: crate::mxf_wrap::MxfType::J2kPicture,
                frame_rate: fps,
                encryption,
                mca_config: None,
                asset_uuid: Some(*picture_uuid.as_bytes()),
            };
            if crate::mxf_wrap::wrap_mxf(&wrap_config) != 0 {
                tracing::error!("Failed to wrap picture MXF");
                return -1;
            }
            tracing::info!("Picture MXF: {picture_mxf_name} ({picture_duration} frames)");
        }
    }

    if progress.cancelled() {
        return -2;
    }
    progress.stage(55, "wrapping sound");
    // ── Wrap sound MXF ────────────────────────────────────────────────
    let sound_uuid = uuid::Uuid::new_v4();
    let sound_mxf_name = format!("sound_{sound_uuid}.mxf");
    let sound_mxf_path = config.output_dir.join(&sound_mxf_name);
    let mut has_sound = false;
    let sound_duration = picture_duration; // match picture duration
    let mut sound_key = None;
    // sound layout for the SMPTE CompositionMetadataAsset (ST 429-16)
    let mut main_sound = None;

    if let Some(audio_path) = prepared_audio.as_ref().or(config.audio_path.as_ref())
        && audio_path.exists()
    {
        sound_key = match crate::versions::mint_key(
            config,
            crate::encrypt::KeyType::Mdak,
            &sound_uuid.to_string(),
        ) {
            Ok(k) => k,
            Err(()) => return -1,
        };
        // labels follow the content, counts follow the container: the fill added
        // silent channels the soundfield must not claim, so the source is probed
        // for the layout and the prepared file for the width
        let content_source = config.audio_path.as_deref().unwrap_or(audio_path.as_path());
        let (content_channels, packaged_channels) = match (
            crate::mxf_wrap::wav_channels(content_source),
            crate::mxf_wrap::wav_channels(audio_path),
        ) {
            (Ok(content), Ok(packaged)) => (content as u32, packaged as u32),
            (Err(e), _) | (_, Err(e)) => {
                tracing::error!("{e}");
                return -1;
            }
        };
        // sign-language: override the MCA config so channel 15 is labelled SLVS
        // (the audio is already the combined 16-channel track). Otherwise derive
        // the layout from the channel count plus any HI/VI channels.
        let mca_labels = if let Some(lang) = config.sign_language_lang.as_ref() {
            let main_ch = config.sign_language_main_channels.unwrap_or(0);
            let main =
                crate::mxf_wrap::build_mca_config(main_ch, main_ch, None, None).unwrap_or_default();
            Some(crate::sign_language::slvs_mca_config(
                &main,
                main_ch as usize,
                lang,
            ))
        } else {
            crate::mxf_wrap::build_mca_config(
                content_channels,
                packaged_channels,
                config.hi_channel,
                config.vi_channel,
            )
        };
        let mca_config = mca_labels.map(|labels| postkit::mxf_wrap::McaConfig {
            labels,
            spoken_language: config.audio_language.clone(),
        });
        // MainSoundConfiguration for the CPL metadata asset, from the same counts
        // as the MCA labels (silent fill channels become '-').
        if let Some(configuration) = crate::cpl::main_sound_configuration(
            content_channels,
            packaged_channels,
            config.hi_channel,
            config.vi_channel,
        ) {
            let sample_rate = crate::mxf_wrap::wav_sample_rate(audio_path).unwrap_or(48000);
            main_sound = Some(crate::cpl::MainSound {
                configuration,
                sample_rate,
            });
        }
        // when padding, extend the PCM with silence so the sound stays aligned
        // with the padded picture (sample-accurate at frame edges)
        let mut padded_audio: Option<PathBuf> = None;
        let wrap_source = if padding {
            let sample_rate = crate::mxf_wrap::wav_sample_rate(audio_path).unwrap_or(48000);
            if let Err(e) = crate::pad::check_frame_aligned_sample_rate(sample_rate, fps) {
                tracing::error!("{e}");
                return -1;
            }
            let spf = (sample_rate / fps) as u64;
            let out = config
                .output_dir
                .join(format!(".dcpwizard_padded_{sound_uuid}.wav"));
            if let Err(e) = crate::pad::pad_wav_with_silence(
                audio_path,
                head_frames * spf,
                tail_frames * spf,
                &out,
            ) {
                tracing::error!("audio padding failed: {e}");
                return -1;
            }
            padded_audio = Some(out.clone());
            out
        } else {
            audio_path.clone()
        };
        let wrap_config = crate::mxf_wrap::MxfWrapConfig {
            input_path: wrap_source,
            output_mxf: sound_mxf_path.clone(),
            mxf_type: crate::mxf_wrap::MxfType::PcmAudio,
            frame_rate: fps,
            encryption: sound_key
                .as_ref()
                .map(|k| postkit::mxf_wrap::MxfEncryption {
                    content_key: k.key,
                    key_id: k.key_id,
                }),
            mca_config,
            asset_uuid: Some(*sound_uuid.as_bytes()),
        };
        let wrap_code = crate::mxf_wrap::wrap_mxf(&wrap_config);
        if let Some(tmp) = padded_audio {
            let _ = std::fs::remove_file(tmp);
        }
        if wrap_code != 0 {
            tracing::error!("Failed to wrap sound MXF");
            return -1;
        }
        has_sound = true;
        tracing::info!("Sound MXF: {sound_mxf_name}");
    }

    // ── Wrap subtitle (SMPTE timed text) MXF ──────────────────────────
    let subtitle_uuid = uuid::Uuid::new_v4();
    let subtitle_mxf_name = format!("subtitle_{subtitle_uuid}.mxf");
    let subtitle_mxf_path = config.output_dir.join(&subtitle_mxf_name);
    let mut has_subtitle = false;
    let mut subtitle_duration = 0u64;
    let mut subtitle_key = None;
    let subtitle_lang = if config.subtitle_language.is_empty() {
        "en"
    } else {
        &config.subtitle_language
    };

    if let Some(subtitle_path) = config.subtitle_path.as_ref()
        && subtitle_path.exists()
    {
        let kind = match crate::subtitle::detect_subtitle_kind(subtitle_path) {
            Ok(k) => k,
            Err(e) => {
                tracing::error!("{e}");
                return -1;
            }
        };
        subtitle_key = match crate::versions::mint_key(
            config,
            crate::encrypt::KeyType::Mdsk,
            &subtitle_uuid.to_string(),
        ) {
            Ok(k) => k,
            Err(()) => return -1,
        };
        let track = if kind == crate::subtitle::SubtitleInputKind::SmpteDcstPassthrough {
            // preserve authored SMPTE XML unchanged (placement, styling, timing)
            let wrap_config = crate::mxf_wrap::MxfWrapConfig {
                input_path: subtitle_path.clone(),
                output_mxf: subtitle_mxf_path.clone(),
                mxf_type: crate::mxf_wrap::MxfType::TimedText,
                frame_rate: fps,
                encryption: subtitle_key.as_ref().map(crate::reel::mxf_enc),
                mca_config: None,
                asset_uuid: Some(*subtitle_uuid.as_bytes()),
            };
            match crate::mxf_wrap::wrap_mxf_result(&wrap_config) {
                Some(t) => t,
                None => {
                    tracing::error!("Failed to wrap subtitle MXF");
                    return -1;
                }
            }
        } else {
            // SRT and the styled formats: convert to DCST with placement/RTL/wrap/
            // font options, shifting cues by head_frames, and embed any font/PNGs.
            let dcst_path = config
                .output_dir
                .join(format!("subtitle_{subtitle_uuid}.xml"));
            let prepared = match crate::subtitle::prepare_subtitle_track(
                subtitle_path,
                cue_timing,
                subtitle_lang,
                fps,
                &config.subtitle_opts,
                &dcst_path,
            ) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("Subtitle conversion failed: {e}");
                    return -1;
                }
            };
            let wrapped = crate::mxf_wrap::wrap_timed_text_resources(
                &prepared.dcst_path,
                &prepared.resources,
                &subtitle_mxf_path,
                fps,
                Some(*subtitle_uuid.as_bytes()),
                None,
                subtitle_key.as_ref().map(crate::reel::mxf_enc),
            );
            // the DCST and any staged font now live inside the MXF
            let _ = std::fs::remove_file(&dcst_path);
            for (p, _) in &prepared.resources {
                if p.starts_with(&config.output_dir) {
                    let _ = std::fs::remove_file(p);
                }
            }
            match wrapped {
                Some(t) => t,
                None => {
                    tracing::error!("Failed to wrap subtitle MXF");
                    return -1;
                }
            }
        };
        subtitle_duration = track.duration;
        has_subtitle = true;
        tracing::info!("Subtitle MXF: {subtitle_mxf_name}");
    }

    // ── Wrap closed-caption (ST 429-12 timed text) MXF ────────────────
    let ccap_uuid = uuid::Uuid::new_v4();
    let ccap_mxf_name = format!("ccap_{ccap_uuid}.mxf");
    let ccap_mxf_path = config.output_dir.join(&ccap_mxf_name);
    let mut has_ccap = false;
    let mut ccap_duration = 0u64;
    let mut ccap_key = None;
    let ccap_lang = if config.ccap_language.is_empty() {
        "en"
    } else {
        &config.ccap_language
    };
    if let Some(ccap_path) = config.ccap_path.as_ref()
        && ccap_path.exists()
    {
        ccap_key = match crate::versions::mint_key(
            config,
            crate::encrypt::KeyType::Mdsk,
            &ccap_uuid.to_string(),
        ) {
            Ok(k) => k,
            Err(()) => return -1,
        };
        match wrap_timed_text_track(
            crate::mxf_wrap::MxfWrapConfig {
                input_path: ccap_path.clone(),
                output_mxf: ccap_mxf_path.clone(),
                mxf_type: crate::mxf_wrap::MxfType::TimedText,
                frame_rate: fps,
                encryption: ccap_key.as_ref().map(crate::reel::mxf_enc),
                mca_config: None,
                asset_uuid: Some(*ccap_uuid.as_bytes()),
            },
            ccap_lang,
            cue_timing,
            &config.subtitle_opts.for_closed_caption(),
        ) {
            Some(d) => {
                ccap_duration = d;
                has_ccap = true;
                tracing::info!("Closed-caption MXF: {ccap_mxf_name}");
            }
            None => return -1,
        }
    }

    // ── Wrap Atmos / DCData auxiliary MXF (ST 429-18) ─────────────────
    let atmos_uuid = uuid::Uuid::new_v4();
    let atmos_mxf_name = format!("atmos_{atmos_uuid}.mxf");
    let atmos_mxf_path = config.output_dir.join(&atmos_mxf_name);
    let mut aux_data: Option<crate::cpl::AuxData> = None;
    let mut atmos_key = None;

    if let Some(ref atmos_path) = config.atmos_path {
        if let Err(e) = crate::preflight::check_atmos_path(atmos_path) {
            tracing::error!("{e}");
            return -1;
        }
        atmos_key = match crate::versions::mint_key(
            config,
            crate::encrypt::KeyType::Mdek,
            &atmos_uuid.to_string(),
        ) {
            Ok(k) => k,
            Err(()) => return -1,
        };
        // the synthetic id keeps CPL/PKL/ASSETMAP consistent (same pattern as
        // picture/sound)
        let wrap_config = crate::mxf_wrap::MxfWrapConfig {
            input_path: atmos_path.clone(),
            output_mxf: atmos_mxf_path.clone(),
            mxf_type: crate::mxf_wrap::MxfType::Atmos,
            frame_rate: fps,
            encryption: atmos_key.as_ref().map(crate::reel::mxf_enc),
            mca_config: None,
            asset_uuid: Some(*atmos_uuid.as_bytes()),
        };
        let Some(track) = crate::mxf_wrap::wrap_mxf_result(&wrap_config) else {
            tracing::error!("Failed to wrap Atmos MXF");
            return -1;
        };
        if let Err(e) = crate::preflight::check_atmos_frame_count(track.duration, picture_duration)
        {
            tracing::error!("{e}");
            return -1;
        }
        aux_data = Some(crate::cpl::AuxData {
            id: atmos_uuid.to_string(),
            edit_rate_num: fps,
            edit_rate_den: 1,
            duration: track.duration,
            entry_point: 0,
            key_id: atmos_key.as_ref().map(|k| k.info.key_id.clone()),
            data_type: ATMOS_DATA_TYPE_UL.to_string(),
        });
        tracing::info!("Atmos MXF: {atmos_mxf_name} ({} frames)", track.duration);
    }

    if progress.cancelled() {
        return -2;
    }
    progress.stage(85, "writing CPL/PKL/ASSETMAP");
    // ── Generate CPL ──────────────────────────────────────────────────
    let cpl_uuid = uuid::Uuid::new_v4().to_string();
    let pkl_uuid = uuid::Uuid::new_v4().to_string();

    // the CPL declares the raster the encoder produced; the container is the
    // active area inside it
    let geometry = match crate::cpl::picture_geometry(
        j2k_dir,
        config.container_width,
        config.container_height,
    ) {
        Ok(g) => g,
        Err(e) => {
            tracing::error!("{e}");
            return -1;
        }
    };

    let markers =
        match crate::markers::markers_for_composition(&config.markers, fps, picture_duration) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("{e}");
                return -1;
            }
        };

    // hashed here rather than with the rest of the PKL entries because the CPL
    // carries the same values and is written first
    let pic_hash = crate::hash::hash_file(&picture_mxf_path).unwrap_or_default();
    let snd_hash = has_sound
        .then(|| crate::hash::hash_file(&sound_mxf_path).unwrap_or_default())
        .filter(|h| !h.is_empty());
    let subtitle_hash = has_subtitle
        .then(|| crate::hash::hash_file(&subtitle_mxf_path).unwrap_or_default())
        .filter(|h| !h.is_empty());
    let ccap_hash = has_ccap
        .then(|| crate::hash::hash_file(&ccap_mxf_path).unwrap_or_default())
        .filter(|h| !h.is_empty());

    let reel = crate::cpl::CplReel {
        reel_id: uuid::Uuid::new_v4().to_string(),
        picture_id: picture_uuid.to_string(),
        picture_hash: (!pic_hash.is_empty()).then(|| pic_hash.clone()),
        sound_hash: snd_hash.clone(),
        picture_width: geometry.stored_width,
        picture_height: geometry.stored_height,
        picture_active_width: geometry.active_width,
        picture_active_height: geometry.active_height,
        picture_edit_rate_num: fps,
        picture_edit_rate_den: 1,
        picture_duration,
        picture_entry_point: 0,
        picture_key_id: picture_key.as_ref().map(|k| k.info.key_id.clone()),
        sound_id: if has_sound {
            Some(sound_uuid.to_string())
        } else {
            None
        },
        sound_edit_rate_num: fps,
        sound_edit_rate_den: 1,
        sound_duration,
        sound_entry_point: 0,
        sound_key_id: sound_key.as_ref().map(|k| k.info.key_id.clone()),
        subtitle_id: if has_subtitle {
            Some(subtitle_uuid.to_string())
        } else {
            None
        },
        subtitle_edit_rate_num: fps,
        subtitle_edit_rate_den: 1,
        subtitle_duration,
        subtitle_entry_point: 0,
        subtitle_language: if has_subtitle {
            Some(subtitle_lang.to_string())
        } else {
            None
        },
        subtitle_hash,
        subtitle_key_id: subtitle_key.as_ref().map(|k| k.info.key_id.clone()),
        ccap_id: if has_ccap {
            Some(ccap_uuid.to_string())
        } else {
            None
        },
        ccap_edit_rate_num: fps,
        ccap_edit_rate_den: 1,
        ccap_duration,
        ccap_entry_point: 0,
        ccap_language: if has_ccap {
            Some(ccap_lang.to_string())
        } else {
            None
        },
        ccap_hash,
        ccap_key_id: ccap_key.as_ref().map(|k| k.info.key_id.clone()),
        stereoscopic,
        aux_data: aux_data.clone(),
        markers,
    };

    let cpl_path = config.output_dir.join(format!("CPL_{cpl_uuid}.xml"));
    let cpl_config = crate::cpl::CplConfig {
        title: config.title.clone(),
        content_kind: config.content_type.as_cpl_kind().into(),
        ratings: config.ratings.clone(),
        content_version_label: config.content_versions.first().cloned(),
        reels: vec![reel],
        standard: config.standard,
        main_sound,
        sign_language: config.sign_language_lang.clone(),
        release_territory: config.release_territory.clone(),
        version_number: config.version_number,
        chain: config.chain.clone(),
        distributor: config.distributor.clone(),
        facility: config.facility.clone(),
        luminance: config.luminance.clone(),
        ..Default::default()
    };
    if crate::cpl::generate_cpl(&cpl_config, &cpl_uuid, &cpl_path) != 0 {
        tracing::error!("Failed to generate CPL");
        return -1;
    }
    // Sign before the PKL hashes the file, otherwise the PKL records the hash of
    // the unsigned CPL and no longer matches what is on disk.
    if !crate::package_signature::sign_if_configured(config.signer.as_ref(), &cpl_path, "CPL") {
        return -1;
    }

    // ── Generate PKL ──────────────────────────────────────────────────
    let pkl_path = config.output_dir.join(format!("PKL_{pkl_uuid}.xml"));
    let cpl_hash = crate::hash::hash_file(&cpl_path).unwrap_or_default();
    let cpl_size = std::fs::metadata(&cpl_path).map(|m| m.len()).unwrap_or(0);
    let mut pkl_entries = vec![crate::pkl::PklEntry {
        id: cpl_uuid.clone(),
        asset_type: "text/xml".into(),
        file: cpl_path.clone(),
        hash: cpl_hash,
        size: cpl_size,
    }];

    let pic_size = std::fs::metadata(&picture_mxf_path)
        .map(|m| m.len())
        .unwrap_or(0);
    pkl_entries.push(crate::pkl::PklEntry {
        id: picture_uuid.to_string(),
        asset_type: "application/mxf".into(),
        file: picture_mxf_path.clone(),
        hash: pic_hash,
        size: pic_size,
    });
    if has_sound {
        let snd_hash = snd_hash.clone().unwrap_or_default();
        let snd_size = std::fs::metadata(&sound_mxf_path)
            .map(|m| m.len())
            .unwrap_or(0);
        pkl_entries.push(crate::pkl::PklEntry {
            id: sound_uuid.to_string(),
            asset_type: "application/mxf".into(),
            file: sound_mxf_path.clone(),
            hash: snd_hash,
            size: snd_size,
        });
    }
    if has_subtitle {
        let sub_hash = crate::hash::hash_file(&subtitle_mxf_path).unwrap_or_default();
        let sub_size = std::fs::metadata(&subtitle_mxf_path)
            .map(|m| m.len())
            .unwrap_or(0);
        pkl_entries.push(crate::pkl::PklEntry {
            id: subtitle_uuid.to_string(),
            asset_type: "application/mxf".into(),
            file: subtitle_mxf_path.clone(),
            hash: sub_hash,
            size: sub_size,
        });
    }
    if has_ccap {
        let cc_hash = crate::hash::hash_file(&ccap_mxf_path).unwrap_or_default();
        let cc_size = std::fs::metadata(&ccap_mxf_path)
            .map(|m| m.len())
            .unwrap_or(0);
        pkl_entries.push(crate::pkl::PklEntry {
            id: ccap_uuid.to_string(),
            asset_type: "application/mxf".into(),
            file: ccap_mxf_path.clone(),
            hash: cc_hash,
            size: cc_size,
        });
    }
    if aux_data.is_some() {
        let aux_hash = crate::hash::hash_file(&atmos_mxf_path).unwrap_or_default();
        let aux_size = std::fs::metadata(&atmos_mxf_path)
            .map(|m| m.len())
            .unwrap_or(0);
        pkl_entries.push(crate::pkl::PklEntry {
            id: atmos_uuid.to_string(),
            asset_type: "application/mxf".into(),
            file: atmos_mxf_path.clone(),
            hash: aux_hash,
            size: aux_size,
        });
    }

    if crate::pkl::generate_pkl(
        &pkl_entries,
        &pkl_uuid,
        config.standard,
        Some(&config.title),
        &pkl_path,
    ) != 0
    {
        tracing::error!("Failed to generate PKL");
        return -1;
    }
    // Nothing hashes the PKL, so this can follow the write.
    if !crate::package_signature::sign_if_configured(config.signer.as_ref(), &pkl_path, "PKL") {
        return -1;
    }

    // ── Generate ASSETMAP ─────────────────────────────────────────────
    let mut am_entries = vec![
        crate::assetmap::AssetMapEntry {
            id: pkl_uuid,
            path: pkl_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            packing_list: true,
        },
        crate::assetmap::AssetMapEntry {
            id: cpl_uuid.clone(),
            path: cpl_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            packing_list: false,
        },
    ];
    am_entries.push(crate::assetmap::AssetMapEntry {
        id: picture_uuid.to_string(),
        path: picture_mxf_name,
        packing_list: false,
    });
    if has_sound {
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: sound_uuid.to_string(),
            path: sound_mxf_name,
            packing_list: false,
        });
    }
    if has_subtitle {
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: subtitle_uuid.to_string(),
            path: subtitle_mxf_name,
            packing_list: false,
        });
    }
    if has_ccap {
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: ccap_uuid.to_string(),
            path: ccap_mxf_name,
            packing_list: false,
        });
    }
    if aux_data.is_some() {
        am_entries.push(crate::assetmap::AssetMapEntry {
            id: atmos_uuid.to_string(),
            path: atmos_mxf_name,
            packing_list: false,
        });
    }

    if crate::assetmap::generate_assetmap(&am_entries, &config.output_dir, config.standard, None)
        != 0
    {
        tracing::error!("Failed to generate ASSETMAP");
        return -1;
    }

    // Persist the content keys for the KDM step. The essence was already
    // encrypted at wrap time, so this only writes the keys file. The path is
    // caller-supplied (key_out); keys are never written next to the DCP.
    if config.encrypt {
        // Guaranteed Some by the early guard, but stay defensive: never fall
        // back to a default location for secret key material.
        let Some(keys_path) = config.key_out.as_ref() else {
            tracing::error!("--key-out is required when encrypting");
            return -1;
        };
        if let Some(parent) = keys_path.parent().filter(|p| !p.as_os_str().is_empty())
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::error!("Failed to create key-out directory: {e}");
            return -1;
        }
        let mut bundle = crate::encrypt::KeyBundle {
            cpl_id: cpl_uuid.clone(),
            keys: Vec::new(),
        };
        for key in [picture_key, sound_key, subtitle_key, ccap_key, atmos_key]
            .into_iter()
            .flatten()
        {
            bundle.keys.push(key.info);
        }
        if let Err(e) = bundle.write(keys_path) {
            tracing::error!("Failed to write keys file: {e}");
            return -1;
        }
        tracing::warn!(
            "Wrote content keys to {} — this file holds the plaintext AES keys. \
             Keep it secret, do not ship it in the DCP, and use it only to build KDMs.",
            keys_path.display()
        );
    }

    tracing::info!("DCP created: {}", config.output_dir.display());
    if let Some(path) = prepared_audio {
        let _ = std::fs::remove_file(path);
    }
    progress.stage(100, "done");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CancelledSink;
    impl ProgressSink for CancelledSink {
        fn stage(&self, _percent: u32, _message: &str) {}
        fn cancelled(&self) -> bool {
            true
        }
    }

    #[test]
    fn create_bails_when_cancelled() {
        // an already-cancelled sink must stop before touching inputs
        let config = DcpConfig::default();
        assert_eq!(create_dcp_with_progress(&config, &CancelledSink), -2);
    }

    #[test]
    fn container_dims_validation() {
        // 2K bounds
        assert!(validate_container_dims(2048, 1080, false).is_ok());
        assert!(validate_container_dims(1920, 1080, false).is_ok());
        assert!(validate_container_dims(2050, 1080, false).is_err()); // too wide for 2K
        // 4K bounds
        assert!(validate_container_dims(4096, 2160, true).is_ok());
        assert!(validate_container_dims(4096, 2160, false).is_err()); // 4K dims on 2K
        // odd and zero rejected
        assert!(validate_container_dims(1921, 1080, false).is_err());
        assert!(validate_container_dims(1920, 1081, false).is_err());
        assert!(validate_container_dims(0, 1080, false).is_err());
    }

    #[test]
    fn test_create_dcp_requires_picture_input() {
        let dir = tempfile::tempdir().unwrap();
        let config = DcpConfig {
            title: "Test Film".into(),
            output_dir: dir.path().to_path_buf(),
            frame_rate_num: 24,
            frame_rate_den: 1,
            ..Default::default()
        };
        assert_eq!(create_dcp(&config), -1);
        assert!(!dir.path().join("ASSETMAP.xml").exists());
    }

    #[test]
    fn create_dcp_encrypt_requires_key_out() {
        let dir = tempfile::tempdir().unwrap();
        let j2k_dir = dir.path().join("j2k");
        std::fs::create_dir_all(&j2k_dir).unwrap();
        let config = DcpConfig {
            title: "Test Film".into(),
            output_dir: dir.path().join("out"),
            j2k_dir: Some(j2k_dir),
            encrypt: true,
            key_out: None,
            frame_rate_num: 24,
            frame_rate_den: 1,
            ..Default::default()
        };
        // must fail before wrapping, and must not write keys anywhere
        assert_eq!(create_dcp(&config), -1);
        assert!(!dir.path().join("out/KEYS.json").exists());
    }
}
