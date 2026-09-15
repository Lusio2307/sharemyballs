# Building on Windows

Two scripts do the whole job:

```powershell
pwsh -File scripts/fetch-ffmpeg.ps1     # once
pwsh -File scripts/build-windows.ps1    # each build
```

The app links FFmpeg, and `ac-ffmpeg` supports **FFmpeg v4-v7 only**. Do not
point it at the FFmpeg 8/9 builds that winget or the BtbN "latest" release now
ship — it will not compile. `fetch-ffmpeg.ps1` pins a known-good FFmpeg 7.1.1
shared build.

## Prerequisites

- Rust with the `x86_64-pc-windows-msvc` target (`rustup target list --installed`).
- Visual Studio Build Tools with the C++ workload, plus a Windows SDK.
- `pwsh` (PowerShell 7) or `powershell.exe` (5.1) — both work.

## 1. Fetch FFmpeg

```powershell
pwsh -File scripts/fetch-ffmpeg.ps1
```

This populates `third_party/ffmpeg/{include,lib,bin}` and fails loudly if the
archive turns out not to contain real headers and import libraries — a directory
named `include` full of DLLs is a failure mode this project has hit before.

## 2. Build

```powershell
pwsh -File scripts/build-windows.ps1
```

The script, in order:

1. Verifies `third_party/ffmpeg` really has headers and import libraries.
2. Locates and imports the MSVC developer environment via `vswhere` +
   `vcvars64.bat`, so `rc.exe` (used by `build.rs` through `embed-resource`) and
   `link.exe` are on `PATH`.
3. Sets `FFMPEG_INCLUDE_DIR` and `FFMPEG_LIB_DIR` to absolute paths inside the
   repo, **after** importing vcvars.
4. Runs `cargo build --release`.
5. Copies `third_party/ffmpeg/bin/*.dll` next to the executable.

### Why not just `cargo build`?

Two things break a bare `cargo build`:

- Cargo's `[env]` section cannot be scoped to a target, and a plain value under
  `[target.<triple>.env]` is **not** forced, so it loses to an already-set
  environment variable. A stale machine-wide `FFMPEG_LIB_DIR` — for example one
  pointing at a `lib/x64` that does not exist — would be used instead, and the
  build would fail to find the import libraries. The extended
  `{ value = ..., relative = true, force = true }` form is rejected outright
  there.
- The MSVC environment is not imported in a plain shell, so `rc.exe` is missing.

Step 3 happens after step 2 deliberately: the imported vcvars environment still
carries the machine-wide `FFMPEG_*` values and would otherwise overwrite them.

## 3. Run

Copy `config.toml.example` to `config.toml` and edit it, then:

```powershell
.\target\release\mira_sharer.exe --config config.toml
```

With `[webui] enabled` (the default) the app logs the viewer page URL and the
invite link appears in the GUI's Invite tab. See
[deployment.md](deployment.md) for TLS, coturn and unattended operation.

## Verifying the build really linked FFmpeg

`cargo check` does not link, so it passes even when the import libraries are
wrong. A successful `cargo build --release` plus a launch that gets as far as
`WebRTC initialized` (rather than failing at encoder creation) is the real
signal.
