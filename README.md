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

FFmpeg is NOT required on PATH — the fetch script downloads a checksum-verified GPL
build and stages it as the Tauri sidecar (`src-tauri/binaries/ffmpeg-<triple>`,
gitignored because it is large). Every build is SHA256-verified before staging: the
Windows (Gyan.dev) and macOS (ffmpeg-static) builds are pinned to a fixed hash, while
the Linux (BtbN) build tracks the rolling `latest` n8.1 tag and is verified against
that release's own published checksums.

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
bash scripts/fetch-ffmpeg.sh        # BtbN ffmpeg n8.1 static build (manifest-verified)
npm run tauri dev
```

The Linux script downloads the [BtbN](https://github.com/BtbN/FFmpeg-Builds) GPL static
build (x86_64 or aarch64, auto-detected from the Rust host triple) — encoders
`libx264`/`libx265`/`aac` for the CPU path plus `*_nvenc` for NVIDIA machines. NVENC is
only used when an NVIDIA driver is present; otherwise compression falls back to libx264
automatically. No system FFmpeg install is needed. BtbN's `latest` tag is a rolling
rebuild, so the script verifies the download against the checksums published in that
same release rather than a hard-coded hash (the Windows/macOS builds are hash-pinned).

## Build

```sh
npm run tauri build
```

`bundle.targets` is `"all"`, so this emits an `.msi`/NSIS installer on Windows and
`.rpm` + `.deb` + AppImage on Linux, with the FFmpeg sidecar bundled into each.

## Releasing

Releases are built in CI — you don't build installers locally. Cutting one:

```sh
npm run set-version -- 0.3.3          # bumps tauri.conf.json, package.json, Cargo.toml, Cargo.lock
git commit -am "chore: bump version to 0.3.3"
git push origin main
git tag -a v0.3.3 -m "Klipt 0.3.3" && git push origin v0.3.3
```

The tag push triggers `.github/workflows/release.yml`, which builds every OS's installers
on GitHub's runners and attaches them to a **draft** release:

- **Windows** — `.msi` + NSIS `.exe`
- **Linux** — `.deb` + `.rpm` + AppImage
- **macOS** — `.dmg` (arm64 + Intel, built separately)

Review the draft on the [Releases page](https://github.com/uoRetr0/klipt/releases) and click
**Publish**. The tag must match the version in `tauri.conf.json` (the `set-version` script keeps
them in sync). macOS bundles are unsigned, so first launch needs right-click → Open (or
`xattr -cr Klipt.app`); add Apple Developer signing via tauri-action's `APPLE_*` secrets later.
