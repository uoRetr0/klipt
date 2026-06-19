# Bundle FFmpeg as a Tauri sidecar

Klipt invokes FFmpeg for the actual trim. We bundle FFmpeg as a Tauri **sidecar** binary
shipped with the app, rather than requiring the user to install FFmpeg or relying on one
already on PATH.

Why: zero-config for the user (it Just Works on a clean machine), and a checksum-verified,
version-stable build means predictable behaviour. The cost is a larger installer (FFmpeg is tens of MB) — a
deliberate trade of install size for reliability and a frictionless first run. We accept
the size hit because "drop a clip, trim, done" with no setup is the core promise.

The sidecar is per-platform. Tauri resolves it by the host target triple
(`src-tauri/binaries/ffmpeg-<triple>`, with a `.exe` suffix on Windows), so the calling
code is identical across platforms — only the staged binary differs. A `fetch-ffmpeg`
script per platform downloads a SHA256-verified GPL build into that path (Windows:
Gyan.dev; macOS: ffmpeg-static; Linux: BtbN static). The Windows and macOS builds are
pinned to a fixed hash; BtbN ships a rolling `latest` tag, so the Linux build is verified
against that release's own published checksums rather than a hard-coded hash. The builds
include `libx264` (the universal CPU
encode path) and the NVIDIA `*_nvenc` encoders, which are used only when an NVIDIA driver
is present — otherwise `compress_clip` detects the unavailable-encoder stderr and falls
back to libx264, so the GPU brand and OS never gate functionality.
