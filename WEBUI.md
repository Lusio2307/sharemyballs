# WebUI page to view the live stream

## Summary

Yes, this is easy: the app already streams H.264/Opus over WebRTC to *any* viewer that joins its room via the signaller WebSocket — the hosted viewer (mirashare.app) is exactly such a page. So the feature is a **self-contained, view-only viewer page** added to this repo as `webui/index.html`: vanilla JS + inline CSS, no dependencies, no build step, **no Rust code changes**. It implements the viewer side of the existing signalling protocol (verified against the open-source viewer's `viewmodel.ts` and this repo's `src/signaller/mod.rs`).

## Changes

### 1. New file: `webui/index.html` (single self-contained file)

**Config & entry**
- Read config from URL query params: `room`, `pwd`, `signaller` (default `wss://ws.mirashare.app`, same as `src/config.rs`), optional `name` (default `"WebUI"`).
- If `room` or `pwd` is missing, show a small join form (room, passcode, signaller URL) instead of connecting.
- Works from `file://` (no CORS/mixed-content issues: only outbound `wss://` + WebRTC).

**Signalling flow (JSON over WebSocket, `type`-tagged snake_case — matches `SignallerMessage`)**
- Generate `uuid = crypto.randomUUID()`; open `WebSocket(signaller)`.
- On open: send `{type:"join", room, from:uuid, name, auth:{type:"password", password:pwd}}`; start 30s interval sending `{type:"keep_alive"}`.
- Status bar states: `Connecting…` → `Waiting for approval…` (join sent) → `Waiting for stream…` (offer received) → `Live`.
- Ignore any incoming message whose `uuid` equals our own (echo suppression, as the official viewer does).
- On page close/unload: send `{type:"leave", from:uuid}`, close peer connection and socket.

**WebRTC handshake**
- On `offer` (`{sdp, from, to, ice_servers}`): create `RTCPeerConnection`; `setConfiguration({iceServers: …})` mapping `ice_servers` entries `{urls, username, credential, credential_type}` to browser format (omit `credentialType` when it is `unspecified`, since browsers only accept `password`/`oauth`); `setRemoteDescription(sdp)` → `createAnswer()` → `setLocalDescription(answer)` → send `{type:"answer", sdp, from:uuid, to:room}`.
- ICE: on `pc.onicecandidate` send `{type:"ice", ice:candidate, from:uuid, to:room}`; on incoming `ice` messages call `pc.addIceCandidate(ice)`. **Queue candidates that arrive before the remote description is set and flush them after** (the sharer can send ICE before the page finishes the offer dance).

**Media rendering**
- `pc.ontrack`: video track → `<video autoplay playsinline>` (fills window); audio track → hidden `<audio>` element; `audio.play().catch(…)` → show a "tap to enable audio" button (autoplay policy).
- `pc.onconnectionstatechange`: surface `disconnected`/`failed` in the status bar.

**Error / lifecycle messages**
- `join_declined` → show reason (0 unknown, 1 incorrect password, 2 no credentials, 3 user declined) with the number mapped to text.
- `room_closed` → "Session ended" (sharer stopped or kicked).
- WebSocket error/close before join → error state with a Reconnect button.

**Out of scope (explicit):** remote mouse/keyboard control from the page (view-only; the sharer's `control` data channel simply goes unused), embedded HTTP server, GUI button, separate repo.

### 2. `README.md`

Add a short "Local web viewer" section: copy Room + Passcode from the sharer GUI's Invite tab, open `webui/index.html` in a browser (optionally with `?room=…&pwd=…&signaller=…`), then accept the pending viewer on the Viewers tab.

## Behavior notes / edge cases

- The sharer requires the sharer-side user to **approve the pending viewer** in the GUI before the stream starts — the page must sit in "Waiting for approval…" until the `offer` arrives (no timeout; approval can take arbitrarily long).
- No Rust changes, so the existing mirashare.app viewer and all current behavior are untouched.
- Page is view-only; it never sends input events.

## Verification

1. `cargo check` && `cargo fmt` (per AGENTS.md; expected clean since no Rust changes).
2. Manual happy path: `cargo run --release` → start sharing → open `webui/index.html?room=<room>&pwd=<passcode>` → accept the pending viewer in the GUI → page shows live video + audio, status `Live`.
3. Wrong passcode: page shows "incorrect password" (sharer auto-rejects).
4. Decline in GUI: page shows "user declined".
5. Stop sharing in GUI: page shows "Session ended".
6. No params: opening `webui/index.html` bare shows the join form; joining from the form works.
7. Regression: mirashare.app invite link still works as before (no protocol changes).

## Assumptions

- Default signaller is the public `wss://ws.mirashare.app`; self-hosted signallers work via the `signaller` param (config file stays the source of truth for the app itself).
- The public signaller routes a viewer's `join` (which carries `room`) to the sharer in that room — same mechanism the open-source viewer relies on.
- "For now" scope is a manually-opened static page; wiring (embedded HTTP server / GUI button) is a possible follow-up but excluded here.
