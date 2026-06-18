#!/usr/bin/env bash
# Stages an ffmpeg binary as a Tauri sidecar (gitignored, not committed).
# Linux/macOS counterpart of fetch-ffmpeg.ps1. Downloads a pinned GPL *static*
# build from BtbN/FFmpeg-Builds (GitHub releases) — static means it needs no
# system ffmpeg and runs the same on Fedora, Ubuntu, Arch, etc.
#
#   bash scripts/fetch-ffmpeg.sh
#
# Encoders: libx264 / libx265 (CPU, GPL), aac, plus h264_nvenc / hevc_nvenc /
# av1_nvenc — the NVENC encoders are only used if an NVIDIA driver is present at
# runtime; on any other GPU (AMD/Intel) or none, Klipt falls back to libx264
# automatically. Demuxers: mp4, mov, mkv, avi, webm, m4v.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$root/src-tauri/binaries"
mkdir -p "$dest"

# Match the sidecar name Tauri looks for: ffmpeg-<host-triple>. `host-tuple` is
# the modern rustc query; fall back to parsing `rustc -vV` on older toolchains.
triple="$(rustc --print host-tuple 2>/dev/null | tr -d '[:space:]' || true)"
if [ -z "$triple" ]; then
  triple="$(rustc -vV | awk -F': ' '/^host:/{print $2}' | tr -d '[:space:]')"
fi
echo "Target triple: $triple"

# Map the Rust target triple → the matching BtbN asset. Pinned to the n8.1
# series to mirror the Windows build (Gyan 8.1.1). Extend this case to support
# more platforms.
case "$triple" in
  x86_64-unknown-linux-gnu)  asset="ffmpeg-n8.1-latest-linux64-gpl-8.1.tar.xz" ;;
  aarch64-unknown-linux-gnu) asset="ffmpeg-n8.1-latest-linuxarm64-gpl-8.1.tar.xz" ;;
  *)
    echo "error: no prebuilt ffmpeg mapping for triple '$triple'." >&2
    echo "Install ffmpeg via your package manager and copy or symlink it to:" >&2
    echo "  $dest/ffmpeg-$triple" >&2
    echo "e.g.  ln -sf \"\$(command -v ffmpeg)\" \"$dest/ffmpeg-$triple\"" >&2
    exit 1
    ;;
esac

base="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $asset ..."
curl -fL --retry 3 -o "$tmp/ffmpeg.tar.xz" "$base/$asset"
curl -fL --retry 3 -o "$tmp/checksums.sha256" "$base/checksums.sha256"

# The `latest` tag is a rolling rebuild, so the n8.1 hash changes over time.
# We verify against the release's own checksum file rather than a hardcoded
# value — this still catches a corrupted or truncated download. (If you need a
# stronger supply-chain guarantee, pin a specific autobuild tag + hash here.)
echo "Verifying checksum..."
expected="$(awk -v f="$asset" '{n=$2; sub(/^\*/,"",n); if (n==f) {print $1; exit}}' "$tmp/checksums.sha256")"
if [ -z "$expected" ]; then
  echo "error: $asset not found in checksums.sha256" >&2
  exit 1
fi
got="$(sha256sum "$tmp/ffmpeg.tar.xz" | awk '{print $1}')"
if [ "$got" != "$expected" ]; then
  echo "error: ffmpeg checksum mismatch" >&2
  echo "  expected: $expected" >&2
  echo "  got:      $got" >&2
  exit 1
fi
echo "Checksum verified: $got"

echo "Extracting..."
tar -xf "$tmp/ffmpeg.tar.xz" -C "$tmp"
src_bin="$(find "$tmp" -type f -path '*/bin/ffmpeg' | head -n1)"
[ -z "$src_bin" ] && src_bin="$(find "$tmp" -type f -name ffmpeg | head -n1)"
if [ -z "$src_bin" ]; then
  echo "error: ffmpeg binary not found inside the archive" >&2
  exit 1
fi

out="$dest/ffmpeg-$triple"
cp -f "$src_bin" "$out"
chmod +x "$out"
size_mb="$(du -m "$out" | awk '{print $1}')"
echo "Staged ffmpeg sidecar (${size_mb} MB) -> $out"
