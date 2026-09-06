# Plan: Embedded local WebUI + signalling server (replaces mirashare for local viewing)

## Goal

One self-contained feature: the sharer app starts a small embedded axum server that (a) serves a local viewer page and (b) implements the mirashare signalling protocol, so the whole view path works with **no third-party server and no separate repo**. The existing mirashare path stays available via config.

## Architecture

```
Browser ──HTTP GET /──────────────► axum server (in-process, 127.0.0.1:8765)
   │  ws /signaller (viewer: join/answer/ice/leave)
   └──────────────────────────────► axum server
Sharer (same process) ──ws /signaller (sharer: start/offer/ice/leave)──► axum server
Sharer ◄────WebRTC media (SRTP, P2P, loopback or LAN)────► Browser
```

The sharer's existing `WebSocketSignaller` is already a client of exactly this protocol — **no changes** to WebRTC, capture, encoder, or auth code. The axum server is a room router between the two WS connections.

## Changes

### 1. New dependency — `Cargo.toml`

- `axum = { version = "0.7", features = ["ws"] }`

### 2. New module — `src/webui/mod.rs` (~250–300 lines)

- `pub async fn start(port: u16) -> std::io::Result<()>`: binds `127.0.0.1:<port>`, serves the router, runs until process exit.
- Routes:
  - `GET /` → embedded `include_str!("../../webui/index.html")` (works in packaged `.exe`/`.app`, no runtime file dependency).
  - `GET /signaller` → WebSocket upgrade.
- Signalling state: `Arc<SignallerState>` holding
  - `rooms: Mutex<HashMap<room_id, SharerConn>>` (sharer WS sender)
  - `viewers: Mutex<HashMap<viewer_uuid, (ViewerConn, room_id)>>`
- Per-connection message handling (JSON, `type`-tagged, snake_case — must match the sharer's `SignallerMessage` serde names exactly):
  - `start` → generate room UUID (uuid crate), register sharer, reply `start_response { room }`
  - `join { room, from, name, auth }` → relay **as-is** to that room's sharer (sharer ignores the extra `room` field); register viewer. Unknown room / no active session → send `join_declined { reason: 0, to: <viewer> }` directly
  - `offer` / `ice` with `to` = viewer UUID → forward to that viewer
  - `answer` / `ice` with `to` = room ID → forward to that room's sharer
  - `leave` from sharer → send `room_closed { to, room }` to each viewer, drop room + viewers; `leave` from viewer → relay `leave { from }` to sharer, drop viewer
  - `join_declined` / `room_closed` from sharer (kick) → forward to addressed viewer, drop viewer registration
  - `ice_servers` → reply `ice_servers_response { ice_servers: [] }` (sharer then falls back to its configured STUN server; host candidates make LAN work with no STUN at all)
  - `keep_alive` → update last-activity; a 10s ticker drops connections idle > 90s
- Routing ambiguity note: rooms and viewer UUIDs are both random v4 — check `viewers` map before `rooms` map; collision risk negligible.
- Port bind failure → `error!` log, app continues without webui (fall back to `config.signaller_url`).

### 3. Config — `src/config.rs`

```toml
[webui]
enabled = true
port = 8765
```

- New `WebuiConfig { enabled: bool, port: u16 }` with `#[serde(default)]` on both the field and members (existing `config.toml` files keep working).

### 4. Wiring — `src/main.rs`, `src/gui/app.rs`, `src/capture/capturer.rs`

- `main.rs`: parse `Args` and load `Config` here (moved out of `App::new`); if `config.webui.enabled`, `tokio::spawn(webui::start(port))` before `App::run()`; pass `(args, config)` as iced `Flags`. (Same runtime already backs iced's executor — the app already `tokio::spawn`s from `update`.)
- `App::new`: takes flags instead of re-parsing/re-loading.
- `Capturer::capture`: signaller URL selection —
  `if config.webui.enabled { "ws://127.0.0.1:<port>/signaller" } else { config.signaller_url }`
- `Capturer::get_invite_link`: when webui enabled → `http://127.0.0.1:<port>/?room=..&pwd=..` (the page derives its WS URL from its own origin, so no `signaller` param needed). The sharing page's existing "Invite Link" card then just shows the local URL — **no other GUI changes**.

### 5. New file — `webui/index.html` (~250 lines, vanilla JS, no CDN, no build step)

- Config from query params `room`, `pwd`; signaller defaults to `ws(s)://<location.host>/signaller` (overridable via `signaller` param). Missing room/pwd → small join form.
- Flow: `WebSocket` open → send `join { room, from: crypto.randomUUID(), name: "WebUI", auth: { type: "password", password } }` → keepalive every 30s.
- On `offer`: `RTCPeerConnection`, `setConfiguration({ iceServers })` (map `credential_type` → `credentialType`, omit when empty/`unspecified`), `setRemoteDescription` → `createAnswer` → send `answer { sdp, from, to: room }`.
- ICE both directions; **queue** inbound ICE candidates that arrive before the remote description is set, flush after (the public viewer just drops these — we don't).
- `ontrack`: video → `<video autoplay playsinline>`, audio → `<audio autoplay>` with a "click to enable audio" fallback for autoplay policies.
- Status UI: *Connecting → Waiting for approval → Waiting for stream → Live*; errors: `join_declined` (map reason 0/1/2/3 to text), `room_closed` → "Session ended", WS drop → error + reconnect button; `leave` sent on page close.
- View-only (no mouse/keyboard forwarding). Minimal dark CSS, letterboxed full-window video.

### 6. Docs — `README.md`

Short "Local WebUI" section: what it does, config keys, how to use (start sharing → open the invite link or `http://127.0.0.1:8765/` → accept the pending viewer), and that `webui.enabled = false` restores the mirashare flow.

## Out of scope (possible follow-ups)

- "Open local viewer" button in the iced GUI (needs an `open`-crate or platform spawn).
- Remote/LAN exposure beyond loopback, TLS, multiple sharers, viewer-side remote control.

## Verification

1. `cargo check` + `cargo fmt` (per AGENTS.md), plus LSP diagnostics on changed files.
2. E2E: `cargo run --release` → start sharing → open `http://127.0.0.1:8765/` → join with room + passcode from the GUI → accept the pending viewer in the GUI → **live video with audio**.
3. Negative cases: wrong passcode → "incorrect passcode" on page; decline in GUI → "declined by host"; stop sharing while viewing → "Session ended"; join with no active session → declined message.
4. Regression: `webui.enabled = false` → app uses `config.signaller_url` (mirashare) exactly as before; sharer-side protocol untouched, so the official mirashare viewer still works.
5. Port already in use → error logged, app runs normally without webui.

## Assumptions

- Default port `8765`, bound to `127.0.0.1` only (no firewall exposure, loopback signalling).
- Empty `ice_servers_response` is acceptable (STUN fallback + host candidates suffice for LAN).
- One sharer session at a time (already true in the app), so one room per sharer connection is fine.
- axum 0.7 (its internal tokio-tungstenite coexists with the existing 0.19 client dep).
