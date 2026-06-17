# Klipt

A lightweight, fast Windows desktop app for trimming game clips (primarily NVIDIA
ShadowPlay recordings) down to a single short moment — **losslessly**, with no
re-encode. Built to replace ClipChamp and beat LosslessCut on design.

- **Lossless** — FFmpeg stream-copy (`-c copy`), near-instant, zero quality loss.
- **Lightweight** — Tauri + Svelte. A small native shell, not a Chromium bundle.
- **Simple** — drop a clip (or pick from your recent ShadowPlay folder), drag the in/out
  handles to the funny moment, hit Trim. Output lands next to the source as
  `<name>_trim.mp4`, never overwriting the original.

## Design & decisions

- Glossary: [`CONTEXT.md`](./CONTEXT.md)
- Architecture decisions: [`docs/adr/`](./docs/adr/)

## Develop

Prerequisites: **Node**, **Rust** (`stable-x86_64-pc-windows-msvc`) + **VS C++ Build
Tools**. FFmpeg is NOT required on PATH — the script below downloads a pinned slim GPL
build automatically.

```sh
npm install
powershell -ExecutionPolicy Bypass -File scripts/fetch-ffmpeg.ps1   # downloads pinned slim ffmpeg (~97 MB)
npm run tauri dev
```

The FFmpeg sidecar (`src-tauri/binaries/ffmpeg-<triple>.exe`) is gitignored because it is
large; the script above downloads the pinned Gyan.dev essentials GPL build (ffmpeg 8.1.1,
~97 MB — encoders: h264\_nvenc, libx264, aac) and verifies its SHA256 checksum before
staging it. No system FFmpeg install is needed.

## Build

```sh
npm run tauri build
```
