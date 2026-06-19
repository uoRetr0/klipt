#!/usr/bin/env bash
# One-shot Linux setup for Klipt: installs build tools + the H.264 video codec,
# stages the ffmpeg sidecar, builds the optimized release, and installs the app.
#
#   bash scripts/linux-install.sh
#
# Supports Fedora/RHEL (dnf) and Debian/Ubuntu (apt). After it finishes, launch
# "Klipt" from your app menu or run `klipt`.
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if command -v dnf >/dev/null 2>&1; then
  echo "==> Fedora/RHEL detected — installing dependencies"
  # RPM Fusion (free) carries gstreamer1-libav, the H.264 decoder WebKitGTK needs
  # to play clips in the preview. Best-effort: ignore if it's already set up.
  sudo dnf install -y "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm" || true
  sudo dnf install -y \
    rust cargo nodejs npm \
    webkit2gtk4.1-devel gtk3-devel libsoup3-devel openssl-devel \
    libappindicator-gtk3-devel librsvg2-devel \
    gcc gcc-c++ make file pkgconf-pkg-config rpm-build \
    gstreamer1-libav gstreamer1-plugins-good
  PKG_GLOB="./src-tauri/target/release/bundle/rpm/*.rpm"
  INSTALL_CMD="sudo dnf install -y"
elif command -v apt-get >/dev/null 2>&1; then
  echo "==> Debian/Ubuntu detected — installing dependencies"
  sudo apt-get update
  sudo apt-get install -y \
    rustc cargo nodejs npm \
    libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libssl-dev \
    libayatana-appindicator3-dev librsvg2-dev build-essential file pkg-config \
    gstreamer1.0-libav gstreamer1.0-plugins-good
  PKG_GLOB="./src-tauri/target/release/bundle/deb/*.deb"
  INSTALL_CMD="sudo apt-get install -y"
else
  echo "error: need dnf (Fedora) or apt (Debian/Ubuntu)." >&2
  exit 1
fi

echo "==> Installing JS deps"
npm install

echo "==> Staging the ffmpeg sidecar"
bash scripts/fetch-ffmpeg.sh

echo "==> Building Klipt (optimized release — this takes a few minutes the first time)"
npm run tauri build

echo "==> Installing the package"
# shellcheck disable=SC2086
$INSTALL_CMD $PKG_GLOB

echo
echo "✅ Done. Launch \"Klipt\" from your app menu, or run:  klipt"
