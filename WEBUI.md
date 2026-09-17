# Web UI

## What it is

`webui/sharemyballs-webui/` is a client-only React SPA (Vite + TanStack Router +
Chakra UI). It builds to a **single self-contained `dist/index.html`**, which
`src/webui/mod.rs` inlines with `include_str!` and serves at `GET /` from the
in-process axum server — so the packaged `.exe`/`.app` needs no page files next
to it.

Two screens share the one bundle:

- `/` — the **viewer**. It reads `room`/`pwd` (and an optional `signaller`
  override) from the invite URL, joins the room over `/signaller`, waits for the
  host, and plays the live stream in a letterboxed `<video>` (plus a hidden
  `<audio>` for sound). No invite parameters in the URL means a small room +
  passcode form.
- `/admin` — the operator control plane (see below).

The viewer is view-only: the sharer's `control` data channel is ignored, so
there is no remote input. `<video>` is the media surface — the browser's
hardware-decoded path beats anything a canvas can do — and the retired
hand-written viewer (`webui/index.html`, 297 lines of vanilla JS) is what the
SPA port was built from. Its hard-won details are preserved in the checklist
below and in `src/lib/viewerSession.ts`.

### Viewer layout

```
src/lib/protocol.ts            wire types + signaller URL + decline-reason text
src/lib/uuid.ts                a v4 id that works outside a secure context
src/lib/ice.ts                 toRTCIceServers() + the pre-offer ICE queue
src/lib/fullscreen.ts          element fullscreen, with webkit fallbacks
src/lib/viewerSession.ts       socket + RTCPeerConnection state machine (no React)
src/hooks/useViewerSession.ts  React binding, incl. the beforeunload teardown
src/components/ViewerStage.tsx the media elements, overlay and controls
src/routes/index.tsx           join form | stage
```

The state machine is `idle → connecting → awaitingApproval → awaitingStream →
live`, plus `lost` (recoverable: the page offers Reconnect) and `failed`
(terminal: declined or the room closed).


## Build

Requires [bun](https://bun.sh) >= 1.2 (`bun.lock` is the text-format lockfile;
`package-lock.json` was removed so there is only one).

```sh
cd webui/sharemyballs-webui
bun install --frozen-lockfile
bun run build            # -> dist/index.html
```

`dist/` is gitignored and `include_str!` is a compile-time read, so **cargo
cannot build until the SPA has been built**. `build.rs` fails with the exact
command instead of an opaque `couldn't read ...` error, and
`MIRA_WEBUI_AUTOBUILD=1` makes it run the build itself. Both
`scripts/build-windows.ps1` and `make webui-build` run it before cargo.

Because the page is embedded, a change under `src/` needs a rebuild **and** an
app restart; reloading the browser tab is not enough.

## Build constraints

- `vite-plugin-singlefile` inlines every asset. The build must keep producing
  `dist/index.html` with **no sibling `assets/` directory**.
- Never add dynamic `import()` / `React.lazy()` / manual chunks: split chunks
  cannot be inlined and would be requested from a server that does not have
  them.
- `base: './'` keeps asset URLs relative, so the page works on loopback and
  behind the `webui.public_url` reverse proxy alike.
- The Rust test `signaller_relay_roundtrip` asserts that `GET /` contains
  `Mira Sharer` and does **not** contain `/src/main.tsx` — i.e. that the built
  page is embedded, not the Vite dev template (which would render blank).
- The embedded server has no SPA fallback: `/` and `/admin` are registered
  explicitly in `src/webui/mod.rs`, so a further route needs its own Rust route
  serving the same document.
- `class="dark"` on `<html>` in `index.html` is load-bearing — it is what puts
  Chakra's semantic tokens (`fg`, `bg`, `border`) into their dark values. Without
  it every token falls back to light and the page renders near-black text on the
  black background. The app is dark-only, so there is no runtime toggle. A Rust
  test asserts the served page carries the class.

## Dev workflow

```sh
cd webui/sharemyballs-webui && bun run dev     # http://localhost:3000
```

The dev server serves the page from Vite rather than from the binary, and
`vite.config.ts` proxies `/signaller` (WebSocket included) to
`ws://127.0.0.1:8765` — the viewer derives its signalling URL from
`location.host`, so without the proxy the page would open a socket back to the
Vite server. The same proxy is registered under `preview`.

### Frontend tests

`bun run test` runs **Vitest** (jsdom). `bun test` would run bun's own runner
instead and silently do nothing useful, so always use `bun run test`.

```sh
bun run test        # vitest run
bun run typecheck
bun run lint
```

The suite covers the parts that are pure and were expensive to get right:
`toRTCIceServers` (omitting `credentialType`), the pre-offer ICE queue and its
order, the decline-reason mapping, the UUID fallback, and the session's
send/state bookkeeping against fakes for `WebSocket`/`RTCPeerConnection`. A real
`RTCPeerConnection` is deliberately **not** exercised — jsdom has none, and a
fake SDP engine would only assert that the fake was called — so the media
plumbing itself is verified by eye against a running sharer.
`src/test-setup.ts` supplies the media-element APIs jsdom lacks
(`srcObject`, `muted`, a working `play()`).


## Admin control plane

Setting `[webui] admin_password` turns on an operator page at `/admin` that
replaces the desktop GUI's buttons: start/stop sharing, the invite details, and
accept/decline/kick for viewers. It works from any device that can reach the
server, so the sharer can be driven from a phone.

| Route | Auth | Purpose |
| --- | --- | --- |
| `GET /admin` | none | the same single-file bundle; the SPA router renders the admin screen |
| `GET /api/admin/state` | Bearer | running, room, passcode, invite link, `autoAccept`, pending and viewing viewers |
| `POST /api/admin/session/start` / `stop` | Bearer | start or stop the capture session |
| `POST /api/admin/viewers/:uuid/accept` / `decline` / `kick` | Bearer | decide on a viewer |

- **Fail closed.** With no usable secret (absent, empty or blank) these routes are
  never registered: `/admin` and `/api/admin/*` both 404. It is not "serve, then
  check".
- The secret is compared in constant time (`src/webui/admin.rs`) and travels as
  `Authorization: Bearer <admin_password>`. It is never logged. The viewer
  passcode is deliberately not accepted here, because every viewer knows it.
- A stale page cannot break anything: acting on a viewer that already left is a
  404, never a panic — `ViewerManager::send_viewer_auth_result` returns a result
  instead of unwrapping.
- The page **polls `/api/admin/state` every 2 seconds** while it is open. That is
  a deliberate v1 simplification of roadmap M2B.3 (a `broadcast` feeding an
  `/admin/ws` socket): polling already means a new pending viewer appears without
  anyone refreshing, and it avoids both a second WebSocket and putting a token in
  a URL. Push remains a follow-up.
- Display selection is **not** exposed: `WGCScreenCapture::select_display` has no
  effect on Windows (roadmap M2B.5), so a picker would be a lie.
- Reach it over TLS when it is available from off-box — the password is sent on
  every request. See `docs/deployment.md`.

## Server side

- `GET /` → the embedded page.
- `GET /admin` → the embedded page again, when `webui.admin_password` is set.
- `GET /signaller` → WebSocket room router implementing the mirashare signalling
  protocol between the sharer (`WebSocketSignaller`, already a client of it) and
  browser viewers. `src/config.rs` holds
  `[webui] enabled/port/bind/public_url/admin_password`; `enabled = false` falls
  back to the external signaller and viewer.

## Checklist for the viewer

Every item below is a fix that cost real debugging time in the retired page.
`src/lib/viewerSession.ts` and `src/lib/ice.ts` carry the same notes inline;
`src/lib/viewerSession.test.ts` pins the ones that are testable.

- **Read the track kind from `ev.track.kind`.** `RTCTrackEvent` has no `kind` of
  its own, so branching on `ev.kind` leaves both `pc.ontrack` branches
  unreachable: `video.srcObject` is never set and the viewer shows a black stage
  while frames decode perfectly. Reveal the stage only for a *video* track.
- Fall back to `new MediaStream([ev.track])` when the SDP carries no msid.
- **Queue ICE candidates that arrive before `setRemoteDescription`** and flush
  them after, in arrival order; the sharer can send ICE before the offer dance
  finishes. The queue must survive the "replace the peer connection" step of the
  dance — resetting it there is what silently drops those candidates.
- When mapping the sharer's ICE servers, **omit `credentialType`** for empty
  /`Unspecified`, and only ever pass `password`/`oauth`: `IceCredentialType`
  also serializes `Twilio`/`Signaller`, which the browser rejects with a
  `TypeError`.
- Keep the 30 s `keep_alive`: the server reaps connections idle for more than
  90 s (`IDLE_TIMEOUT` in `src/webui/mod.rs`).
- Generate the viewer id without `crypto.randomUUID()`: it is secure-context
  only, and a LAN viewer on `http://192.168.x.x:8765/` would throw before
  connecting. Use `getRandomValues` with a `Math.random` fallback.
- Distinguish **terminal** failures (`join_declined`, `room_closed`) from
  **recoverable** ones (socket loss → offer a Reconnect button).
- Send `{type:"leave"}` and close the socket/peer connection on `beforeunload` —
  a React effect cleanup does not fire when the tab closes.
- Handle the autoplay policy: a "click to enable audio" affordance, and
  `playsinline` on a muted `<video>`.
- The session is idempotent while a socket is live, and mints the viewer id once
  per page load: React 19's StrictMode double-invokes effects in development,
  and a second `join` would register a second pending viewer for one tab.

## Verification

1. `cd webui/sharemyballs-webui && bun run build`, then confirm `dist/` contains
   only `index.html`.
2. `bun run typecheck`, `bun run lint`, `bun run test`.
3. `cargo check`, `cargo fmt`, `cargo test` (per `AGENTS.md`).
4. Run the app and open the invite link: the page joins, waits for approval,
   then plays live video with sound. Then walk the negative paths — wrong
   passcode ("Incorrect passcode."), Decline on `/admin` ("Declined by host."),
   Stop on `/admin` ("Session ended."), kill the sharer (socket loss, then a
   working Reconnect), blocked autoplay (the audio button appears), fullscreen
   with Esc to exit, and a phone-sized viewport.
5. Regression: `/admin` still starts/stops and accepts/declines/kicks;
   `webui.enabled = false` leaves the mirashare path untouched.

