use std::path::Path;

fn main() {
    copy_libav_runtime_libs();

    // Linux: the dev/test binaries and the shipped app both load the libav .so
    // files from beside the binary (copied there above; bundled next to it as
    // resources), so bake matching rpaths in. `$ORIGIN/../lib/...` covers the
    // installed .deb layout (binary in usr/bin, resources in usr/lib/<app>) and
    // the AppImage layout (linuxdeploy stages libs in usr/lib).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib");
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib/Klipt");
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib/klipt");
    }

    tauri_build::build()
}

/// Copy the staged libav runtime libraries (scripts/fetch-libav.ps1 or .sh ->
/// src-tauri/libav) next to the built binaries. Cargo puts the target profile
/// dir on the library search path when it runs binaries, so this lets
/// `cargo run`, `cargo test`, and the pre-bundle binary load the
/// load-time-linked libav libraries in dev and CI alike. The shipped installer
/// gets them separately via tauri conf `bundle.resources` (see the per-platform
/// tauri.windows/linux.conf.json). Version-agnostic: copies whatever `*.dll`
/// (Windows) or `*.so*` (Linux) fetch-libav staged, so bumping the FFmpeg build
/// only touches that script.
fn copy_libav_runtime_libs() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let src_dir = Path::new(&manifest).join("libav");
    println!("cargo:rerun-if-changed={}", src_dir.display());

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // OUT_DIR = target/<profile>/build/<pkg-hash>/out -> the profile dir is 3 up.
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let profile_dir = match Path::new(&out_dir).ancestors().nth(3) {
        Some(p) => p.to_path_buf(),
        None => return,
    };
    let deps_dir = profile_dir.join("deps");

    let entries = match std::fs::read_dir(&src_dir) {
        Ok(e) => e,
        Err(_) => {
            println!(
                "cargo:warning=libav runtime libs not staged in {} (run scripts/fetch-libav.ps1 or scripts/fetch-libav.sh)",
                src_dir.display()
            );
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Windows DLLs end in .dll; Linux sonames end in `.so` or carry a
        // version suffix after it (libavcodec.so.62), so match both shapes.
        let is_runtime_lib = match target_os.as_str() {
            "windows" => name_str.ends_with(".dll"),
            "linux" => name_str.ends_with(".so") || name_str.contains(".so."),
            _ => false,
        };
        if is_runtime_lib && path.is_file() {
            for dst_dir in [&profile_dir, &deps_dir] {
                let _ = std::fs::create_dir_all(dst_dir);
                let _ = std::fs::copy(&path, dst_dir.join(&name));
            }
        }
    }
}
