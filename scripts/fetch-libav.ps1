# Stages the LGPL *shared* FFmpeg libraries (libav*) used for the in-process
# filmstrip decode path (open-once / seek-many). Decode-only, dynamically linked
# -> LGPL, additive to the GPL ffmpeg.exe sidecar (see fetch-ffmpeg.ps1).
#
# Stages two things (both gitignored):
#   * src-tauri/vendor/ffmpeg/{include,lib,bin}  -> FFMPEG_DIR for the build
#     (ffmpeg-sys-the-third reads include/ + lib/ import libs from here).
#   * src-tauri/libav/*.dll                      -> the 5 runtime DLLs, bundled
#     next to the app exe (tauri.conf bundle.resources) and copied beside the
#     dev exe (target/debug, target/release) so a linked build can load them.
#
# Run from anywhere: powershell -ExecutionPolicy Bypass -File scripts/fetch-libav.ps1
$ErrorActionPreference = "Stop"

$root = Split-Path $PSScriptRoot -Parent

# Pinned BtbN autobuild: FFmpeg 8.1.2, win64, LGPL, shared. Lib majors:
# avutil-60 / avcodec-62 / avformat-62 / swscale-9 (matches ffmpeg-the-third 5.x,
# and the same major line as the 8.1.1 GPL sidecar).
# NOTE: BtbN prunes old autobuild releases over time. If this URL 404s, pick a
# newer autobuild that still carries the n8.1.x LGPL shared asset, re-run, and
# update the URL + sha256 below with the printed hash.
$url    = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-06-28-13-24/ffmpeg-n8.1.2-win64-lgpl-shared-8.1.zip"
$sha256 = "66D8BE9787C5ADB1E3532E96EDF1DC8605AFD1AD2D5113DFABD3547FF2096BA5"
$zip    = Join-Path $env:TEMP "klipt-libav.zip"

# The decode-only runtime closure: avformat/avcodec/avutil/swscale + swresample
# (avcodec/avformat pull it in). avfilter/avdevice are NOT needed and dropped.
$runtimeDlls = @("avformat-62.dll", "avcodec-62.dll", "avutil-60.dll", "swscale-9.dll", "swresample-6.dll")

Write-Host "Downloading LGPL shared FFmpeg 8.1.2 (libav*)..."
Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing

$got = (Get-FileHash $zip -Algorithm SHA256).Hash
if ($got -ne $sha256) {
    Write-Error "libav checksum mismatch`n  expected: $sha256`n  got:      $got"
}
Write-Host "Checksum verified: $got"

$tmp = Join-Path $env:TEMP "klipt-libav"
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
Expand-Archive $zip $tmp -Force
$src = (Get-ChildItem $tmp -Directory | Select-Object -First 1).FullName
if (-not $src) { Write-Error "extracted build root not found" }

# Stage FFMPEG_DIR (include + lib + bin) for the linker/bindgen.
$vendor = Join-Path $root "src-tauri\vendor\ffmpeg"
Remove-Item -Recurse -Force $vendor -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $vendor | Out-Null
foreach ($d in @("include", "lib", "bin")) {
    Copy-Item (Join-Path $src $d) (Join-Path $vendor $d) -Recurse -Force
}
# Clearly-named copy so the shipped file (bundle.resources) reads as the libav
# license, not Klipt's own. LGPL compliance: this ships next to the exe.
Copy-Item (Join-Path $src "LICENSE.txt") (Join-Path $vendor "LICENSE-libav.txt") -Force
Write-Host "Staged FFMPEG_DIR -> $vendor"

# Stage the runtime DLLs for bundling.
$libavDir = Join-Path $root "src-tauri\libav"
New-Item -ItemType Directory -Force $libavDir | Out-Null
Get-ChildItem $libavDir -Filter *.dll -ErrorAction SilentlyContinue | Remove-Item -Force
foreach ($dll in $runtimeDlls) {
    Copy-Item (Join-Path $src "bin\$dll") (Join-Path $libavDir $dll) -Force
}
Write-Host "Staged $($runtimeDlls.Count) runtime DLLs -> $libavDir"

# Convenience for local dev: drop the DLLs next to any already-built exe so
# `cargo run` / `tauri dev` can load them without a PATH dance. (A fresh clone
# builds first, then re-run this, or copy manually — cargo makes target/ lazily.)
foreach ($profile in @("debug", "release")) {
    $tdir = Join-Path $root "src-tauri\target\$profile"
    if (Test-Path $tdir) {
        foreach ($dll in $runtimeDlls) { Copy-Item (Join-Path $libavDir $dll) $tdir -Force }
        Write-Host "Copied runtime DLLs -> target\$profile"
    }
}
Write-Host "Done. Set FFMPEG_DIR=$vendor and LIBCLANG_PATH to your LLVM bin for the build."
