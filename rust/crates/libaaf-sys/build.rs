use std::path::{Path, PathBuf};

/// libaaf lives in the repository's extern directory, not under this crate.
const LIBAAF_SOURCE: &str = "../../../extern/libaaf";

/// libaaf's CMake defines install() rules only on Linux, and names the static
/// archive differently per platform (libaaf.a, libaaf.obj under MSVC, bare
/// "aaf" on macOS), so build the target directly and copy the archive out of
/// the build tree under the one name rustc links.
fn find_static_archive(lib_dir: &Path) -> Option<PathBuf> {
    let mut directories = vec![lib_dir.to_path_buf()];
    for configuration in ["Debug", "Release", "RelWithDebInfo", "MinSizeRel"] {
        directories.push(lib_dir.join(configuration));
    }
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_file() && name.contains("aaf") && !name.ends_with(".pdb") {
                return Some(path);
            }
        }
    }
    None
}

fn main() {
    let source = PathBuf::from(LIBAAF_SOURCE);
    assert!(
        source.join("CMakeLists.txt").exists(),
        "{} is empty: run `git submodule update --init extern/libaaf`",
        source.display()
    );

    let out = cmake::Config::new(&source)
        .define("BUILD_STATIC_LIB", "ON")
        .define("BUILD_SHARED_LIB", "OFF")
        .define("BUILD_TOOLS", "OFF")
        .define("BUILD_DOC", "OFF")
        .define("BUILD_UNIT_TEST", "OFF")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .build_target("aaf-static")
        .build();

    let build_dir = out.join("build");
    let archive = find_static_archive(&build_dir.join("lib"))
        .unwrap_or_else(|| panic!("no libaaf static archive under {}", build_dir.display()));

    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let canonical = if target_env == "msvc" { "aaf.lib" } else { "libaaf.a" };
    let linked = out.join(canonical);
    std::fs::copy(&archive, &linked).unwrap_or_else(|e| {
        panic!("copying {} to {}: {e}", archive.display(), linked.display())
    });
    println!("cargo:rustc-link-search=native={}", out.display());

    cc::Build::new()
        .file("shim/aaf_shim.c")
        .include("shim")
        .include(source.join("include"))
        // the generated libaaf/version.h
        .include(build_dir.join("include"))
        .compile("aaf_shim");

    println!("cargo:rustc-link-lib=static=aaf");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        println!("cargo:rustc-link-lib=dylib=m");
    }

    println!("cargo:rerun-if-changed=shim/aaf_shim.c");
    println!("cargo:rerun-if-changed=shim/aaf_shim.h");
}
