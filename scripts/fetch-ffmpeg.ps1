# Fetch the FFmpeg dev package that this project builds against.
#
# ac-ffmpeg supports FFmpeg v4-v7 ONLY. The BtbN "latest" release now ships
# 8.x/9.x, so we pin a specific n7.1.1 autobuild: switching to 8/9 requires
# replacing ac-ffmpeg first.
#
# Usage (from the repository root):
#     pwsh -File scripts/fetch-ffmpeg.ps1
#
# Result: third_party/ffmpeg/{include,lib,bin} populated with headers, MSVC
# import libraries, and runtime DLLs.

$ErrorActionPreference = 'Stop'

$Url = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2025-07-31-14-15/ffmpeg-n7.1.1-56-gc2184b65d2-win64-gpl-shared-7.1.zip'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$Cache = Join-Path $RepoRoot '.cache'
$Zip = Join-Path $Cache 'ffmpeg7.zip'
$Dest = Join-Path $RepoRoot 'third_party\ffmpeg'

New-Item -ItemType Directory -Force -Path $Cache | Out-Null

if (-not (Test-Path $Zip)) {
    Write-Host "Downloading FFmpeg 7.1.1 shared build..."
    Invoke-WebRequest -Uri $Url -OutFile $Zip
} else {
    Write-Host "Using cached $Zip"
}

$Extract = Join-Path $Cache 'ffmpeg_extract'
if (Test-Path $Extract) { Remove-Item -Recurse -Force $Extract }
New-Item -ItemType Directory -Force -Path $Extract | Out-Null

Write-Host "Extracting..."
Expand-Archive -Path $Zip -DestinationPath $Extract -Force

# The archive has a single top-level directory; strip it so that
# third_party/ffmpeg/bin, /include and /lib end up where .cargo/config.toml
# and the runtime DLL copy expect them.
$Inner = Get-ChildItem -Path $Extract -Directory | Select-Object -First 1
if (-not $Inner) { throw "Unexpected archive layout: no top-level directory in $Extract" }

if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
foreach ($Part in 'bin', 'include', 'lib') {
    $Src = Join-Path $Inner.FullName $Part
    if (-not (Test-Path $Src)) { throw "Archive is missing '$Part' -- wrong package variant?" }
    Copy-Item -Recurse -Force $Src (Join-Path $Dest $Part)
}

# Guard against the failure mode this script exists to prevent: a directory
# named `include` that actually contains DLLs and no headers.
$Header = Join-Path $Dest 'include\libavcodec\avcodec.h'
if (-not (Test-Path $Header)) {
    throw "third_party/ffmpeg/include has no libavcodec headers -- extraction produced a bogus tree."
}
$Lib = Join-Path $Dest 'lib\avcodec.lib'
if (-not (Test-Path $Lib)) {
    throw "third_party/ffmpeg/lib has no avcodec.lib import library."
}

# Mirror the import libraries into lib\x64 as well.
#
# The BtbN package puts them directly in lib\, but environments set up for this
# project commonly persist
#     FFMPEG_LIB_DIR=<repo>\third_party\ffmpeg\lib\x64
# which then dangles, and a plain `cargo build` fails to find the import
# libraries. Keeping the mirror means that variable resolves as-is. The whole
# set is well under a megabyte.
$x64 = Join-Path $Dest 'lib\x64'
New-Item -ItemType Directory -Force -Path $x64 | Out-Null
Copy-Item (Join-Path $Dest 'lib\*.lib') $x64 -Force
Copy-Item (Join-Path $Dest 'lib\*.def') $x64 -Force

if (-not (Test-Path (Join-Path $x64 'avcodec.lib'))) {
    throw "Failed to mirror the import libraries into third_party/ffmpeg/lib/x64."
}

Write-Host "OK: $Dest"
Write-Host "Runtime DLLs are in third_party\ffmpeg\bin -- scripts/build-windows.ps1 copies them for you."
