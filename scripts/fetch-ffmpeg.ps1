# Stages ffmpeg.exe as a Tauri sidecar (gitignored, not committed).
# Run from anywhere: powershell -ExecutionPolicy Bypass -File scripts/fetch-ffmpeg.ps1
$ErrorActionPreference = "Stop"

$root = Split-Path $PSScriptRoot -Parent
$dest = Join-Path $root "src-tauri\binaries"
New-Item -ItemType Directory -Force $dest | Out-Null

$triple = (rustc --print host-tuple).Trim()
Write-Host "Target triple: $triple"

# Prefer an ffmpeg already on PATH (e.g. installed via: winget install Gyan.FFmpeg).
$src = (Get-Command ffmpeg -ErrorAction SilentlyContinue).Source
if (-not $src) {
  Write-Error "ffmpeg not found on PATH. Install it first: winget install Gyan.FFmpeg"
}
$bin = Split-Path $src -Parent

Copy-Item (Join-Path $bin "ffmpeg.exe")  (Join-Path $dest "ffmpeg-$triple.exe")  -Force
Write-Host "Staged ffmpeg sidecar into $dest"
