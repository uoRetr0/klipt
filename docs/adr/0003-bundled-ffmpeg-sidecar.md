# Bundle FFmpeg as a Tauri sidecar

Klipt invokes FFmpeg for the actual trim. We bundle `ffmpeg.exe` as a Tauri **sidecar**
binary shipped with the app, rather than requiring the user to install FFmpeg or relying
on one already on PATH.

Why: zero-config for the user (it Just Works on a clean machine), and a pinned version
means predictable behaviour. The cost is a larger installer (FFmpeg is tens of MB) — a
deliberate trade of install size for reliability and a frictionless first run. We accept
the size hit because "drop a clip, trim, done" with no setup is the core promise.
