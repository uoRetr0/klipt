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
Tools**, and **FFmpeg** on PATH (`winget install Gyan.FFmpeg`).

```sh
npm install
powershell -ExecutionPolicy Bypass -File scripts/fetch-ffmpeg.ps1   # stage ffmpeg sidecars
npm run tauri dev
```

The FFmpeg sidecars (`src-tauri/binaries/`) are gitignored because they're large; the
script above stages them from your local FFmpeg install.

## Build

```sh
npm run tauri build
```
