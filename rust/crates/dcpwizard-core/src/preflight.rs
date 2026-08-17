//! Every refusal a `create` job can make before the encode starts.
//!
//! The rule: a refusal that fires once the encode has run must also fire from
//! here, so nothing spends a whole encode to find out it cannot be packaged.
//! Each check still lives in the module that owns it, and this runs them in one
//! order over one description of the job.

use std::path::{Path, PathBuf};

use postkit::colour::ColourSpace;
use postkit::subtitle_raster::BurnStyleOverrides;

use crate::source_picture::{EncodeGeometry, SourcePictureOptions};
use crate::{ContentType, Standard};

/// The shape of picture a job was pointed at: a video file, a single image held
/// for a length, or a directory of codestreams that is already compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PictureKind {
    #[default]
    Video,
    Still,
    Codestreams,
}

/// What a `create` job settles before the encode, as both front ends describe it.
#[derive(Debug, Clone)]
pub struct CreatePlan {
    pub picture: PathBuf,
    pub picture_kind: PictureKind,
    /// What ffprobe read from a video or still picture. The probe counts frames
    /// by decoding, so the front end that already ran one passes it in rather
    /// than paying for a second.
    pub source: Option<postkit::probe::VideoInfo>,
    /// Frames a still is held for; zero when the picture is not a still.
    pub still_frames: u64,
    pub fps: u32,
    pub picture_options: SourcePictureOptions,
    pub geometry: EncodeGeometry,
    pub trim_start_frames: u64,
    pub trim_end_frames: u64,
    pub pad_head_frames: u64,
    pub pad_tail_frames: u64,
    /// The sound file, or the directory of channel WAVs the build routes into
    /// one. None packages whatever the picture carries.
    pub audio: Option<PathBuf>,
    pub audio_map: Option<String>,
    /// Whether the stereo-to-5.1 upmix is in use.
    pub upmix: bool,
    /// How many channels the packaged sound track is filled to. None leaves the
    /// wrap's own rule, which widens 5.1 to 16 and touches nothing else.
    pub audio_channels: Option<u32>,
    pub audio_language: Option<String>,
    pub subtitle: Option<PathBuf>,
    pub ccap: Option<PathBuf>,
    pub burn_subtitle: Option<PathBuf>,
    pub burn_subtitle_font: Option<PathBuf>,
    pub burn_style: BurnStyleOverrides,
    pub source_colourspace: ColourSpace,
    /// Whether the encoder is handed X'Y'Z' frames already, by the source colour
    /// space, the HDR-to-DCI LUT or a PQ source.
    pub frames_already_xyz: bool,
    pub atmos: Option<PathBuf>,
    /// `LABEL=timecode` marker requests, unparsed.
    pub markers: Vec<String>,
    pub standard: Standard,
    pub content_type: ContentType,
    pub encrypt: bool,
    pub hdr_dci: bool,
    pub video_bit_rate_mbps: u32,
    pub right_eye: Option<PathBuf>,
    pub four_k: bool,
    pub reel_length_minutes: u32,
    pub reel_split_frames: Vec<u64>,
    /// How many library items are joined onto the build as extra reels.
    pub library_items: usize,
}

impl Default for CreatePlan {
    fn default() -> Self {
        CreatePlan {
            picture: PathBuf::new(),
            picture_kind: PictureKind::default(),
            source: None,
            still_frames: 0,
            fps: 24,
            picture_options: SourcePictureOptions::default(),
            geometry: EncodeGeometry::default(),
            trim_start_frames: 0,
            trim_end_frames: 0,
            pad_head_frames: 0,
            pad_tail_frames: 0,
            audio: None,
            audio_map: None,
            upmix: false,
            audio_channels: None,
            audio_language: None,
            subtitle: None,
            ccap: None,
            burn_subtitle: None,
            burn_subtitle_font: None,
            burn_style: BurnStyleOverrides::default(),
            source_colourspace: ColourSpace::Rec709,
            frames_already_xyz: false,
            atmos: None,
            markers: Vec::new(),
            standard: Standard::default(),
            content_type: ContentType::default(),
            encrypt: false,
            hdr_dci: false,
            video_bit_rate_mbps: 0,
            right_eye: None,
            four_k: false,
            reel_length_minutes: 0,
            reel_split_frames: Vec::new(),
            library_items: 0,
        }
    }
}

impl CreatePlan {
    /// The one WAV the job packages, when there is one to read now. A directory
    /// of channel WAVs is routed into a single file during the build, and sound
    /// demuxed from the picture does not exist yet, so neither is readable here.
    pub fn packaged_wav(&self) -> Option<&Path> {
        self.audio.as_deref().filter(|path| path.is_file())
    }

    fn is_codestreams(&self) -> bool {
        self.picture_kind == PictureKind::Codestreams
    }
}

/// What the encode will hand the packager: the raster it produces, and the size
/// the content itself lands at inside that raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedPicture {
    pub raster: (u32, u32),
    pub content: (u32, u32),
}

/// Run every plan-time refusal, cheapest and most specific first so a job with
/// two faults names the one a reader can act on.
pub fn check_before_encode(plan: &CreatePlan) -> Result<(), String> {
    if plan.is_codestreams() {
        crate::encode::check_precompressed_colourspace(plan.source_colourspace)?;
        crate::source_picture::check_precompressed_picture(&plan.picture_options)?;
    }
    crate::hfr::validate_fps_resolution(plan.fps, plan.four_k, plan.standard == Standard::Smpte)?;
    check_burn(plan)?;
    check_reel_splitting(plan)?;
    check_library_items(plan)?;
    check_audio_map(plan)?;
    check_audio_channels(plan)?;
    check_active_area(plan)?;
    check_audio_frame_alignment(plan)?;
    check_atmos(plan)?;
    check_timed_text(plan)
}

fn check_burn(plan: &CreatePlan) -> Result<(), String> {
    let Some(burn) = &plan.burn_subtitle else {
        return Ok(());
    };
    crate::subtitle::check_burn_supported(
        burn,
        plan.subtitle.as_deref(),
        plan.frames_already_xyz,
        plan.is_codestreams(),
    )?;
    crate::subtitle::prepare_subtitle_burn(
        burn,
        plan.burn_subtitle_font.as_deref(),
        postkit::encode::FrameRate::whole(plan.fps),
        &plan.burn_style,
    )
    .map(|_| ())
}

fn check_reel_splitting(plan: &CreatePlan) -> Result<(), String> {
    check_reel_split_support(&ReelSplitContent {
        splitting: plan.reel_length_minutes > 0 || !plan.reel_split_frames.is_empty(),
        stereo_3d: plan.right_eye.is_some(),
        atmos: plan.atmos.is_some(),
        hdr_dci: plan.hdr_dci,
        markers: !plan.markers.is_empty(),
    })
}

fn check_library_items(plan: &CreatePlan) -> Result<(), String> {
    check_library_item_support(&LibraryItemContent {
        attached: plan.library_items > 0,
        stereo_3d: plan.right_eye.is_some(),
        atmos: plan.atmos.is_some(),
        hdr_dci: plan.hdr_dci,
        markers: !plan.markers.is_empty(),
    })
}

/// What a build with library items joined onto it would have to carry, for the
/// tracks an item reel has no place for.
pub struct LibraryItemContent {
    pub attached: bool,
    pub stereo_3d: bool,
    pub atmos: bool,
    pub hdr_dci: bool,
    pub markers: bool,
}

/// An item reel is ordinary 2D SDR picture and sound, so a composition carrying
/// any of these would need the item conformed to something this cannot make.
pub fn check_library_item_support(content: &LibraryItemContent) -> Result<(), String> {
    if !content.attached {
        return Ok(());
    }
    if content.stereo_3d {
        return Err(
            "library items are not supported with stereoscopic 3D: an item has one eye".to_string(),
        );
    }
    if content.atmos {
        return Err(
            "library items are not supported with Atmos: the auxiliary track covers the \
             feature's reels and an item reel carries none"
                .to_string(),
        );
    }
    if content.hdr_dci {
        return Err(
            "library items are not supported with --hdr-dci: an item would have to be graded \
             to ST 2084 PQ to sit in the same composition"
                .to_string(),
        );
    }
    if content.markers {
        return Err(
            "--marker is not supported with library items: a marker offset is relative to its \
             own reel, and the items move which reel the feature is. A composition with items \
             gets the default FFOC/LFOC pair"
                .to_string(),
        );
    }
    Ok(())
}

/// What a split composition would have to carry, for the tracks the multi-reel
/// packer has no place for.
pub struct ReelSplitContent {
    pub splitting: bool,
    pub stereo_3d: bool,
    pub atmos: bool,
    pub hdr_dci: bool,
    pub markers: bool,
}

/// Reel splitting takes the single-reel package apart, and these three ride on
/// the whole composition, so the packer would have to drop them.
pub fn check_reel_split_support(content: &ReelSplitContent) -> Result<(), String> {
    if !content.splitting {
        return Ok(());
    }
    if content.stereo_3d || content.atmos {
        return Err("stereoscopic 3D and Atmos are not supported with reel splitting".to_string());
    }
    if content.hdr_dci {
        return Err("--hdr-dci is not supported with reel splitting".to_string());
    }
    if content.markers {
        return Err(
            "--marker is not supported with reel splitting: a marker offset is relative to \
             its own reel. A split composition gets the default FFOC/LFOC pair"
                .to_string(),
        );
    }
    Ok(())
}

fn check_audio_map(plan: &CreatePlan) -> Result<(), String> {
    let (Some(spec), Some(wav)) = (&plan.audio_map, plan.packaged_wav()) else {
        return Ok(());
    };
    crate::audio_map::parse_audio_map(spec, crate::audio_map::probe_channel_count(wav)?).map(|_| ())
}

/// How many channels the content fills once the map or the upmix has placed
/// them, before any silent fill. None when there is no WAV to read yet.
pub(crate) fn content_channels(plan: &CreatePlan) -> Option<u32> {
    let wav = plan.packaged_wav()?;
    let source_channels = u32::from(read_wav_spec(wav).ok()?.channels);
    match (&plan.audio_map, plan.upmix) {
        (Some(spec), _) => Some(
            crate::audio_map::parse_audio_map(spec, source_channels as usize)
                .ok()?
                .output_channels() as u32,
        ),
        (None, true) => Some(crate::mxf_wrap::CANONICAL_51_CHANNELS),
        (None, false) => Some(source_channels),
    }
}

/// Filling adds silent channels, so a source wider than the count asked for has
/// nowhere to go. The wrap refuses it too, and this says so before the encode.
fn check_audio_channels(plan: &CreatePlan) -> Result<(), String> {
    let Some(packaged) = plan.audio_channels else {
        return Ok(());
    };
    crate::mxf_wrap::check_packaged_channel_count(packaged)?;
    let Some(content) = content_channels(plan) else {
        return Ok(());
    };
    crate::mxf_wrap::check_source_fits_packaged_channels(content, packaged)
}

/// The active area a container declares has to fit inside the frames the encoder
/// produces, and the plan says what those frames will be.
fn check_active_area(plan: &CreatePlan) -> Result<(), String> {
    let Some((active_width, active_height)) = plan.geometry.container else {
        return Ok(());
    };
    let Some(planned) = plan_picture(plan)? else {
        return Ok(());
    };
    crate::cpl::check_active_area_fits(
        planned.raster.0,
        planned.raster.1,
        active_width,
        active_height,
    )
}

fn check_audio_frame_alignment(plan: &CreatePlan) -> Result<(), String> {
    let Some(wav) = plan.packaged_wav() else {
        return Ok(());
    };
    crate::pad::check_frame_aligned_sample_rate(read_wav_spec(wav)?.sample_rate, plan.fps)
}

fn check_atmos(plan: &CreatePlan) -> Result<(), String> {
    let Some(atmos) = &plan.atmos else {
        return Ok(());
    };
    check_atmos_path(atmos)?;
    let Some(picture_frames) = planned_picture_frames(plan) else {
        return Ok(());
    };
    check_atmos_frame_count(
        crate::mxf_wrap::collect_inputs(atmos)?.len() as u64,
        picture_frames,
    )
}

/// Parse every timed-text file with the reader the wrap uses, so a file the
/// packager cannot read is refused in a second rather than after the encode.
fn check_timed_text(plan: &CreatePlan) -> Result<(), String> {
    for path in [plan.subtitle.as_deref(), plan.ccap.as_deref()]
        .into_iter()
        .flatten()
        .filter(|path| path.exists())
    {
        crate::subtitle::check_timed_text_readable(path, plan.fps)?;
    }
    Ok(())
}

/// The Atmos essence has to be there before the picture is encoded around it.
pub fn check_atmos_path(atmos: &Path) -> Result<(), String> {
    if atmos.exists() {
        return Ok(());
    }
    Err(format!("Atmos input not found: {}", atmos.display()))
}

/// One Atmos input file is one frame, so an auxiliary track that does not cover
/// the picture would produce a broken DCP.
pub fn check_atmos_frame_count(atmos_frames: u64, picture_frames: u64) -> Result<(), String> {
    if atmos_frames == picture_frames {
        return Ok(());
    }
    Err(format!(
        "Atmos track is {atmos_frames} frames but the picture is {picture_frames}; \
         pass a directory with one Atmos frame file per picture frame"
    ))
}

/// What the encode will produce for this job, or None when nothing here can
/// measure the source.
pub fn plan_picture(plan: &CreatePlan) -> Result<Option<PlannedPicture>, String> {
    if plan.is_codestreams() {
        let Some(frame) = crate::reel::collect_frames(&plan.picture)
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let raster = crate::pad::read_j2k_dimensions(&frame)?;
        return Ok(Some(PlannedPicture {
            raster,
            content: raster,
        }));
    }
    let Some(info) = plan.source.as_ref() else {
        return Ok(None);
    };
    let resolved = crate::source_picture::resolve_picture(
        &plan.picture_options,
        &plan.picture,
        info.width,
        info.height,
        &plan.geometry,
        postkit::encode::detect_input_type(&plan.picture)
            == postkit::encode::InputType::ImageSequence,
    )?;
    Ok(Some(PlannedPicture {
        raster: (resolved.encode_width, resolved.encode_height),
        content: (resolved.plan.scaled_width, resolved.plan.scaled_height),
    }))
}

/// How many frames the packaged picture will carry, or None when nothing here
/// can count them.
pub fn planned_picture_frames(plan: &CreatePlan) -> Option<u64> {
    let source_frames = match plan.picture_kind {
        PictureKind::Still => plan.still_frames,
        PictureKind::Codestreams => crate::trim::frame_count(&plan.picture),
        PictureKind::Video => u64::from(plan.source.as_ref()?.total_frames),
    };
    if source_frames == 0 {
        return None;
    }
    Some(
        source_frames
            .saturating_sub(plan.trim_start_frames)
            .saturating_sub(plan.trim_end_frames)
            + plan.pad_head_frames
            + plan.pad_tail_frames,
    )
}

pub(crate) fn read_wav_spec(path: &Path) -> Result<hound::WavSpec, String> {
    hound::WavReader::open(path)
        .map(|reader| reader.spec())
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_with_picture(picture: PathBuf) -> CreatePlan {
        CreatePlan {
            picture,
            picture_kind: PictureKind::Codestreams,
            ..Default::default()
        }
    }

    /// A codestream carrying nothing but the SIZ marker the raster is read from.
    fn write_codestream(path: &Path, width: u32, height: u32) {
        let mut siz: Vec<u8> = Vec::new();
        siz.extend_from_slice(&3u16.to_be_bytes());
        for value in [width, height, 0, 0, width, height, 0, 0] {
            siz.extend_from_slice(&value.to_be_bytes());
        }
        siz.extend_from_slice(&3u16.to_be_bytes());
        siz.extend_from_slice(&[11, 1, 1, 11, 1, 1, 11, 1, 1]);

        let mut codestream: Vec<u8> = vec![0xff, 0x4f, 0xff, 0x51];
        codestream.extend_from_slice(&((siz.len() + 2) as u16).to_be_bytes());
        codestream.extend_from_slice(&siz);
        codestream.extend_from_slice(&[0xff, 0xd9]);
        std::fs::write(path, codestream).unwrap();
    }

    fn write_codestreams(dir: &Path, count: u32, width: u32, height: u32) {
        std::fs::create_dir_all(dir).unwrap();
        for frame in 0..count {
            write_codestream(&dir.join(format!("frame_{frame:08}.j2c")), width, height);
        }
    }

    fn write_wav(path: &Path, channels: u16, sample_rate: u32) {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..(sample_rate as usize * channels as usize) {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    /// A container the encoded frames cannot hold has to be refused before the
    /// encode, not by the CPL writer after it.
    #[test]
    fn a_container_larger_than_the_encode_raster_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let codestreams = dir.path().join("j2k");
        write_codestreams(&codestreams, 1, 1920, 1080);

        let mut plan = plan_with_picture(codestreams);
        plan.geometry.container = Some((2048, 858));
        let error = check_before_encode(&plan).unwrap_err();
        assert!(
            error.contains("2048x858 is larger than the 1920x1080"),
            "{error}"
        );

        plan.geometry.container = Some((1920, 1080));
        assert_eq!(check_before_encode(&plan), Ok(()));
    }

    /// Filling only adds silence, so a source the packaged count cannot hold is
    /// refused at plan time, not an encode later.
    #[test]
    fn a_source_wider_than_the_packaged_channel_count_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let codestreams = dir.path().join("j2k");
        write_codestreams(&codestreams, 1, 2048, 1080);
        let wav = dir.path().join("sound.wav");
        write_wav(&wav, 8, 48_000);

        let mut plan = plan_with_picture(codestreams);
        plan.audio = Some(wav);
        plan.audio_channels = Some(6);
        let error = check_before_encode(&plan).unwrap_err();
        assert!(error.contains('8') && error.contains('6'), "{error}");
        assert!(error.contains("--audio-map"), "{error}");

        plan.audio_channels = Some(16);
        assert_eq!(check_before_encode(&plan), Ok(()));
    }

    /// The upmix widens the content to 5.1 before the fill, so the count the
    /// check measures is what the upmix leaves, not what the file carries.
    #[test]
    fn the_upmix_widens_the_content_the_packaged_count_is_measured_against() {
        let dir = tempfile::tempdir().unwrap();
        let codestreams = dir.path().join("j2k");
        write_codestreams(&codestreams, 1, 2048, 1080);
        let wav = dir.path().join("sound.wav");
        write_wav(&wav, 2, 48_000);

        let mut plan = plan_with_picture(codestreams);
        plan.audio = Some(wav);
        plan.upmix = true;
        assert_eq!(content_channels(&plan), Some(6));
        plan.audio_channels = Some(2);
        assert!(check_before_encode(&plan).is_err());
    }

    #[test]
    fn an_audio_sample_rate_that_is_not_whole_frames_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("sound.wav");
        write_wav(&wav, 2, 44_100);

        let mut plan = plan_with_picture(dir.path().join("j2k"));
        plan.audio = Some(wav.clone());
        let error = check_before_encode(&plan).unwrap_err();
        assert!(error.contains("44100 Hz"), "{error}");

        write_wav(&wav, 2, 48_000);
        assert_eq!(check_before_encode(&plan), Ok(()));
    }

    #[test]
    fn an_atmos_path_that_is_not_there_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = plan_with_picture(dir.path().join("j2k"));
        plan.atmos = Some(dir.path().join("atmos"));

        let error = check_before_encode(&plan).unwrap_err();
        assert!(error.contains("Atmos input not found"), "{error}");
    }

    #[test]
    fn an_atmos_track_shorter_than_the_picture_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let codestreams = dir.path().join("j2k");
        write_codestreams(&codestreams, 3, 1920, 1080);
        let atmos = dir.path().join("atmos");
        std::fs::create_dir_all(&atmos).unwrap();
        for frame in 0..2 {
            std::fs::write(atmos.join(format!("atmos_{frame}.bin")), b"frame").unwrap();
        }

        let mut plan = plan_with_picture(codestreams);
        plan.atmos = Some(atmos.clone());
        let error = check_before_encode(&plan).unwrap_err();
        assert!(error.contains("2 frames but the picture is 3"), "{error}");

        std::fs::write(atmos.join("atmos_2.bin"), b"frame").unwrap();
        assert_eq!(check_before_encode(&plan), Ok(()));
    }

    #[test]
    fn a_subtitle_the_packager_cannot_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let subtitle = dir.path().join("subs.srt");
        std::fs::write(&subtitle, "this is not a cue list").unwrap();

        let mut plan = plan_with_picture(dir.path().join("j2k"));
        plan.subtitle = Some(subtitle.clone());
        assert!(check_before_encode(&plan).is_err());

        std::fs::write(&subtitle, "1\n00:00:05,000 --> 00:00:07,000\nhello\n").unwrap();
        assert_eq!(check_before_encode(&plan), Ok(()));
    }

    #[test]
    fn a_caption_file_in_a_format_with_no_reader_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let ccap = dir.path().join("captions.txt");
        std::fs::write(&ccap, "hello").unwrap();

        let mut plan = plan_with_picture(dir.path().join("j2k"));
        plan.ccap = Some(ccap);
        let error = check_before_encode(&plan).unwrap_err();
        assert!(error.contains("unsupported subtitle format"), "{error}");
    }

    #[test]
    fn an_audio_map_that_does_not_parse_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("sound.wav");
        write_wav(&wav, 2, 48_000);

        let mut plan = plan_with_picture(dir.path().join("j2k"));
        plan.audio = Some(wav);
        plan.audio_map = Some("1:L,2".to_string());
        assert!(check_before_encode(&plan).is_err());

        plan.audio_map = Some("1:L,2:R".to_string());
        assert_eq!(check_before_encode(&plan), Ok(()));
    }

    #[test]
    fn a_split_composition_carrying_a_track_the_packer_drops_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = plan_with_picture(dir.path().join("j2k"));
        plan.right_eye = Some(dir.path().join("right.mov"));

        assert_eq!(check_before_encode(&plan), Ok(()));

        plan.reel_length_minutes = 20;
        let error = check_before_encode(&plan).unwrap_err();
        assert!(
            error.contains("not supported with reel splitting"),
            "{error}"
        );
    }
}
