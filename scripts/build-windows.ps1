# Build mira_sharer for Windows.
#
#     pwsh -File scripts/build-windows.ps1
#
# This exists instead of a plain `cargo build` for two reasons:
#
#  * ac-ffmpeg needs FFMPEG_INCLUDE_DIR / FFMPEG_LIB_DIR pointing at the
#    third_party/ffmpeg shared build. Cargo's [env] section cannot be scoped to
#    a target, and a plain value under [target.<triple>.env] is not forced, so a
#    stale machine-wide FFMPEG_* variable would win.
#  * build.rs runs embed-resource, which needs rc.exe, so the MSVC developer
#    environment must be imported first.
#
# Run scripts/fetch-ffmpeg.ps1 once beforehand.

$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path -Parent $PSScriptRoot
$Ffmpeg = Join-Path $RepoRoot 'third_party\ffmpeg'

foreach ($p in @(
        (Join-Path $Ffmpeg 'include\libavcodec\avcodec.h'),
        (Join-Path $Ffmpeg 'lib\avcodec.lib')
    )) {
    if (-not (Test-Path $p)) {
        throw "Missing $p -- run scripts/fetch-ffmpeg.ps1 first."
    }
}

# Import the MSVC environment (rc.exe for the resource manifest, link.exe).
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vcvars = $null
if (Test-Path $vswhere) {
    $vsPath = & $vswhere -latest -products * `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath
    if ($vsPath) { $vcvars = Join-Path $vsPath 'VC\Auxiliary\Build\vcvars64.bat' }
}
if (-not $vcvars -or -not (Test-Path $vcvars)) {
    throw 'Could not locate vcvars64.bat. Install the Visual Studio C++ build tools.'
}

Write-Host "vcvars64 = $vcvars"
cmd /c "`"$vcvars`" >nul 2>&1 && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') {
        try { Set-Item -Path "env:$($matches[1])" -Value $matches[2] -ErrorAction Stop } catch { }
    }
}

# Set these *after* importing vcvars: the imported environment still carries the
# machine-wide FFMPEG_* values, and they would otherwise overwrite these.
$env:FFMPEG_INCLUDE_DIR = Join-Path $Ffmpeg 'include'
$env:FFMPEG_LIB_DIR = Join-Path $Ffmpeg 'lib'
Write-Host "FFMPEG_INCLUDE_DIR = $env:FFMPEG_INCLUDE_DIR"
Write-Host "FFMPEG_LIB_DIR     = $env:FFMPEG_LIB_DIR"

Set-Location $RepoRoot
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }

# FFmpeg is linked dynamically and loaded from the executable's directory.
Copy-Item (Join-Path $Ffmpeg 'bin\*.dll') $RepoRoot -Force

$exe = Join-Path $RepoRoot 'target\release\mira_sharer.exe'
if (-not (Test-Path $exe)) { throw "Build reported success but $exe is missing." }

Write-Host ''
Write-Host "OK: $exe"
Write-Host 'Run it with:  .\target\release\mira_sharer.exe --config config.toml'
