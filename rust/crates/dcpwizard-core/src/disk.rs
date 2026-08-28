//! Disk writer: format a delivery drive ext2/ext3 and check an existing drive's
//! filesystem, for cinema hard-drive delivery (dom#2095, dom#2112).
//!
//! Safety is non-negotiable: `format_drive` refuses any target that appears in
//! /proc/mounts (or whose partitions do), requires an explicit `--yes`, and only
//! writes to a block device unless `--image` opts into a regular file (for tests
//! and loopback). `check_drive` never modifies and works unprivileged on images.

use std::path::Path;

/// ext superblock lives at byte 1024; these are absolute offsets into the device.
const SB_MAGIC_OFFSET: u64 = 0x438;
const SB_FEATURE_COMPAT_OFFSET: u64 = 0x45c;
const SB_FEATURE_INCOMPAT_OFFSET: u64 = 0x460;
const SB_FEATURE_RO_COMPAT_OFFSET: u64 = 0x464;
const SB_LABEL_OFFSET: u64 = 0x478;
const EXT_MAGIC: u16 = 0xEF53;
const FEATURE_COMPAT_HAS_JOURNAL: u32 = 0x0004;
// features ext3 understands; anything beyond these on a journalled fs means ext4
const EXT3_INCOMPAT_SUPP: u32 = 0x0002 | 0x0004 | 0x0010; // filetype | recover | meta_bg
const EXT3_RO_COMPAT_SUPP: u32 = 0x0001 | 0x0002; // sparse_super | large_file

/// Filesystem the disk writer can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtFs {
    Ext2,
    Ext3,
}

impl ExtFs {
    fn mkfs_bin(self) -> &'static str {
        match self {
            ExtFs::Ext2 => "mkfs.ext2",
            ExtFs::Ext3 => "mkfs.ext3",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "ext2" => Ok(ExtFs::Ext2),
            "ext3" => Ok(ExtFs::Ext3),
            other => Err(format!(
                "unsupported filesystem '{other}'; use ext2 or ext3"
            )),
        }
    }
}

/// A drive's detected filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveInfo {
    pub fstype: Option<String>,
    pub label: Option<String>,
}

/// Return the mount source devices from `/proc/mounts` (first field per line).
fn mounted_devices() -> Vec<String> {
    std::fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

/// True if `target` or any of its partitions is currently mounted.
fn is_mounted(target: &Path) -> bool {
    let t = target.to_string_lossy();
    mounted_devices()
        .iter()
        .any(|dev| dev.as_str() == t || dev.starts_with(t.as_ref()))
}

/// Format `target` as `fs` with an optional volume `label`.
///
/// Refuses if the target (or a partition of it) is mounted, if `yes` is false, or,
/// without `image`, if the target is not a block device. Shells out to
/// mkfs.ext2 / mkfs.ext3.
pub fn format_drive(
    target: &Path,
    fs: ExtFs,
    label: Option<&str>,
    yes: bool,
    image: bool,
) -> Result<(), String> {
    // mount check first: never touch anything mounted, regardless of --yes
    if is_mounted(target) {
        return Err(format!(
            "{} (or one of its partitions) is mounted; refusing to format",
            target.display()
        ));
    }
    if !yes {
        return Err("refusing to format without --yes (this erases the target)".into());
    }

    let meta =
        std::fs::metadata(target).map_err(|e| format!("cannot stat {}: {e}", target.display()))?;
    if image {
        if !meta.is_file() {
            return Err(format!(
                "--image requires a regular file, but {} is not one",
                target.display()
            ));
        }
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            if !meta.file_type().is_block_device() {
                return Err(format!(
                    "{} is not a block device; use --image to format a regular file",
                    target.display()
                ));
            }
        }
        #[cfg(not(unix))]
        return Err(format!(
            "{} is not usable: block device formatting is unix-only; use --image to format a regular file",
            target.display()
        ));
    }

    if which(fs.mkfs_bin()).is_none() {
        return Err(format!("{} not found on PATH", fs.mkfs_bin()));
    }

    let mut cmd = std::process::Command::new(fs.mkfs_bin());
    // -F forces mke2fs to proceed on a regular file without prompting
    if image {
        cmd.arg("-F");
    }
    if let Some(l) = label {
        cmd.arg("-L").arg(l);
    }
    cmd.arg(target);

    let out = cmd
        .output()
        .map_err(|e| format!("failed to run {}: {e}", fs.mkfs_bin()))?;
    if !out.status.success() {
        return Err(format!(
            "{} failed: {}",
            fs.mkfs_bin(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Report `target`'s filesystem type and label without modifying it.
///
/// Uses `blkid -o export` when present, else reads the ext superblock directly so
/// it works unprivileged on image files.
pub fn check_drive(target: &Path) -> Result<DriveInfo, String> {
    if !target.exists() {
        return Err(format!("{} does not exist", target.display()));
    }
    if let Some(info) = blkid_info(target) {
        return Ok(info);
    }
    superblock_info(target)
}

fn blkid_info(target: &Path) -> Option<DriveInfo> {
    let out = std::process::Command::new("blkid")
        .arg("-o")
        .arg("export")
        .arg(target)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut fstype = None;
    let mut label = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("TYPE=") {
            fstype = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("LABEL=") {
            label = Some(v.trim().to_string());
        }
    }
    Some(DriveInfo { fstype, label })
}

/// Parse the ext2/3/4 superblock directly.
fn superblock_info(target: &Path) -> Result<DriveInfo, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(target)
        .map_err(|e| format!("cannot open {}: {e}", target.display()))?;

    let read_u16 = |f: &mut std::fs::File, off: u64| -> Result<u16, String> {
        let mut b = [0u8; 2];
        f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
        f.read_exact(&mut b).map_err(|e| e.to_string())?;
        Ok(u16::from_le_bytes(b))
    };
    let read_u32 = |f: &mut std::fs::File, off: u64| -> Result<u32, String> {
        let mut b = [0u8; 4];
        f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
        f.read_exact(&mut b).map_err(|e| e.to_string())?;
        Ok(u32::from_le_bytes(b))
    };

    if read_u16(&mut f, SB_MAGIC_OFFSET)? != EXT_MAGIC {
        // not an ext filesystem we recognise
        return Ok(DriveInfo {
            fstype: None,
            label: None,
        });
    }

    let compat = read_u32(&mut f, SB_FEATURE_COMPAT_OFFSET)?;
    let incompat = read_u32(&mut f, SB_FEATURE_INCOMPAT_OFFSET)?;
    let ro_compat = read_u32(&mut f, SB_FEATURE_RO_COMPAT_OFFSET)?;

    let fstype = if compat & FEATURE_COMPAT_HAS_JOURNAL == 0 {
        "ext2"
    } else if (incompat & !EXT3_INCOMPAT_SUPP) != 0 || (ro_compat & !EXT3_RO_COMPAT_SUPP) != 0 {
        "ext4"
    } else {
        "ext3"
    };

    let mut label_buf = [0u8; 16];
    f.seek(SeekFrom::Start(SB_LABEL_OFFSET))
        .map_err(|e| e.to_string())?;
    f.read_exact(&mut label_buf).map_err(|e| e.to_string())?;
    let end = label_buf.iter().position(|&b| b == 0).unwrap_or(16);
    let label = if end == 0 {
        None
    } else {
        Some(String::from_utf8_lossy(&label_buf[..end]).into_owned())
    };

    Ok(DriveInfo {
        fstype: Some(fstype.to_string()),
        label,
    })
}

/// Locate a binary on PATH.
fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fs() {
        assert_eq!(ExtFs::parse("ext2").unwrap(), ExtFs::Ext2);
        assert_eq!(ExtFs::parse("EXT3").unwrap(), ExtFs::Ext3);
        assert!(ExtFs::parse("ntfs").is_err());
    }

    #[test]
    fn refuses_without_yes() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("disk.img");
        std::fs::write(&img, vec![0u8; 4096]).unwrap();
        let err = format_drive(&img, ExtFs::Ext2, None, false, true).unwrap_err();
        assert!(err.contains("--yes"), "{err}");
    }

    #[test]
    fn refuses_regular_file_without_image() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("notadisk");
        std::fs::write(&f, b"x").unwrap();
        let err = format_drive(&f, ExtFs::Ext2, None, true, false).unwrap_err();
        assert!(err.contains("block device"), "{err}");
    }

    // /proc/mounts and mkfs.ext2 are linux
    #[cfg(target_os = "linux")]
    #[test]
    fn refuses_a_mounted_device() {
        // the device backing "/" is always in /proc/mounts; formatting it must
        // be refused before anything else.
        let dev = mounted_devices()
            .into_iter()
            .find(|d| d.starts_with("/dev/"))
            .expect("a /dev mount source in /proc/mounts");
        let err = format_drive(Path::new(&dev), ExtFs::Ext2, None, true, false).unwrap_err();
        assert!(err.contains("mounted"), "{err}");
    }

    #[test]
    fn check_reports_unknown_for_non_ext() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("random.bin");
        std::fs::write(&f, vec![0xAAu8; 8192]).unwrap();
        // blkid may or may not be present; superblock parse must say unknown
        let info = superblock_info(&f).unwrap();
        assert_eq!(info.fstype, None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn format_and_check_roundtrip_on_image() {
        which("mkfs.ext2").expect("mkfs.ext2 on PATH");
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("disk.img");
        // 8 MiB sparse image is enough for a tiny ext2 fs
        let f = std::fs::File::create(&img).unwrap();
        f.set_len(8 * 1024 * 1024).unwrap();
        drop(f);

        format_drive(&img, ExtFs::Ext2, Some("DCP_DELIVERY"), true, true).unwrap();

        // read superblock directly (unprivileged, no blkid needed)
        let info = superblock_info(&img).unwrap();
        assert_eq!(info.fstype.as_deref(), Some("ext2"));
        assert_eq!(info.label.as_deref(), Some("DCP_DELIVERY"));

        // ext3 gets a journal and must be detected as such
        format_drive(&img, ExtFs::Ext3, Some("DCP2"), true, true).unwrap();
        let info3 = superblock_info(&img).unwrap();
        assert_eq!(info3.fstype.as_deref(), Some("ext3"));
        assert_eq!(info3.label.as_deref(), Some("DCP2"));
    }
}
