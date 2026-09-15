

<!-- PROJECT LOGO -->
<br />
<div align="center">
  <a href="https://github.com/mira-screen-share/sharer">
    <img src="resources/logo.png" alt="Logo">
  </a>

  <h3 align="center">Mira Screenshare</h3>

  <p align="center">
    A high-performance screen-sharing / remote collaboration software written in Rust.
  </p>
</div>

<div align="center">

[![Rust][rust-shield]][github-url]
[![Stargazers][stars-shield]][stars-url]
[![MIT License][license-shield]][license-url]

</div>

## Introduction


<div align="center">

[![Screenshot]][github-url]

</div>

Currently this project is completely free and open-source. We have hosted a TURN server and a signalling server for public use, though we cannot make any guarantees about their availability or performance.
The viewer client is available at [mirashare.app][viewer-page].

You may also choose to host your own viewer and signalling server so everything is under your control.

At this stage, we do not recommend using it for any sensitive or mission-critical applications. Contributions are welcome.

## Downloads

Pre-compiled binaries for macOS (aarch64 / x86-64) and Windows are available for download at the [releases page][release-url].

## Features

* High performance screen capturing and streaming
* Remote mouse and keyboard control
* System audio capturing
* Cross-platform (macOS, Windows)
* Concurrent viewers support

## Performance
* 60 FPS encoding at 4K resolution
* 110 ms E2E latency

## Technical Details

Mira is built on top of [the WebRTC stack][webrtc], and consists of three parts, namely the sharer
client, the viewer client, and the signalling server.

* The [sharer client][github-url] will be responsible for capturing
and streaming the screen directly to the viewer(s) through a P2P connection.
    - if your Internet environment does not permit such a P2P connection to be established, a TURN server is required to relay the data.
* The [viewer][viewer-url] can dispatch
input (e.g. keyboard, mouse events) to the sharer to achieve control of the sharer's operating system.
* The [signalling server][signaller-url] is reponsible for peer discovery and initial connection negotiations.

For screen capturing, we use `Windows.Graphics.Capture` on Windows, and `ScreenCaptureKit` on macOS. This requires at least Windows 10 v1803 and macOS 13.0.

For encoding, by default x264 is used, however you can use other codecs/encoder or adjust its settings (quality, speed, compression, etc.) in the configuration file, `config.toml` (`~/Library/Application Support/Mira-Sharer/config.toml` for macOS).

## Q & A
Q. Can you see my screens?

A. No, we cannot see your screens. In an ideal environment, the data is transmitted directly from the sharer to the viewer(s), and the signaller is only used for initial connection negotiation. We do not have access to the data.
Even when a direct connection cannot be established, the data is relayed through a TURN server, and is encrypted with the WebRTC stack (e.g. Secure Real Time Protocol (SRTP)).

However, if you are still concerned, you could host your own signalling server and TURN server, and use the viewer client from the source code.

Q. Do you collect any data from me?

A. The signalling server we host does collect some metrics such as the number/length of sessions and the number of unique users estimated through your hashed IP address (salted and hashed with argon2). However, we do not collect any personal data. We do not have access to the data transmitted between the sharer and the viewer(s).

## Build

`ac-ffmpeg` supports **FFmpeg v4-v7 only**, so an FFmpeg 8/9 install will not
compile. On Windows two scripts handle everything; see
[docs/build-windows.md](docs/build-windows.md) for the detail.

* For macOS, you could use `brew install ffmpeg@5` (later versions will not compile).
  * You will also need to `cargo install apple-bindgen` and run `apple-bindgen CoreFoundation --sdk macosx`
* For Linux, FFmpeg is discovered through pkg-config, so the distro's
  `libavcodec-dev` and friends are enough.
* For Windows:
  ```powershell
  pwsh -File scripts/fetch-ffmpeg.ps1    # once: pinned FFmpeg 7.1.1 into third_party/ffmpeg
  pwsh -File scripts/build-windows.ps1   # imports vcvars, sets FFMPEG_*, builds, copies DLLs
  ```

Then, simply run `cargo run --release` (or the built
`target\release\mira_sharer.exe --config config.toml` on Windows).

## Configure
Configuration file is by default `config.toml`; copy
[config.toml.example](config.toml.example) to start, and see the preset configs
in `configs/` for encoder-specific settings. Every key is optional, and the
defaults contact no third-party service.

For macOS, the configuration file is located at `~/Library/Application Support/Mira-Sharer/config.toml`.

Key settings for unattended operation:

```toml
room = "desk"          # fixed room => stable invite URL
password = "…"         # fixed password (default: random per session)
auto_accept = true     # admit viewers without a GUI click (password still checked)
auto_start = true      # begin sharing on launch
```

Deployment (TLS via Caddy, coturn for off-LAN viewers, firewall ports) is
covered in [docs/deployment.md](docs/deployment.md).

## Local WebUI

By default, the sharer starts an embedded local web server (loopback only, no third-party server needed) that serves a built-in viewer page and acts as the signalling server for it. Media still flows P2P via WebRTC between the sharer and the browser — the local server only handles signalling.

```toml
[webui]
enabled = true   # set to false to use the external (mirashare) signaller/viewer
port = 8765
bind = "127.0.0.1"   # use "0.0.0.0" to expose the page to the LAN
# public_url = "https://stream.example.com/"   # base URL for the invite link
```

To use it: start sharing, then open the **Invite Link** shown on the sharing page (or just `http://127.0.0.1:8765/` and enter the room id and passcode), and accept the pending viewer in the app — or set `auto_accept = true` to skip that step. Setting `webui.enabled = false` restores the original mirashare flow. If the port is already in use, the error is logged and the app continues with the configured `signaller_url`.

## License

GPLv3

Note that files under `src/capture/macos` are also dual-licensed under MIT.

## Attributions
* Some code is adapted from [scrap](https://github.com/quadrupleslap/scrap), which is licensed under the MIT license.
* Some code from [MirrorX](https://github.com/MirrorX-Desktop/MirrorX), licensed under GPLv3.

[release-url]: https://github.com/mira-screen-share/sharer/releases
[webrtc]: https://webrtc.org/
[screenshot]: resources/screenshot.png
[github-url]: https://github.com/mira-screen-share/sharer
[viewer-url]: https://github.com/mira-screen-share/viewer
[signaller-url]: https://github.com/mira-screen-share/signaller
[viewer-page]: https://mirashare.app/
[rust-shield]: https://img.shields.io/badge/Lang-Rust-EF7B3C?style=for-the-badge
[stars-shield]: https://img.shields.io/github/stars/mira-screen-share/sharer?style=for-the-badge
[stars-url]: https://github.com/mira-screen-share/sharer/stargazers
[license-shield]: https://img.shields.io/github/license/mira-screen-share/sharer.svg?style=for-the-badge
[license-url]: https://github.com/mira-screen-share/sharer/blob/master/LICENSE.txt
