use std::path::Path;

fn main() {
    copy_libav_dlls();
    tauri_build::build()
}

/// Copy the staged libav runtime DLLs (scripts/fetch-libav.ps1 -> src-tauri/libav)
/// next to the built binaries. Cargo puts the target profile dir on the DLL
/// search path when it runs binaries, so this lets `cargo run`, `cargo test`, and
/// the pre-bundle exe load the load-time-linked libav DLLs in dev and CI alike.
/// The shipped installer gets them separately via tauri.conf `bundle.resources`.
/// Version-agnostic: copies whatever `*.dll` fetch-libav staged, so bumping the
/// FFmpeg build only touches that script.
fn copy_libav_dlls() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let src_dir = Path::new(&manifest).join("libav");
    println!("cargo:rerun-if-changed={}", src_dir.display());

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
                "cargo:warning=libav DLLs not staged in {} (run scripts/fetch-libav.ps1)",
                src_dir.display()
            );
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("dll") {
            let name = entry.file_name();
            for dst_dir in [&profile_dir, &deps_dir] {
                let _ = std::fs::create_dir_all(dst_dir);
                let _ = std::fs::copy(&path, dst_dir.join(&name));
            }
        }
    }
}
