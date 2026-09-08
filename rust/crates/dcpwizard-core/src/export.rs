use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Export format for transcoding DCP content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExportFormat {
    ProRes,
    #[default]
    H264,
    H265,
    DnxHr,
    ImageSequence,
}

impl ExportFormat {
    fn ffmpeg_codec(&self) -> &'static str {
        match self {
            ExportFormat::ProRes => "prores_ks",
            ExportFormat::H264 => "libx264",
            ExportFormat::H265 => "libx265",
            ExportFormat::DnxHr => "dnxhd",
            ExportFormat::ImageSequence => "png",
        }
    }

    fn file_extension(&self) -> &'static str {
        match self {
            ExportFormat::ProRes => "mov",
            ExportFormat::H264 => "mp4",
            ExportFormat::H265 => "mp4",
            ExportFormat::DnxHr => "mxf",
            ExportFormat::ImageSequence => "png",
        }
    }

    fn pixel_format(&self) -> &'static str {
        match self {
            ExportFormat::ProRes => "yuv422p10le",
            ExportFormat::DnxHr => "yuv422p",
            ExportFormat::H264 | ExportFormat::H265 => "yuv420p",
            ExportFormat::ImageSequence => "rgb48le",
        }
    }

    // a ProRes or DNxHR master for approval carries PCM, only the delivery codecs take AAC
    fn audio_codec(&self) -> &'static str {
        match self {
            ExportFormat::ProRes | ExportFormat::DnxHr => "pcm_s24le",
            ExportFormat::H264 | ExportFormat::H265 | ExportFormat::ImageSequence => "aac",
        }
    }
}

/// Export configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportConfig {
    pub input_mxf: PathBuf,
    pub output_path: PathBuf,
    pub format: ExportFormat,
    pub quality_crf: u32,
    pub audio_mxf: Option<PathBuf>,
}

/// Export / transcode DCP MXF content to a delivery format via ffmpeg.
pub fn export_dcp(config: &ExportConfig) -> Result<(), String> {
    if !config.input_mxf.exists() {
        return Err(format!(
            "input MXF not found: {}",
            config.input_mxf.display()
        ));
    }

    let crf = if config.quality_crf == 0 {
        18
    } else {
        config.quality_crf
    };

    if config.format == ExportFormat::ImageSequence {
        return export_image_sequence(&config.input_mxf, &config.output_path);
    }

    let output = if config.output_path.extension().is_none() {
        config
            .output_path
            .with_extension(config.format.file_extension())
    } else {
        config.output_path.clone()
    };

    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.arg("-y").arg("-v").arg("error");
    cmd.arg("-i").arg(&config.input_mxf);

    if let Some(audio) = &config.audio_mxf
        && audio.exists()
    {
        cmd.arg("-i").arg(audio);
    }

    cmd.arg("-vf").arg(rec709_filter(config.format));
    cmd.arg("-c:v").arg(config.format.ffmpeg_codec());

    match config.format {
        ExportFormat::H264 | ExportFormat::H265 => {
            cmd.arg("-crf").arg(crf.to_string());
            cmd.arg("-preset").arg("medium");
        }
        ExportFormat::ProRes => {
            cmd.arg("-profile:v").arg("3"); // ProRes HQ
        }
        ExportFormat::DnxHr => {
            cmd.arg("-profile:v").arg("dnxhr_hq");
        }
        ExportFormat::ImageSequence => unreachable!(),
    }

    cmd.arg("-c:a")
        .arg(config.format.audio_codec())
        .arg(&output);

    run_ffmpeg(cmd, &format!("export to {}", output.display()))?;
    tracing::info!("Exported DCP to {}", output.display());
    Ok(())
}

// a DCP picture is X'Y'Z' at DCI gamma 2.6: swscale undoes that, out_color_matrix picks the
// Rec.709 matrix over swscale's 601 default, and setparams tags what the player has to assume
fn rec709_filter(format: ExportFormat) -> String {
    format!(
        "scale=out_color_matrix=bt709:out_range=tv,format={},\
         setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv",
        format.pixel_format()
    )
}

fn run_ffmpeg(mut cmd: std::process::Command, operation: &str) -> Result<(), String> {
    match cmd.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "ffmpeg could not {operation}: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!("could not run ffmpeg: {e}")),
    }
}

/// Extract a single frame from an MXF at the given frame number.
pub fn extract_frame(mxf_path: &Path, frame_number: u64, output_path: &Path) -> i32 {
    let fps = 24; // Default DCP frame rate
    let timestamp = format!(
        "{:02}:{:02}:{:02}.{:03}",
        frame_number / (fps * 3600),
        (frame_number / (fps * 60)) % 60,
        (frame_number / fps) % 60,
        ((frame_number % fps) * 1000) / fps
    );

    let result = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-ss")
        .arg(&timestamp)
        .arg("-i")
        .arg(mxf_path)
        .arg("-frames:v")
        .arg("1")
        .arg(output_path)
        .output();

    match result {
        Ok(o) if o.status.success() => {
            tracing::info!(
                "Extracted frame {} to {}",
                frame_number,
                output_path.display()
            );
            0
        }
        Ok(o) => {
            tracing::error!(
                "ffmpeg frame extraction failed: {}",
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

fn export_image_sequence(input_mxf: &Path, output_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("could not create {}: {e}", output_dir.display()))?;

    let pattern = output_dir.join("frame_%08d.png");

    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(input_mxf)
        .arg(&pattern);

    run_ffmpeg(cmd, &format!("write frames to {}", output_dir.display()))?;
    tracing::info!("Exported image sequence to {}", output_dir.display());
    Ok(())
}
