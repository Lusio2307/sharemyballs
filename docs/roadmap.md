# Roadmap

Next work, in the order it should be done, with the decisions each piece needs and the
traps already identified in the code. Task ids (`M1.2`, …) are stable, so they can be
tracked as todos or issues.

Two constraints apply to everything below:

- **The web UI is a bundled SPA (decided — this supersedes the earlier "no web build step"
  constraint).** `webui/sharemyballs-webui/` is a client-only React + Vite + TanStack Router
  + Chakra UI project, built with bun into a single self-contained `dist/index.html` that is
  still inlined into the binary via `include_str!`. The bundler was introduced deliberately:
  the page outgrew hand-written vanilla JS, and single-file output preserves the "no runtime
  files next to the binary" property. Keep it one file — no dynamic imports, no manual
  chunks. See `WEBUI.md` for the build step and for the checklist of viewer behaviour that
  still has to be ported onto the SPA.
- **Windows is the reference platform** for capture (WGC). macOS/Linux backends exist but
  are unverified here; don't delete them on the assumption they are dead (see M3).

---

## Milestone 1 — Reach the viewer page from other devices on the LAN

**Why:** this already works, but only after the operator reads `docs/deployment.md` and
hand-assembles it (find IP, set `public_url`, add a firewall rule). Make it a product
feature rather than a checklist.

**Already in place** (verified): `webui.bind` and `webui.public_url` config keys, the
loopback-only default, the LAN walk-through in `docs/deployment.md` §"Watching from another
device on the same network", the secure-context-safe viewer id (commit `1784093`), and
host-candidate-only ICE which is enough on a LAN.

### Tasks

- **M1.1 — Derive the invite link from the actual bind.** `Capturer::get_invite_link()`
  (`src/capture/capturer.rs`) falls back to `http://127.0.0.1:<port>/`, which is useless to
  a phone. When `public_url` is unset and `bind` is not loopback, advertise a reachable
  address instead. Done when: `bind = "0.0.0.0"` with no `public_url` produces a link that
  opens from another device.
- **M1.2 — Handle multiple adapters.** This machine already has VirtualBox, WSL and Wi-Fi
  addresses; picking "the" IP blindly will pick wrong. Either enumerate candidate URLs (one
  per adapter, shown in the admin page / logged) or let the operator choose
  (`webui.lan_address`). Prefer `GetAdaptersAddresses` via the existing `windows` crate
  over a new dependency. Done when: with ≥2 non-loopback adapters, the operator can get the
  reachable URL without editing config by hand.
- **M1.3 — Stop hardcoding the sharer's own signaller URL.** `Capturer::signaller_url()`
  returns `ws://127.0.0.1:<port>/signaller`; harmless with `bind = "0.0.0.0"`, but if `bind`
  is a specific LAN address the app cannot reach itself and never gets a room. Derive the
  client URL from `bind`, or always connect to loopback while binding wider. Done when: the
  app gets a room with `bind` set to a single non-loopback address.
- **M1.4 — Firewall.** Ship the elevated `New-NetFirewallRule` one-liner in the docs (it is
  there) *and* detect the failure: when binding fails or the first LAN join never arrives,
  log an explicit hint naming the rule to add. Optional: a `--install-firewall-rule` flag.
- **M1.5 — Brute-force and exposure guard rails.** With `bind != 127.0.0.1` and
  `auto_accept = true`, the room password is the only gate on the whole LAN, and the local
  signaller has no throttling. Add a simple per-IP attempt throttle/lockout and log a
  warning at startup when a non-loopback bind is combined with `auto_accept`. Done when:
  repeated wrong passwords from one address are delayed rather than free.
- **M1.6 — Mobile/LAN viewer polish.** Phone ergonomics on the viewer page: a fullscreen
  button, a visible `Live` indicator (the page never sets the documented `Live` status), and
  the audio unblock affordance that already exists. Keep `playsinline`.
- **M1.7 — Verify cross-device for real.** Phone + laptop, Chrome and Safari, including a
  viewer whose host candidates are mDNS-obfuscated (`.local`). `webrtc-ice` ships an mDNS
  resolver and a lever for it (`SettingEngine::set_ice_multicast_dns_mode`); confirm the
  effective default is not `Disabled` before assuming cross-device works by accident.

**Acceptance:** a device that has never seen this repo opens a link, or scans a QR from the
admin page, and gets live video without anyone editing config or reading a doc.

---

## Milestone 2 — Webapp only: drop the GUI, add an admin webapp

**Why:** the operator surface should be a browser page, not an iced window, so the sharer
can run unattended on a box with no display attached to *its* session, be driven from a
phone, and lose the entire GPU/windowing dependency tree.

### 2A. Decide the shape (the open question)

**Recommendation: one server, two routes in the SPA.** Serve `/` (viewer, the thing you hand
to other people) and `/admin` (a route that embeds the same media code and adds controls).
The shared code lives in framework-agnostic modules under
`webui/sharemyballs-webui/src/` (protocol + media), imported by both routes. The old plan — a
served `webui/viewer.js` with no build step — is obsolete now that the page is a bundled SPA
(see `WEBUI.md`).

The server has **no SPA fallback**, so `/admin` is registered as its own Rust route serving
the same built document. The auth gate turned out to belong to 2B, not 2C.

Why not "extend the current page with an admin panel": the viewer page's audience is
untrusted (you send its URL to other people), and the admin page's is not. Keeping them
separate routes keeps the privileged UI out of any page that can be shared by accident,
and makes the auth check a property of one route rather than of one DOM subtree. The cost is
one route and a shared module.

**Decided and implemented (v1).** `/admin` is a route in the same bundle, served by
`GET /admin`, gated per request by the admin secret, and control-only — it does not show the
live stream, because the viewer has not been ported back yet. Consequence worth stating: the
privileged *code* therefore ships to viewers as well, so the gate is the server's 401/404, not
the page's absence. A separate admin document remains a possible later refinement.

### 2B. Backend: make the app admin-able

Today the webui server has no access to the app at all:

- **M2B.1 — Wire the server to the app.** **Done.** `main` now builds the `Capturer` as an
  `Arc<Mutex<Capturer>>` and shares it: the iced app drives it directly, and the server
  reaches it through `AppControl` in `src/session.rs`, behind the `SessionControl` trait, so
  the routes can be tested against a fake with no capture backend. Iced's flags became
  `(Arc<Mutex<Capturer>>, Receiver<()>)`; `Args`/`Config` ride along inside the `Capturer`.
- **M2B.2 — Admin auth, fail closed.** **Done.** `[webui] admin_password` plus
  `Authorization: Bearer` (constant-time compare, never logged), with the secret held in
  `sessionStorage`. With no usable secret the routes are not registered at all, so `/admin`
  and `/api/admin/*` 404 — covered by a test. Loopback-only was rejected: it would make the
  page useless from a phone, which is the point of the milestone.
- **M2B.3 — Admin push channel.** **Deferred on purpose; v1 polls instead.** The page calls
  `/api/admin/state` every 2 s, which already satisfies "a new pending viewer appears with no
  refresh" without a second WebSocket, a `broadcast` threaded through `ViewerManager` and
  `WebSocketSignaller`, or a token in a WS query string. `notify_update` still pokes iced and
  is untouched. Revisit if the poll ever shows up in a profile.
- **M2B.4 — Command surface.** **Done, except display selection**, which is deliberately not
  exposed until M2B.5 is fixed. One HTTP command per admin action, mapping 1:1 onto methods
  that already exist:

  | GUI element today | Backing API |
  | --- | --- |
  | Display picker | not exposed — see M2B.5 |
  | Start Sharing | `Capturer::run` |
  | End | `Capturer::shutdown` |
  | Room / Passcode / Invite Link (+ copy) | `get_room_id` / `get_room_password` / `get_invite_link` |
  | Accept / Decline a pending viewer | `ViewerManager::permit_viewer` / `decline_viewer` |
  | Kick a viewing viewer | `Capturer::kick_viewer` + `ViewerManager::kick_viewer` |
  | Live refresh | `notify_update` → `/admin/ws` (M2B.3) |

- **M2B.5 — Fix display selection before exposing it.** **Still open, and the reason `/admin`
  has no display picker.** On the WGC backend,
  `WGCScreenCapture::select_display` builds a new `CaptureEngine` and stores it, but
  `start_capture` builds a *fresh* engine from `self.item`, which is still display 0 from
  `new()`. So the current picker has no effect on what is captured on Windows — an admin
  dropdown would be a lie. Done when: selecting display 2 actually captures display 2.

### 2C. Remove the GUI

- **M2C.1 — Move `main` to a headless daemon.** Delete `src/gui/**`, the `App::run` call and
  the iced `Settings`. `auto_start` becomes the normal path (or is folded away), and the
  `--display`, `--file`, `--disable-control`, `--profiler`, `--config` CLI args stay.
- **M2C.2 — Drop the dependencies.** `iced`, `iced_aw` and their transitive tree (wgpu,
  winit, …) leave `Cargo.toml`/`Cargo.lock`. Also review `[package.metadata.bundle]`,
  `resources/Barlow-*.ttf`, `resources/Material-Icons.ttf` and the iced window icon include
  in `main.rs`. Keep `mira-manifest.rc` / `build.rs` / `mira_sharer.exe.manifest` (exe
  manifest) and the macOS bundle metadata if macOS stays supported.
- **M2C.3 — Logs without a console.** The binary is `#![windows_subsystem = "windows"]` and
  after M2C.1 there is no window at all, so today's stdout-only `fern` setup is invisible.
  Add file logging (default under `%LOCALAPPDATA%`, path configurable, size-capped), and
  keep a console variant for debugging (`--console`, or a `console` cargo feature).

### 2D. Unattended operation

- **M2D.1 — Supervision.** With no GUI, a panic in the capture task is a silent death that
  looks exactly like the black-screen bug we just fixed. Replace the `.expect("TODO: panic
  message")` calls on the media path (`src/output/webrtc_output.rs`, the `unwrap()`s in
  `src/capture/wgc/wgc_capture.rs`) with logged errors, and make the process exit non-zero
  (or restart capture) when the pipeline stops so a logon task can recover it.
- **M2D.2 — Autostart.** Keep the documented Task Scheduler/logon-task approach: WGC needs
  an interactive session, so a session-0 Windows service cannot capture. Say so explicitly
  in the docs once the GUI is gone, since "no GUI" invites the wrong assumption.
- **M2D.3 — Optional tray icon.** If a taskbar presence is wanted for "is it running /
  stop", `Shell_NotifyIcon` via the existing `windows` crate avoids a new dependency.

**Acceptance:** no window, no `iced` anywhere in the tree, and every action the old GUI
offered is available from `/admin` to an authenticated operator, from another device.

**Status:** 2A and 2B are **done** — the admin surface exists and is tested. 2C (remove the
GUI) and 2D (unattended operation) remain; until they land the iced app stays as the fallback
operator surface, which is also what keeps `webui.enabled = false` working.

---

## Milestone 3 — Codebase cleanup

**Method:** inventory with `cargo machete` (unused deps), `cargo +nightly udeps`, `cargo
clippy -- -D warnings`, `cargo tree -d` (duplicate versions), and the compiler's own
`dead_code` warnings; record binary size before/after. Remove one thing per commit with a
`cargo check` + `cargo test` in between.

**Known dead code** (already confirmed, safe to remove):

- `src/output/noop_output.rs` and its `pub use` — `NoOpOutput` is never constructed.
- `YUVFrame::display_time` (`src/capture/frame.rs`) — never read; produces the only
  compiler warning in the crate.
- `FrameData::BGR0` (`src/encoder/ffmpeg.rs`) — no platform constructs it; all three
  capture backends use `FrameData::NV12`.
- The `api` field on `WebRTCOutput` (kept alive by `#[allow(dead_code)]`).

**Dependency candidates** (each needs a decision, not just a grep):

- `twilio-rs` + `base64` + `IceCredentialType::Twilio` — only reachable when a user
  configures `credential_type = "Twilio"`, which contradicts the project's "no third-party
  services" default. Removing it also drops the `httpclient`/`hyper-rustls` subtree.
- `chrono` + `howlong` — used only by `PerformanceProfiler`; `std::time::Instant` replaces
  both.
- `strum` + `strum_macros` — exist only to map `SignallerMessage` variants to topic strings;
  a `match` does it. Low value, medium risk: do it only if M3 has slack.
- Anything the GUI removal (M2C.2) makes unused: fonts, icons, window icon, and possibly
  `directories` if the config-path logic changes shape.

**Repository hygiene:**

- `.idea/vcs.xml` and `mira_sharer.iml` are tracked even though `.idea/` is in
  `.gitignore` — drop them from the index.
- `deploy/external/{viewer,signaller}` are untracked, git-ignored local clones (with
  `node_modules`) kept as protocol references. Decide: keep as documented reference, make
  them submodules, or delete. They are not repo bloat, just disk.
- `configs/` presets are all asserted to parse by `bundled_presets_parse` (which requires
  ≥4). If a preset (e.g. VP9) is not actually verified to stream, either verify it or drop
  it and update the test.

**Docs consolidation:**

- `WEBUI.md` describes a design that was never built (static page, external signaller, "no
  Rust changes") — it is now actively misleading. Delete or rewrite.
- `docs/plan-local-webui.md` and `docs/plan-webui-followup.md` are history; fold what still
  matters into `docs/troubleshooting.md` / this roadmap and delete them.
- `docs/project-overview.md` is stale: it claims wgpu shaders (the Windows path is D3D11 +
  precompiled HLSL), "no web frontend in this repo", and an iced GUI that M2 deletes.
  Rewrite after M2, and make it the docs index.
- `.cargo/config.toml` is a comment-only file kept as build documentation; keep or move the
  text into `docs/build-windows.md`.

**Acceptance:** `cargo machete` clean, no `dead_code` warnings, no stale design docs, and a
recorded before/after of binary size and `cargo tree` package count.

---

## Known bugs to fold into a milestone

| Bug | Evidence | Where to fix |
| --- | --- | --- |
| System audio panics on multichannel devices | `cpal_wasapi_in` panic, `src/capture/audio/mod.rs:60`; stereo Opus encoder vs 6-channel device | M2D or its own small change |
| Display picker has no effect on Windows | `WGCScreenCapture::select_display` vs `start_capture`'s `self.item` | M2B.5 |
| Viewer page never shows `Live` | `pc.ontrack` hid the overlay without setting status | M1.6 |

---

## Suggested order

1. **M1** — small, self-contained, and it makes the rest testable from a second device.
2. **M2B → 2A → 2C → 2D** — build the replacement and prove it before deleting the GUI.
   Deleting the GUI first leaves a period with no operator surface at all.
3. **M3** — sweep up everything the removals exposed, so the inventory does not have to be
   done twice.

## Open questions

1. Admin auth model: separate admin password, loopback-only, or both?
2. Should `/admin` also show the live stream (watch + control in one page), or control only?
3. Keep the external mirashare signaller/viewer path (`webui.enabled = false`) or make the
   embedded webui mandatory?
4. Is macOS/Linux still supported, or is this Windows-only now? (Affects M3: platform
   backends, `Makefile`, bundle metadata.)
5. OK to remove the Twilio ICE path and its dependencies?
6. With one shared encoded stream, every viewer costs the same 4K bitrate. Cap
   resolution/fps for LAN viewers, or leave it?
7. Tray icon, or truly no visible process?
