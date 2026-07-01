#!/usr/bin/env bash
# Stages the LGPL *shared* FFmpeg libraries (libav*) used for the in-process
# filmstrip decode path (open-once / seek-many). Decode-only, dynamically linked
# -> LGPL, additive to the GPL ffmpeg sidecar (see fetch-ffmpeg.sh). Linux
# counterpart of fetch-libav.ps1.
#
# Stages two things (both gitignored):
#   * src-tauri/vendor/ffmpeg/{include,lib,bin}  -> FFMPEG_DIR for the build
#     (ffmpeg-sys-the-third reads include/ + lib/ shared objects from here).
#   * src-tauri/libav/*.so.*                     -> the 5 runtime sonames,
#     bundled next to the app binary (tauri.linux.conf.json bundle.resources)
#     and copied beside the dev binary (target/debug, target/release) by
#     build.rs so a linked build can load them ($ORIGIN rpath).
#
# Run from anywhere: bash scripts/fetch-libav.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Pinned BtbN autobuild: FFmpeg 8.1.2, linux64, LGPL, shared — the same
# autobuild tag as the win64 asset pinned in fetch-libav.ps1. Lib majors:
# avutil-60 / avcodec-62 / avformat-62 / swscale-9 (matches ffmpeg-the-third
# 5.x, and the same major line as the GPL sidecar).
# NOTE: BtbN prunes old autobuild releases over time. If this URL 404s, pick a
# newer autobuild that still carries the n8.1.x linux64 LGPL shared asset,
# re-run, and update the URL + sha256 below with the printed hash.
url="https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-06-28-13-24/ffmpeg-n8.1.2-linux64-lgpl-shared-8.1.tar.xz"
sha256="671bbd185a8f67b8f05f046ca8a057acfcd5ff2fa2b037b2acd1541e20a31822"
tarball="${TMPDIR:-/tmp}/klipt-libav.tar.xz"

# The decode-only runtime closure: avformat/avcodec/avutil/swscale + swresample
# (avcodec/avformat pull it in). avfilter/avdevice are NOT needed and dropped.
# Soname (`.so.N`) names — what the linked binary actually requests at load.
runtime_libs=(libavformat.so.62 libavcodec.so.62 libavutil.so.60 libswscale.so.9 libswresample.so.6)

echo "Downloading LGPL shared FFmpeg 8.1.2 (libav*)..."
curl -fsSL -o "$tarball" "$url"

got="$(sha256sum "$tarball" | cut -d' ' -f1)"
if [ "$got" != "$sha256" ]; then
    echo "libav checksum mismatch" >&2
    echo "  expected: $sha256" >&2
    echo "  got:      $got" >&2
    exit 1
fi
echo "Checksum verified: $got"

tmp="${TMPDIR:-/tmp}/klipt-libav"
rm -rf "$tmp"
mkdir -p "$tmp"
tar -xJf "$tarball" -C "$tmp"
src="$(find "$tmp" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [ -z "$src" ]; then
    echo "extracted build root not found" >&2
    exit 1
fi

# Stage FFMPEG_DIR (include + lib + bin) for the linker/bindgen. `cp -a` keeps
# the .so symlink farm intact so `-lavcodec` resolves at link time.
vendor="$root/src-tauri/vendor/ffmpeg"
rm -rf "$vendor"
mkdir -p "$vendor"
for d in include lib bin; do
    cp -a "$src/$d" "$vendor/$d"
done
# Clearly-named copy so the shipped file (bundle.resources) reads as the libav
# license, not Klipt's own. LGPL compliance: this ships next to the binary.
cp -f "$src/LICENSE.txt" "$vendor/LICENSE-libav.txt"
echo "Staged FFMPEG_DIR -> $vendor"

# Stage the runtime sonames for bundling, dereferenced (-L) so each staged file
# is a real library, not a dangling symlink once copied out of the tree.
libav_dir="$root/src-tauri/libav"
mkdir -p "$libav_dir"
find "$libav_dir" -maxdepth 1 -name '*.so*' -delete
for lib in "${runtime_libs[@]}"; do
    cp -fL "$src/lib/$lib" "$libav_dir/$lib"
done
echo "Staged ${#runtime_libs[@]} runtime libraries -> $libav_dir"

# Convenience for local dev: drop the libs next to any already-built binary so
# `cargo run` / `tauri dev` can load them without an LD_LIBRARY_PATH dance. (A
# fresh clone builds first, then re-run this, or copy manually — cargo makes
# target/ lazily; build.rs also re-copies on every build.)
for profile in debug release; do
    tdir="$root/src-tauri/target/$profile"
    if [ -d "$tdir" ]; then
        for lib in "${runtime_libs[@]}"; do
            cp -f "$libav_dir/$lib" "$tdir/$lib"
        done
        echo "Copied runtime libraries -> target/$profile"
    fi
done
echo "Done. FFMPEG_DIR is set by .cargo/config.toml; export LIBCLANG_PATH to your distro's libclang dir (e.g. \$(llvm-config --libdir)) for the build."
