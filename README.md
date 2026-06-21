# Klipt

A lightweight, fast Windows desktop app for turning game clips (primarily NVIDIA
ShadowPlay recordings) into something shareable — trim a moment **losslessly**, or
compress / crop / convert it when you need a smaller file or a different format.
Built to replace ClipChamp and beat LosslessCut on design.

- **Lossless first** — the default Trim is an FFmpeg stream-copy (`-c copy`):
  near-instant, zero quality loss, output lands next to the source as
  `<name>_trim.mp4` and never overwrites the original.
- **Lightweight** — Tauri + Svelte. A small native shell, not a Chromium bundle.
- **GPU-aware** — re-encodes use NVIDIA NVENC when available, with an automatic
  libx264 CPU fallback.

## Features

### Export modes
- **Trim (lossless)** — stream-copy the selected Region, no re-encode.
- **Compress** — re-encode to a target **file size** (MB, two-pass) or a **quality
  preset** (480p / 720p / 1080p / source). NVENC GPU encode with libx264 fallback,
  and a live progress bar streamed from FFmpeg.
- **Crop** — drag a rectangle over the preview to keep just that area of the frame
  (re-encodes via the Compress path; crop is applied before scaling).
- **GIF / WebP** — animated export of the Region: palette-optimised GIF or
  true-colour animated WebP, with configurable fps and width.
- **Audio only** — extract the Region's audio as **M4A** (AAC stream-copy, lossless
  and instant for ShadowPlay sources) or **MP3** (re-encoded via libmp3lame).
- Keep-or-drop the audio track on any video export.

### Timeline & playback
- In / out handles to set the Region; slide the whole Region without resizing it.
- Loop playback, scoped to the Region or the whole clip.
- Frame-stepping with a frame readout.
- Audio **waveform** overlay and a **filmstrip** you can hover-scrub for precise cuts.
- Preview volume control (does not affect exported audio).

### Library
- Recent-clips grid from your watched folder, virtualised so it stays fast on large
  libraries.
- **Search**, **sort** by date / name / size (with direction toggle), and filter by
  **game** and by **date** range.
- Card hover-preview scrubbing.
- Right-click a card to **Reveal in folder**, **Rename**, or **Delete**.
- Delete moves the original to the Recycle Bin with a one-click **Undo** to restore it.

### Settings & quality-of-life
- Configurable output location and a naming-scheme template
  (tokens: `{name}`, `{stem}`, `{action}`, `{ext}`, `{n}`), plus a theme accent.
- Remembers your last export settings across sessions.
- Keyboard shortcuts in the editor:
  - `Enter` export · `Esc` back · `Space` play/pause
  - `I` / `O` set in / out · `J` / `K` / `L` shuttle rewind / pause / forward
  - `←` `→` (or `,` `.`) step one frame

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
~97 MB — includes h264\_nvenc, libx264, aac, libmp3lame, libwebp) and verifies its SHA256
checksum before staging it. No system FFmpeg install is needed.

## Build

```sh
npm run tauri build
```

## Releasing

```sh
npm run set-version -- 0.4.1          # bumps tauri.conf.json, package.json, Cargo.toml, Cargo.lock
git commit -am "chore: bump version to 0.4.1"
git push origin main
git tag -a v0.4.1 -m "Klipt 0.4.1" && git push origin v0.4.1
```

The tag push triggers `.github/workflows/release.yml`, which builds the Windows installer
(`.msi` + NSIS `.exe`) on GitHub's runner and attaches it to a **draft** release. Review it
on the [Releases page](https://github.com/uoRetr0/klipt/releases) and click **Publish**.
