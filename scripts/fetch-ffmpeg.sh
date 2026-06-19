#!/usr/bin/env bash
# Stages an ffmpeg binary as a Tauri sidecar (gitignored, not committed).
# Linux/macOS counterpart of fetch-ffmpeg.ps1 — downloads a pinned GPL *static*
# build so it needs no system ffmpeg and runs the same everywhere.
#
#   bash scripts/fetch-ffmpeg.sh
#
#   * Linux  → BtbN/FFmpeg-Builds (n8.1 series), verified against the release's
#              own checksums.sha256.
#   * macOS  → eugeneware/ffmpeg-static (b6.1.1), verified against a pinned hash.
#
# Encoders in every build: libx264 / libx265 (CPU, GPL) + aac; the Linux build
# also carries *_nvenc (used only when an NVIDIA driver is present — otherwise
# Klipt falls back to libx264). Demuxers: mp4, mov, mkv, avi, webm, m4v.
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

# sha256 of a file, portable across Linux (sha256sum) and macOS (shasum).
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

out="$dest/ffmpeg-$triple"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

case "$triple" in
  # ---- Linux: BtbN GPL static tarball, verified against its manifest ----------
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu)
    case "$triple" in
      x86_64-unknown-linux-gnu)  asset="ffmpeg-n8.1-latest-linux64-gpl-8.1.tar.xz" ;;
      aarch64-unknown-linux-gnu) asset="ffmpeg-n8.1-latest-linuxarm64-gpl-8.1.tar.xz" ;;
    esac
    base="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest"

    echo "Downloading $asset ..."
    curl -fL --retry 3 -o "$tmp/ffmpeg.tar.xz" "$base/$asset"
    curl -fL --retry 3 -o "$tmp/checksums.sha256" "$base/checksums.sha256"

    # The `latest` tag is a rolling rebuild, so the n8.1 hash changes over time.
    # Verify against the release's own checksum file rather than a hardcoded value
    # — this still catches a corrupted or truncated download.
    echo "Verifying checksum..."
    expected="$(awk -v f="$asset" '{n=$2; sub(/^\*/,"",n); if (n==f) {print $1; exit}}' "$tmp/checksums.sha256")"
    if [ -z "$expected" ]; then
      echo "error: $asset not found in checksums.sha256" >&2; exit 1
    fi
    got="$(sha256 "$tmp/ffmpeg.tar.xz")"
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
      echo "error: ffmpeg binary not found inside the archive" >&2; exit 1
    fi
    cp -f "$src_bin" "$out"
    ;;

  # ---- macOS: eugeneware/ffmpeg-static raw binary, pinned hash ----------------
  aarch64-apple-darwin|x86_64-apple-darwin)
    rel="b6.1.1"
    case "$triple" in
      aarch64-apple-darwin)
        url="https://github.com/eugeneware/ffmpeg-static/releases/download/$rel/ffmpeg-darwin-arm64"
        expected="a90e3db6a3fd35f6074b013f948b1aa45b31c6375489d39e572bea3f18336584" ;;
      x86_64-apple-darwin)
        url="https://github.com/eugeneware/ffmpeg-static/releases/download/$rel/ffmpeg-darwin-x64"
        expected="ebdddc936f61e14049a2d4b549a412b8a40deeff6540e58a9f2a2da9e6b18894" ;;
    esac

    echo "Downloading $(basename "$url") ..."
    curl -fL --retry 3 -o "$tmp/ffmpeg" "$url"
    echo "Verifying checksum..."
    got="$(sha256 "$tmp/ffmpeg")"
    if [ "$got" != "$expected" ]; then
      echo "error: ffmpeg checksum mismatch" >&2
      echo "  expected: $expected" >&2
      echo "  got:      $got" >&2
      exit 1
    fi
    echo "Checksum verified: $got"
    cp -f "$tmp/ffmpeg" "$out"
    ;;

  *)
    echo "error: no prebuilt ffmpeg mapping for triple '$triple'." >&2
    echo "Install ffmpeg via your package manager and copy or symlink it to:" >&2
    echo "  $dest/ffmpeg-$triple" >&2
    echo "e.g.  ln -sf \"\$(command -v ffmpeg)\" \"$dest/ffmpeg-$triple\"" >&2
    exit 1
    ;;
esac

chmod +x "$out"
size_mb="$(du -m "$out" | awk '{print $1}')"
echo "Staged ffmpeg sidecar (${size_mb} MB) -> $out"
