use std::path::{Path, PathBuf};

// every scratch directory a create run writes inside the output directory
const INTERMEDIATE_DIRECTORIES: [&str; 7] = [
    "j2k",
    "j2k_trimmed",
    "j2k_right",
    "j2k_right_trimmed",
    postkit::still::HELD_PICTURE_DIR,
    "right",
    "audio_work",
];

// every scratch file a create run writes inside the output directory
const INTERMEDIATE_FILES: [&str; 8] = [
    "frames.ffconcat",
    "j2k_trimmed.wav",
    "audio_demux.wav",
    "audio_pullup.wav",
    "range_corrected.mkv",
    "hdr_to_dci_source.mov",
    "hdr_tonemap.mov",
    "slvs_sound.wav",
];

pub fn remove_intermediates(output_dir: &Path, keep: &[&Path]) {
    let kept: Vec<PathBuf> = keep.iter().map(|path| resolve(path)).collect();
    for name in INTERMEDIATE_DIRECTORIES {
        let path = output_dir.join(name);
        if kept.contains(&resolve(&path)) {
            continue;
        }
        let _ = std::fs::remove_dir_all(path);
    }
    for name in INTERMEDIATE_FILES {
        let path = output_dir.join(name);
        if kept.contains(&resolve(&path)) {
            continue;
        }
        let _ = std::fs::remove_file(path);
    }
    crate::encode_qol::EncodeState::clear(output_dir);
}

fn resolve(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scratch_goes_and_the_package_stays() {
        let dir = tempfile::TempDir::new().unwrap();
        let output = dir.path();
        for name in INTERMEDIATE_DIRECTORIES {
            std::fs::create_dir_all(output.join(name)).unwrap();
            std::fs::write(output.join(name).join("frame_00000000.j2c"), [0u8]).unwrap();
        }
        for name in INTERMEDIATE_FILES {
            std::fs::write(output.join(name), [0u8]).unwrap();
        }
        crate::encode_qol::EncodeState {
            source: "clip.mov".into(),
            total_frames: 2,
            fps: 24,
            width: 1998,
            height: 1080,
            bitrate_mbps: 125,
        }
        .save(output)
        .unwrap();
        let package = ["ASSETMAP.xml", "CPL_1.xml", "PKL_1.xml", "VIDEO_1.mxf"];
        for name in package {
            std::fs::write(output.join(name), [0u8]).unwrap();
        }

        remove_intermediates(output, &[]);

        for name in INTERMEDIATE_DIRECTORIES {
            assert!(!output.join(name).exists(), "{name} survived the cleanup");
        }
        for name in INTERMEDIATE_FILES {
            assert!(!output.join(name).exists(), "{name} survived the cleanup");
        }
        assert_eq!(crate::encode_qol::EncodeState::load(output), None);
        for name in package {
            assert!(output.join(name).exists(), "{name} was not part of the DCP");
        }
    }

    #[test]
    fn a_codestream_directory_handed_in_is_never_deleted() {
        let dir = tempfile::TempDir::new().unwrap();
        let output = dir.path();
        let source = output.join("j2k");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("frame_00000000.j2c"), [0u8]).unwrap();
        std::fs::create_dir_all(output.join("j2k_trimmed")).unwrap();

        remove_intermediates(output, &[&source]);

        assert!(source.join("frame_00000000.j2c").exists());
        assert!(!output.join("j2k_trimmed").exists());
    }
}
