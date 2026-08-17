use sha1::Digest;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Evict a file's pages from the page cache so a following read hits the device.
/// Without this the read-back below just returns the bytes we cached on write and
/// verifies nothing about what actually landed on the drive.
#[cfg(target_os = "linux")]
fn drop_page_cache(file: &File) {
    use std::os::unix::io::AsRawFd;
    unsafe {
        libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

// macos has no posix_fadvise; the read-back there may be served from cache
#[cfg(not(target_os = "linux"))]
fn drop_page_cache(_file: &File) {}

/// Copy a DCP to a target drive with SHA-1 hash verification.
pub fn copy_to_drive(dcp_dir: &Path, target_dir: &Path) -> i32 {
    if !dcp_dir.exists() {
        tracing::error!("Source DCP directory not found: {}", dcp_dir.display());
        return -1;
    }

    let dest = target_dir.join(
        dcp_dir
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("DCP")),
    );

    if let Err(e) = std::fs::create_dir_all(&dest) {
        tracing::error!("Failed to create target directory: {e}");
        return -1;
    }

    let files = collect_files(dcp_dir);
    let total = files.len();

    // Fail early if the DCP won't fit on the destination (DoM bug 3150).
    let required: u64 = files
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    if let Err(e) = postkit::free_space::check_destination_space(&dest, required) {
        tracing::error!("{e}");
        return -1;
    }

    tracing::info!(
        "Copying {total} files from {} to {}",
        dcp_dir.display(),
        dest.display()
    );

    for (i, src_path) in files.iter().enumerate() {
        let rel = src_path.strip_prefix(dcp_dir).unwrap_or(src_path);
        let dst_path = dest.join(rel);

        if let Some(parent) = dst_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::error!("Failed to create directory {}: {e}", parent.display());
            return -1;
        }

        // Read source and compute hash
        let src_data = match std::fs::read(src_path) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to read {}: {e}", src_path.display());
                return -1;
            }
        };

        let src_hash = sha1_hex(&src_data);

        // Write to destination, flush to the device, then evict from the page
        // cache so the read-back reads the drive rather than our own write cache.
        match File::create(&dst_path).and_then(|mut f| {
            f.write_all(&src_data)?;
            f.sync_all()?;
            drop_page_cache(&f);
            Ok(())
        }) {
            Ok(()) => {}
            Err(e) => {
                tracing::error!("Failed to write {}: {e}", dst_path.display());
                return -1;
            }
        }

        // Verify by reading back from the drive and comparing hash
        let dst_data = match std::fs::read(&dst_path) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to read back {}: {e}", dst_path.display());
                return -1;
            }
        };

        let dst_hash = sha1_hex(&dst_data);

        if src_hash != dst_hash {
            tracing::error!(
                "Hash mismatch for {}: src={} dst={}",
                rel.display(),
                src_hash,
                dst_hash
            );
            return -1;
        }

        tracing::info!(
            "[{}/{}] Verified: {} ({})",
            i + 1,
            total,
            rel.display(),
            src_hash
        );
    }

    tracing::info!("Successfully copied and verified {total} files");
    0
}

fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files_recursive(dir, &mut files);
    files.sort();
    files
}

fn collect_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn sha1_hex(data: &[u8]) -> String {
    let mut hasher = sha1::Sha1::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex_encode(&result)
}

fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_and_verifies_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("MyDCP");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("ASSETMAP.xml"), b"a").unwrap();
        std::fs::write(src.join("sub/picture.mxf"), vec![7u8; 4096]).unwrap();

        let target = dir.path().join("drive");
        assert_eq!(copy_to_drive(&src, &target), 0);

        let dst = target.join("MyDCP");
        assert_eq!(std::fs::read(dst.join("ASSETMAP.xml")).unwrap(), b"a");
        assert_eq!(
            std::fs::read(dst.join("sub/picture.mxf")).unwrap(),
            vec![7u8; 4096]
        );
    }
}
