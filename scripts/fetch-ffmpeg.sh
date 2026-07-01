#!/usr/bin/env bash
# Stages the ffmpeg binary as a Tauri sidecar (gitignored, not committed).
# Linux counterpart of fetch-ffmpeg.ps1: downloads a pinned static GPL build
# from BtbN's FFmpeg-Builds autobuilds.
# Run from anywhere: bash scripts/fetch-ffmpeg.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$root/src-tauri/binaries"
mkdir -p "$dest"

triple="$(rustc --print host-tuple | tr -d '[:space:]')"
echo "Target triple: $triple"

# Pinned BtbN autobuild: FFmpeg 8.1.2, linux64, GPL, static binaries — the same
# autobuild tag as fetch-libav.sh / fetch-libav.ps1, and the same 8.1.x major
# line as the Windows sidecar (Gyan 8.1.1 essentials).
# Encoders included: h264_nvenc, libx264 (GPL), aac, hevc_nvenc, av1_nvenc, libx265.
# NOTE: BtbN prunes old autobuild releases over time. If this URL 404s, pick a
# newer autobuild that still carries an n8.1.x linux64 GPL asset, re-run, and
# update the URL + sha256 below with the printed hash.
url="https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-06-28-13-24/ffmpeg-n8.1.2-linux64-gpl-8.1.tar.xz"
sha256="1db4ef08de1bc61493a86905a89f6e05d6aa8a4608425d54273127db85dfe798"
tarball="${TMPDIR:-/tmp}/klipt-ffmpeg.tar.xz"

echo "Downloading ffmpeg n8.1.2 linux64 GPL..."
curl -fsSL -o "$tarball" "$url"

got="$(sha256sum "$tarball" | cut -d' ' -f1)"
if [ "$got" != "$sha256" ]; then
    echo "ffmpeg checksum mismatch" >&2
    echo "  expected: $sha256" >&2
    echo "  got:      $got" >&2
    exit 1
fi
echo "Checksum verified: $got"

tmp="${TMPDIR:-/tmp}/klipt-ffmpeg"
rm -rf "$tmp"
mkdir -p "$tmp"
tar -xJf "$tarball" -C "$tmp"

exe="$(find "$tmp" -type f -path '*/bin/ffmpeg' | head -n 1)"
if [ -z "$exe" ]; then
    echo "ffmpeg binary not found inside the downloaded archive" >&2
    exit 1
fi

# App-specific sidecar name: Tauri's .deb installs external binaries into
# /usr/bin, so a bare "ffmpeg" would collide with the distro's ffmpeg package.
out="$dest/klipt-ffmpeg-$triple"
cp -f "$exe" "$out"
chmod +x "$out"
echo "Staged ffmpeg sidecar ($(du -m "$out" | cut -f1) MB) -> $out"
