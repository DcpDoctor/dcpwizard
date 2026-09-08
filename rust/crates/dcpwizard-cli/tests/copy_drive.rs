use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};

const PICTURE_BYTES: usize = 3 << 20;

// larger than any filesystem the tests run on, and sparse, so no blocks are used
const SPARSE_PICTURE_BYTES: u64 = 4 << 40;

// a regressed space check would copy the sparse file's zeros until the temp
// filesystem is full, so cap what the child may write
#[cfg(unix)]
const CHILD_FILE_SIZE_LIMIT_BYTES: u64 = 64 << 20;

// the run must not pick up the developer's own preferences
fn dcpwizard(config_home: &Path) -> Command {
    let mut command = Command::cargo_bin("dcpwizard").unwrap();
    command.env("XDG_CONFIG_HOME", config_home);
    command
}

fn write_small_dcp(dir: &Path) {
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("ASSETMAP.xml"), b"<AssetMap/>").unwrap();
    std::fs::write(dir.join("VOLINDEX.xml"), b"<VolumeIndex/>").unwrap();
    std::fs::write(dir.join("sub/CPL.xml"), b"<CompositionPlaylist/>").unwrap();
    // varying bytes across more than one copy buffer
    let picture: Vec<u8> = (0..PICTURE_BYTES).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.join("sub/picture.mxf"), &picture).unwrap();
}

fn relative_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(root, root, &mut found);
    found.sort();
    found
}

fn collect(root: &Path, dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, found);
        } else {
            found.push(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
}

fn sha1_hex(path: &Path) -> String {
    postkit::hash::hash_file(path, postkit::hash::HashAlgorithm::Sha1)
        .unwrap()
        .hex
}

#[test]
fn copy_puts_every_file_on_the_drive_with_the_source_hash() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("MyFilm_FTR_F_EN-XX_OV");
    write_small_dcp(&source);
    let drive = temp.path().join("drive");

    dcpwizard(&temp.path().join("config"))
        .args([
            "copy",
            "--src",
            source.to_str().unwrap(),
            "--dst",
            drive.to_str().unwrap(),
        ])
        .assert()
        .success();

    let copied = drive.join("MyFilm_FTR_F_EN-XX_OV");
    let sources = relative_files(&source);
    assert_eq!(sources.len(), 4);
    assert_eq!(relative_files(&copied), sources);
    for relative in sources {
        let from = source.join(&relative);
        let to = copied.join(&relative);
        assert_eq!(
            std::fs::read(&from).unwrap(),
            std::fs::read(&to).unwrap(),
            "{} differs",
            relative.display()
        );
        assert_eq!(
            sha1_hex(&from),
            sha1_hex(&to),
            "{} hashes differ",
            relative.display()
        );
    }
}

#[cfg(unix)]
fn limited_dcpwizard(config_home: &Path) -> Command {
    use std::os::unix::process::CommandExt;

    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin("dcpwizard"));
    command.env("XDG_CONFIG_HOME", config_home);
    unsafe {
        command.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: CHILD_FILE_SIZE_LIMIT_BYTES,
                rlim_max: CHILD_FILE_SIZE_LIMIT_BYTES,
            };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        })
    };
    Command::from_std(command)
}

#[cfg(unix)]
#[test]
fn copy_refuses_a_dcp_larger_than_the_drive_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("MyFilm_FTR_F_EN-XX_OV");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("ASSETMAP.xml"), b"<AssetMap/>").unwrap();
    let sparse = std::fs::File::create(source.join("picture.mxf")).unwrap();
    sparse.set_len(SPARSE_PICTURE_BYTES).unwrap();
    drop(sparse);
    let drive = temp.path().join("drive");

    limited_dcpwizard(&temp.path().join("config"))
        .args([
            "copy",
            "--src",
            source.to_str().unwrap(),
            "--dst",
            drive.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("not enough space on destination")
                .and(predicate::str::contains("need 4.0 TiB but only"))
                .and(predicate::str::contains("free")),
        );

    let copied = drive.join("MyFilm_FTR_F_EN-XX_OV");
    assert_eq!(
        relative_files(&copied),
        Vec::<PathBuf>::new(),
        "the refusal came after something was written"
    );
}
