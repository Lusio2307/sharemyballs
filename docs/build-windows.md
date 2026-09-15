# Building on Windows

The app links FFmpeg, and `ac-ffmpeg` supports **FFmpeg v4-v7 only**. Do not
point it at the FFmpeg 8/9 builds that winget or the BtbN "latest" release now
ship -- it will not compile. `scripts/fetch-ffmpeg.ps1` pins a known-good
FFmpeg 7.1.1 shared build.

## Prerequisites

- Rust with the `x86_64-pc-windows-msvc` target (`rustup target list --installed`).
- Visual Studio Build Tools with the C++ workload, plus a Windows SDK.
  `embed-resource` in `build.rs` shells out to `rc.exe`, so the build must run
  from a developer environment (see below), not a plain shell.
- FFmpeg 7.1.x shared build in `third_party/ffmpeg`.

## 1. Fetch FFmpeg

From the repository root:

```powershell
pwsh -File scripts/fetch-ffmpeg.ps1
```

This produces `third_party/ffmpeg/{include,lib,bin}`. The script fails loudly if
the archive turns out not to contain real headers and import libraries, because
a directory named `include` full of DLLs is a failure mode this setup has hit
before.

`.cargo/config.toml` points `FFMPEG_INCLUDE_DIR` and `FFMPEG_LIB_DIR` at those
directories with `force = true`, so stale machine-wide `FFMPEG_*` variables
(including one pointing at a `lib/x64` that does not exist) cannot take
precedence.

## 2. Build

Run inside a developer environment so `rc.exe` and `link.exe` are on `PATH`:

```powershell
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo build --release'
```

Adjust the `vcvars64.bat` path for your installation. A plain PowerShell session
usually is *not* enough: the Rust MSVC target can find the linker itself, but
`embed-resource` needs `rc.exe` on `PATH`.

## 3. Copy the runtime DLLs

```powershell
Copy-Item third_party\ffmpeg\bin\*.dll .
```

The FFmpeg DLLs are loaded at runtime from the executable's directory, and
`third_party/` is git-ignored, so they are not shipped with the repository.

Then run:

```powershell
.\mira_sharer.exe --config config.toml
```

Copy `config.toml.example` to `config.toml` first and edit it. With
`[webui] enabled` (the default) the app prints the viewer page URL and the
invite link appears in the GUI's Invite tab.

## Verifying the build actually links FFmpeg

`cargo check` does not link, so it will pass even if the import libraries are
wrong. A successful `cargo build --release` plus a launch that logs
`WebRTC initialized` (rather than failing at encoder creation) is the real
signal.
