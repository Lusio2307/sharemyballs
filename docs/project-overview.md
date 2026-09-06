# Project Overview

**sharemyballs** is the sharer client of [Mira Screenshare](https://github.com/mira-screen-share/sharer): a high-performance screen-sharing and remote-control desktop application written entirely in Rust (package name: `mira_sharer`).

## Tech Stack

| Area | Technology |
|---|---|
| Language | Rust (edition 2021) |
| Async runtime | Tokio (full) |
| Networking / streaming | WebRTC (`webrtc` crate), SRTP-encrypted P2P with TURN fallback |
| Signalling | WebSocket client (`tokio-tungstenite`) to an external signalling server |
| Screen capture | Windows: `Windows.Graphics.Capture` (wgc); macOS: `ScreenCaptureKit` (via `apple-sys`/`objc` FFI); Linux: `xcap` |
| Audio capture | `cpal` (system audio) |
| Encoding | FFmpeg via `ac-ffmpeg` (default codec: x264), GPU YUV conversion via wgpu shaders |
| Remote input (mouse/keyboard) | `enigo` (fork) |
| GUI / "frontend" | **iced** 0.9 (Rust immediate-mode GUI, rendered with wgpu) + `iced_aw` widgets — no web frontend in this repo |
| Config | TOML (`config.toml`, presets in `configs/`) |
| CLI | `clap` |
| Logging | `log` + `fern` |
| Packaging | `cargo bundle` metadata + `embed-resource` for Windows `.exe` manifest/icons |

Note: the web **viewer** (mirashare.app) and the **signalling server** are separate repositories; this repo is only the sharer.

## Project Organization

```
src/
├── main.rs          # Entry point: logging setup, iced app launch
├── config.rs        # TOML configuration loading
├── capture/         # Screen + audio capture, platform-specific
│   ├── display/     #   display/window capture
│   ├── audio/       #   system audio capture (cpal)
│   ├── wgc/         #   Windows Graphics Capture backend
│   ├── macos/       #   ScreenCaptureKit backend (FFI)
│   ├── linux/       #   xcap backend
│   └── yuv_convert/ #   CPU/GPU color conversion (+ wgpu shaders)
├── encoder/         # FFmpeg encoder wrapper + frame pool
├── output/          # Output sinks: webrtc_output / webrtc_peer (media pipeline),
│                    #   file_output (recording), noop_output
├── signaller/       # WebSocket signalling client (peer discovery, negotiation)
├── inputs/          # Remote keyboard/mouse input handling (enigo, key parsing)
├── auth/            # Session/authentication logic
├── gui/             # iced-based GUI
│   ├── app.rs       #   main iced `Application`
│   ├── component/   #   screens: start screen, sharing screen, avatars
│   ├── theme/       #   custom theme (buttons, tabs, text, colors, …)
│   └── resource.rs  #   embedded assets
├── performance_profiler.rs
└── result.rs        # shared error type
configs/             # preset config.toml files
resources/           # icons, logo, screenshot
build.rs             # embeds Windows resource manifest
```

## Data Flow

1. `capture/` grabs screen + audio frames per platform.
2. `encoder/` encodes frames with FFmpeg (x264 by default).
3. `output/webrtc_*` pushes media into WebRTC `RTCPeerConnection`s.
4. `signaller/` (WebSocket) coordinates peer discovery and SDP negotiation with the signalling server.
5. `inputs/` applies remote keyboard/mouse events received from the viewer.

## Build & Run

- Requires FFmpeg (see `README.md` for platform-specific setup).
- `cargo run --release` (or the `Makefile` targets).
- Config: `config.toml` next to the binary (macOS: `~/Library/Application Support/Mira-Sharer/config.toml`).

## License

GPL-3.0-or-later (files under `src/capture/macos` are additionally MIT-licensed).
