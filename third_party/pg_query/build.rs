#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]

use fs_extra::dir::CopyOptions;
use glob::glob;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
// HACIVAT PATCH §1: `std::process::Command` (used only to probe `protoc`) is not imported.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let build_path = Path::new(".").join("libpg_query");
    let out_header_path = out_dir.join("pg_query").with_extension("h");
    // HACIVAT PATCH §1: `out_protobuf_path` (the regeneration's `.proto` input) is removed.
    let target = env::var("TARGET").unwrap();

    println!("cargo:rerun-if-changed=libpg_query");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=pg_query");

    // Copy the relevant source files to the OUT_DIR, but only if they changed.
    // Without this check, fs_extra::copy_items overwrites files unconditionally,
    // updating their mtimes and causing Cargo to re-run the build script every time.
    let source_paths = vec![
        build_path.join("pg_query").with_extension("h"),
        build_path.join("pg_query_scan_tokens").with_extension("h"),
        build_path.join("postgres_deparse").with_extension("h"),
        build_path.join("Makefile"),
        build_path.join("src"),
        build_path.join("protobuf"),
        build_path.join("vendor"),
    ];

    let hash_file = out_dir.join(".source_hash");
    let current_hash = hash_source_paths(&source_paths);
    let cached_hash = std::fs::read_to_string(&hash_file).unwrap_or_default();

    if cached_hash.trim() != current_hash {
        let copy_options = CopyOptions { overwrite: true, ..CopyOptions::default() };
        fs_extra::copy_items(&source_paths, &out_dir, &copy_options)?;
        std::fs::write(&hash_file, &current_hash)?;
    }

    // Compile the C library.
    let mut build = cc::Build::new();
    build
        .files(glob(out_dir.join("src/*.c").to_str().unwrap()).unwrap().map(|p| p.unwrap()))
        .files(glob(out_dir.join("src/postgres/*.c").to_str().unwrap()).unwrap().map(|p| p.unwrap()))
        .file(out_dir.join("vendor/xxhash/xxhash.c"))
        .file(out_dir.join("vendor/upb/upb.c"))
        .file(out_dir.join("vendor/upb/third_party/utf8_range/utf8_range.c"))
        .file(out_dir.join("protobuf/pg_query.upb_minitable.c"))
        .include(out_dir.join("."))
        .include(out_dir.join("./vendor"))
        .include(out_dir.join("./vendor/upb"))
        .include(out_dir.join("./vendor/upb/third_party/utf8_range"))
        .include(out_dir.join("./src/postgres/include"))
        .include(out_dir.join("./src/include"))
        .warnings(false); // Avoid unnecessary warnings, as they are already considered as part of libpg_query development
    if env::var("PROFILE").unwrap() == "debug" || env::var("DEBUG").unwrap() == "1" {
        build.define("USE_ASSERT_CHECKING", None);
    }
    if !target.contains("msvc") {
        // Required by upb, matching libpg_query's own CFLAGS
        build.flag("-fno-strict-aliasing").flag("-fwrapv");
    }
    if target.contains("windows") {
        build.include(out_dir.join("./src/postgres/include/port/win32"));
        if target.contains("msvc") {
            build.include(out_dir.join("./src/postgres/include/port/win32_msvc"));
        }
    }
    build.compile("pg_query");

    // Generate bindings for Rust
    bindgen::Builder::default()
        .header(out_header_path.to_str().ok_or("Invalid header path")?)
        .generate()
        .map_err(|e| format!("Unable to generate bindings: {e}"))?
        .write_to_file(out_dir.join("bindings.rs"))?;

    // HACIVAT PATCH §1: the protobuf regeneration branch is removed. It ran prost-build when
    // `REGENERATE_PROTOBUF` was set or `protoc --version` could be spawned, and copied the result
    // over `src/protobuf.rs` inside `CARGO_MANIFEST_DIR`. The shipped, pre-generated
    // `src/protobuf.rs` is always used, so a `protoc` on `PATH` changes nothing.

    Ok(())
}

/// Hash the contents of all files under the given paths to detect changes.
fn hash_source_paths(paths: &[PathBuf]) -> String {
    let mut hasher = DefaultHasher::new();
    for path in paths {
        hash_path(&mut hasher, path);
    }
    format!("{:x}", hasher.finish())
}

fn hash_path(hasher: &mut DefaultHasher, path: &Path) {
    if path.is_file() {
        if let Ok(contents) = std::fs::read(path) {
            hasher.write(path.to_string_lossy().as_bytes());
            hasher.write(&contents);
        }
    } else if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                hash_path(hasher, &entry.path());
            }
        }
    }
}
