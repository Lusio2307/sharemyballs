# Task: wire the live WebRTC stream into the SPA viewer

You are working in the `mira_sharer` repo (screen-sharing / remote-control app, Rust + WebRTC). Your task: **wire the real live stream into the web UI's viewer page.** Work autonomously, but read this whole brief first — it contains the protocol and the traps.

## 1. Current state

- The app captures → encodes (ffmpeg) → streams P2P over WebRTC. It embeds its own axum server (`src/webui/`) that serves the web UI and implements the signalling protocol, so no third-party server is involved. Config lives under `[webui]` in `config.toml` (`enabled`, `port`, `bind`, `public_url`, `admin_password`).
- The web UI is a **client-only React 19 + Vite 8 + TanStack Router + Chakra UI SPA** at `webui/sharemyballs-webui/`, built with **bun** into a **single self-contained `dist/index.html`**. `src/webui/mod.rs` inlines that file with `include_str!`, serves it at `GET /`, and serves the same document at `/admin`.
- `/admin` is already implemented and tested (a control plane for start/stop sharing and accept/decline/kick viewers, Bearer-authenticated). **Do not break it.**
- `src/routes/index.tsx` currently renders only a `<canvas>` stub plus a fullscreen button. **The real viewer is not ported yet.** It used to be a hand-written 297-line vanilla page, `webui/index.html`, which was deleted in this repo's history and worked end-to-end on Windows/Chrome.

## 2. Goal and success criteria

Port the viewer onto the SPA so a browser can join a room, get approved, and watch the live stream. Decisions already made: **`<video>` is the media surface** (not the canvas — the browser's hardware-decoded path wins) and **scope is view-only**.

1. Opening the invitation link `http://<host>:8765/?room=<room>&pwd=<pass>` joins the room, waits for host approval, then plays live video **and** audio.
2. Status reflects the real state machine, and all four failure paths behave: decline reasons, room closed, socket loss (offer Reconnect), WebRTC failure.
3. Fullscreen works on the video stage; Esc exits; mobile stays usable (`playsinline`).
4. Autoplay policy is handled (a tap-to-enable-audio affordance; never a silent failure).
5. `/admin` keeps working and the Rust test suite stays green.
6. The build still produces exactly one `dist/index.html` with no sibling `assets/`.

## 3. Hard constraints — breaking these breaks the product

- **Single-file build.** `vite-plugin-singlefile` inlines everything and the result is served as one HTML response. Never add dynamic `import()`, `React.lazy()`, or manual chunks: split chunks cannot be inlined and would be requested from a server that does not serve them.
- **`class="dark"` on `<html>` in `index.html` is load-bearing.** It is what puts Chakra's semantic tokens (`fg`, `bg`, `border`) into their dark values; without it every token falls back to light and text renders near-black on the black background. A Rust test asserts it.
- **The embedded server has no SPA fallback.** Only `/`, `/admin`, and `/signaller` exist (`src/webui/mod.rs`). Keep the viewer at `/`; a new route needs a new Rust route serving the same document.
- **The invite-URL contract is fixed by the Rust side.** `Capturer::get_invite_link()` emits `{public_url or http://127.0.0.1:<port>/}?room=<room>&pwd=<password>`. Read `room`, `pwd`, and the optional `signaller` override from the query string; do not rename them.
- **Derive the signaller URL from the page's own origin** (`ws://` or `wss://` + `/signaller` based on `location.protocol`). That is exactly what makes the `public_url`/Caddy path work (Caddy proxies `stream.example.com` → `127.0.0.1:8765` and passes the WS upgrade through).
- **The sandbox cannot run `vite build`** (Vite spawns `net use` with piped stdio → `EPERM`). Run `bun run typecheck` and `bun run lint` yourself; **ask the human to run `bun run build`** and then run the Rust checks.
- **`dist/` is currently stale** (built before the dark-class fix), so `cargo test` fails today on `webui::tests::signaller_relay_roundtrip` with "served a page without `class=\"dark\"`". That is expected. Ask the human to rebuild; do not edit that test to make it pass.
- `AGENTS.md` requires `cargo check` + `cargo fmt` after code edits. Because the page is embedded, a frontend change needs a rebuild **and** an app restart — reloading the tab is not enough.

## 4. The signalling protocol (authoritative)

Server = the embedded axum router; sharer = the Rust app (a WS client of its own server); viewer = the browser. Routing lives in `src/webui/mod.rs::route`; message types in `src/signaller/mod.rs`.

Connect to `ws(s)://<same origin>/signaller`. Every message is JSON tagged by `type` (snake_case):

| Direction | Message |
| --- | --- |
| viewer → server | `{"type":"join","room":<room>,"from":<viewerUuid>,"name":"WebUI","auth":{"type":"password","password":<pwd>}}` |
| viewer → server | `{"type":"keep_alive"}` every 30 s — the server reaps connections idle for >90 s (`IDLE_TIMEOUT`) |
| sharer → viewer | `{"type":"offer","sdp":{…},"from":<room>,"to":<viewerUuid>,"ice_servers":[…]}` |
| viewer → server | `{"type":"answer","sdp":<localDescription>,"from":<viewerUuid>,"to":<room>}` |
| viewer → server | `{"type":"ice","ice":<candidate.toJSON()>,"from":<viewerUuid>,"to":<room>}` |
| sharer → viewer | `{"type":"ice","ice":…,"from":<room>,"to":<viewerUuid>}` |
| sharer → viewer | `{"type":"join_declined","reason":<0–3>,"to":<viewerUuid>}` — 0 unknown, 1 incorrect password, 2 no credentials, 3 declined by host |
| sharer → viewer | `{"type":"room_closed","to":<viewerUuid>,"room":<room>}` |
| viewer → server | `{"type":"leave","from":<viewerUuid>}` on unload |

Notes that matter:

- The viewer's `to` for `answer`/`ice` is the **room id**; its `from` is its own uuid. The router resolves `ice` by checking registered viewers first, then rooms.
- `ice_servers_response` (always `{"ice_servers":[]}` from the embedded router) is only sent if you ask with `{"type":"ice_servers"}`. You don't need to: the offer already carries `ice_servers`, and empty means the sharer falls back to its configured STUN plus host candidates.
- The sharer adds a **video** track, an **audio** track, and opens an incoming **`control` data channel** (`src/output/webrtc_peer.rs:39-84`). Ignore the data channel — that is the remote-input path and it is out of scope. Ignoring it is safe.
- `pc.ontrack` fires for video and audio separately; attach them independently.

## 5. Porting checklist — every item below was a real bug that cost hours

Taken from `WEBUI.md`:

- **Read the kind from `ev.track.kind`.** `RTCTrackEvent` has no `kind` of its own, so branching on `ev.kind` leaves both `pc.ontrack` branches unreachable: `video.srcObject` is never set and the viewer sees a black stage while frames decode perfectly. Only reveal the stage for a *video* track.
- Fall back to `new MediaStream([ev.track])` when the SDP carries no msid.
- **Queue ICE candidates that arrive before `setRemoteDescription`** and flush them afterwards; the sharer can send ICE before the offer dance finishes.
- When mapping the sharer's ICE servers, **omit `credentialType`** for empty/`Unspecified` and only ever pass `password`/`oauth`: `IceCredentialType` also serializes `Twilio`/`Signaller`, which the browser rejects with a `TypeError`.
- Generate the viewer uuid **without `crypto.randomUUID()`** — it is secure-context-only, and a LAN viewer on `http://192.168.x.x:8765/` would throw before connecting. Use `getRandomValues` with a `Math.random` fallback.
- Distinguish **terminal** failures (`join_declined`, `room_closed` — no reconnect) from **recoverable** ones (socket loss — offer Reconnect).
- Send `{type:"leave"}` and close the socket and peer connection on **`beforeunload`**: a React effect cleanup does not fire when the tab closes.
- Handle the autoplay policy: a "click to enable audio" affordance and `playsinline` on the video.

## 6. Reference implementations to read first

- `deploy/external/viewer/src/components/App/viewmodel.ts` — the upstream React viewer (a local, gitignored clone). Shows the join/leave flow, offer handling, and fullscreen.
- `git show` the deleted `webui/index.html` from history — the vanilla viewer that demonstrably worked (297 lines). Its structure maps directly onto the hooks/components you will write.
- `docs/plan-local-webui.md` and `docs/plan-webui-followup.md` — how the embedded server and viewer were designed, and the two independent bugs that produced the "black screen".
- `WEBUI.md` — build constraints, dev workflow, admin plane, and the checklist above.

## 7. Suggested implementation shape (adapt as you see fit)

```
src/lib/uuid.ts               # secure-context-safe id
src/lib/protocol.ts           # hand-written TS mirror of the signaller messages, commented with a pointer to src/signaller/mod.rs
src/lib/ice.ts                # toRTCIceServers() implementing the credentialType rule
src/hooks/useViewerSession.ts # socket + RTCPeerConnection state machine
src/routes/index.tsx          # join form | stage; <video autoplay playsinline> (+ <audio> if needed) + fullscreen control
```

- State machine: `idle → connecting → awaitingApproval → awaitingStream → live`, plus `failed (terminal)` and `lost (recoverable)`.
- Keep the existing fullscreen behaviour (request fullscreen on a container so the control stays reachable; Esc exits) and reuse the pattern already in `src/routes/index.tsx`.
- Add the dev proxy to `vite.config.ts` so `bun run dev` on `:3000` reaches a running sharer: `server.proxy['/signaller'] = { target: 'ws://127.0.0.1:8765', ws: true }`, and the same under `preview`.
- Use Chakra components and the `#/` import alias as `src/routes/admin.tsx` does; `bun run lint` and `bun run format` must stay clean.

## 8. React 19 gotcha

StrictMode double-invokes effects in development, so a naive "connect" effect opens **two** WebSockets and sends two `join`s — registering two pending viewers for one tab. Either write the effect so its cleanup closes the socket, or create one `viewerUuid` per page load (module scope or a ref) and make the session idempotent.

## 9. Testing

- The frontend has **no test framework yet**. Add **Vitest** + jsdom and cover the pure, high-value pieces: `toRTCIceServers()` (omits `credentialType` for `Unspecified`/`Twilio`), the ICE candidate queue (queued before `setRemoteDescription`, flushed after, in order), the decline-reason mapping, and the uuid fallback. Do not attempt to test a real `RTCPeerConnection`; keep the media plumbing thin and verify it manually.
- **`bun test` runs bun's own runner, not the package `test` script.** Wire `"test": "vitest run"` and use `bun run test`.
- Rust must stay green: `cargo check`, `cargo fmt`, `cargo test` (after the human rebuilds `dist/`).

## 10. Verification and acceptance

1. In `webui/sharemyballs-webui`: `bun run typecheck`, `bun run lint`, `bun run test`.
2. **Ask the human to run `bun run build`**; confirm `dist/` contains only `index.html`.
3. `cargo check`, `cargo fmt`, `cargo test` — all green, including the `class="dark"` guard.
4. Manual end-to-end, which needs a display and the sharer: `pwsh -File scripts/build-windows.ps1`, then run `target\release\mira_sharer.exe --config config.toml` with `room` and `password` pinned (add `auto_accept = true` / `auto_start = true` to remove the clicks). Open the invite link on another device or browser → approve the pending viewer on `/admin` (or rely on `auto_accept`) → live video with audio (verify ~4K on Windows/Chrome).
5. Negative paths: wrong passcode → "Incorrect passcode."; Decline on `/admin` → "Declined by host."; Stop on `/admin` → "Session ended."; kill the sharer → socket loss then a working Reconnect; blocked autoplay → the audio affordance appears; fullscreen and Esc behave; a phone-sized viewport is usable.
6. Regression: `/admin` still starts/stops and accepts/declines/kicks; `webui.enabled = false` leaves the mirashare path untouched.

## 11. Non-goals

- Remote control / input forwarding (the `control` data channel stays ignored).
- Any change to the Rust signalling protocol, the axum routes, the admin plane, or `src/signaller/`.
- Canvas rendering of frames, recording, bitrate or quality controls, multi-viewer UI.
- Making the viewer page show `Live` status for its own sake, unless it falls out of the state machine naturally.

## 12. Working style

- Read the reference material in §6 before writing code; the retired page encodes decisions it took days to find.
- If you must break one of the §3 constraints, stop and ask instead.
- Keep `WEBUI.md` accurate: update its "What it is" and "Verification" sections when the viewer lands, and remove the "it does not stream video yet" warning.
- Report at the end: what you built, what you verified yourself, and exactly what the human still needs to run (build + the manual end-to-end), with the results of each check.
