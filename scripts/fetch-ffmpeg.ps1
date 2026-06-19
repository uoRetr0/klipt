# Stages ffmpeg.exe as a Tauri sidecar (gitignored, not committed).
# Downloads a pinned slim GPL build from Gyan.dev (Gyan D's codexffmpeg).
# Run from anywhere: powershell -ExecutionPolicy Bypass -File scripts/fetch-ffmpeg.ps1
$ErrorActionPreference = "Stop"

$root = Split-Path $PSScriptRoot -Parent
$dest = Join-Path $root "src-tauri\binaries"
New-Item -ItemType Directory -Force $dest | Out-Null

$triple = (rustc --print host-tuple).Trim()
Write-Host "Target triple: $triple"

# Pinned Gyan essentials GPL build — ffmpeg 8.1.1, static, Windows x64.
# Encoders included: h264_nvenc, libx264 (GPL), aac, hevc_nvenc, av1_nvenc, libx265.
# Demuxers included: mp4, mov, mkv, avi, webm, m4v (all present in essentials).
# Filters included: scale, yuv420p, and general video filter chain.
# Size: ~96.8 MB (vs ~217 MB for the full_build).
$url    = "https://github.com/GyanD/codexffmpeg/releases/download/8.1.1/ffmpeg-8.1.1-essentials_build.zip"
$sha256 = "6F58CE889F59C311410F7D2B18895B33C03456463486F3B1EBC93D97A0F54541"
$zip    = Join-Path $env:TEMP "klipt-ffmpeg.zip"

Write-Host "Downloading ffmpeg 8.1.1-essentials_build..."
Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing

$got = (Get-FileHash $zip -Algorithm SHA256).Hash
if ($got -ne $sha256) {
    Write-Error "ffmpeg checksum mismatch`n  expected: $sha256`n  got:      $got"
}
Write-Host "Checksum verified: $got"

$tmp = Join-Path $env:TEMP "klipt-ffmpeg"
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
Expand-Archive $zip $tmp -Force

$exe = Get-ChildItem $tmp -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
if (-not $exe) {
    Write-Error "ffmpeg.exe not found inside the downloaded archive"
}

$outPath = Join-Path $dest "ffmpeg-$triple.exe"
Copy-Item $exe.FullName $outPath -Force
Write-Host "Staged slim ffmpeg sidecar ($([math]::Round($exe.Length/1MB, 1)) MB) -> $outPath"
