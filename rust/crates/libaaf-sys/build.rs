use std::path::PathBuf;

/// libaaf lives in the repository's extern directory, not under this crate.
const LIBAAF_SOURCE: &str = "../../../extern/libaaf";

fn main() {
    let source = PathBuf::from(LIBAAF_SOURCE);
    assert!(
        source.join("CMakeLists.txt").exists(),
        "{} is empty: run `git submodule update --init extern/libaaf`",
        source.display()
    );

    let installed = cmake::Config::new(&source)
        .define("BUILD_STATIC_LIB", "ON")
        .define("BUILD_SHARED_LIB", "OFF")
        .define("BUILD_TOOLS", "OFF")
        .define("BUILD_DOC", "OFF")
        .define("BUILD_UNIT_TEST", "OFF")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .build();

    // GNUInstallDirs picks lib64 on some distributions
    for directory in ["lib", "lib64"] {
        println!(
            "cargo:rustc-link-search=native={}",
            installed.join(directory).display()
        );
    }
    if cfg!(target_os = "windows") {
        for configuration in ["Debug", "Release", "RelWithDebInfo", "MinSizeRel"] {
            println!(
                "cargo:rustc-link-search=native={}",
                installed.join("lib").join(configuration).display()
            );
        }
    }

    cc::Build::new()
        .file("shim/aaf_shim.c")
        .include("shim")
        .include(installed.join("include"))
        .compile("aaf_shim");

    println!("cargo:rustc-link-lib=static=aaf");
    if !cfg!(target_os = "windows") {
        println!("cargo:rustc-link-lib=dylib=m");
    }

    println!("cargo:rerun-if-changed=shim/aaf_shim.c");
    println!("cargo:rerun-if-changed=shim/aaf_shim.h");
}
