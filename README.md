# Klipt

A lightweight, fast desktop app for trimming game clips down to a single short
moment — **losslessly**, with no re-encode. Works with recordings from any
capture tool (NVIDIA ShadowPlay, OBS, AMD ReLive, Xbox Game Bar, …) in any common
container (mp4, mov, mkv, avi, webm, m4v). Built to replace ClipChamp and beat
LosslessCut on design.

- **Lossless** — FFmpeg stream-copy (`-c copy`), near-instant, zero quality loss.
- **Lightweight** — Tauri + Svelte. A small native shell, not a Chromium bundle.
- **Simple** — drop a clip (or pick from your recent clips folder), drag the in/out
  handles to the funny moment, hit Trim. Output lands next to the source as
  `<name>_trim.mp4`, never overwriting the original.
- **Any GPU** — the optional "compress to share" re-encode uses NVIDIA NVENC when
  present and falls back to a CPU (libx264) encode on AMD/Intel or GPU-less machines.

Windows is the primary target; Linux is supported (see below).

## Design & decisions

- Glossary: [`CONTEXT.md`](./CONTEXT.md)
- Architecture decisions: [`docs/adr/`](./docs/adr/)

## Develop

FFmpeg is NOT required on PATH — the fetch script downloads a pinned GPL build and
stages it as the Tauri sidecar (`src-tauri/binaries/ffmpeg-<triple>`, gitignored
because it is large). It verifies a SHA256 checksum before staging.

### Windows

Prerequisites: **Node**, **Rust** (`stable-x86_64-pc-windows-msvc`) + **VS C++ Build Tools**.

```sh
npm install
powershell -ExecutionPolicy Bypass -File scripts/fetch-ffmpeg.ps1   # pinned Gyan.dev ffmpeg 8.1.1 (~97 MB)
npm run tauri dev
```

### Linux (Fedora, etc.)

Prerequisites: **Node**, **Rust** (`stable-x86_64-unknown-linux-gnu`), plus the Tauri
WebKitGTK system libraries. On Fedora:

```sh
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file \
  libappindicator-gtk3-devel librsvg2-devel gcc gcc-c++ make
```

(On Debian/Ubuntu the equivalents are `libwebkit2gtk-4.1-dev`, `build-essential`,
`libssl-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `file`, `wget`, `curl`.)

```sh
npm install
bash scripts/fetch-ffmpeg.sh        # pinned BtbN ffmpeg n8.1 static build
npm run tauri dev
```

The Linux script downloads the [BtbN](https://github.com/BtbN/FFmpeg-Builds) GPL static
build (x86_64 or aarch64, auto-detected from the Rust host triple) — encoders
`libx264`/`libx265`/`aac` for the CPU path plus `*_nvenc` for NVIDIA machines. NVENC is
only used when an NVIDIA driver is present; otherwise compression falls back to libx264
automatically. No system FFmpeg install is needed.

## Build

```sh
npm run tauri build
```

`bundle.targets` is `"all"`, so this emits an `.msi`/NSIS installer on Windows and
`.rpm` + `.deb` + AppImage on Linux, with the FFmpeg sidecar bundled into each.
