# Plan: WebUI — DTLS follow-up

Status: **done; verified in Chrome on Windows.** Companion to
`plan-local-webui.md`. See `vendor/README.md` for the DTLS change and
`tests/dtls_curve_selection.rs` for the regression tests.

## TL;DR

Two independent bugs produced the same "black screen":

1. **DTLS handshake failure** in the `webrtc-dtls 0.7.2` dependency (fixed in
   `vendor/webrtc-dtls`, see below). It affected the old mirashare flow identically.
2. **The viewer page never attached the video track.** `webui/index.html` read
   `ev.kind` in `pc.ontrack`, but `RTCTrackEvent` has no `kind` — the kind is on
   `ev.track`. Both branches were therefore unreachable, the overlay was hidden
   anyway, and the stage stayed black while Chrome happily decoded the frames.
   Fixed by branching on `ev.track.kind` (`webui/index.html`).

With both fixes, Windows + Chrome shows live 4K video (see "Verification" below).

## Second bug: the page dropped the video track

`pc.ontrack` used `ev.kind === "video"`. `RTCTrackEvent.prototype` exposes only
`receiver`, `track`, `streams` and `transceiver` (`"kind" in RTCTrackEvent.prototype`
is `false`), so `ev.kind` was `undefined`: neither branch ran, `video.srcObject` was
never set, and the `live` class + hidden overlay still turned the page into a black
stage. `getStats()` on the affected page showed `dtlsState: connected`, 278 frames
decoded at 3840x2160 — and `video.videoWidth === 0`.

The handler now branches on `ev.track.kind`, falls back to `new MediaStream([ev.track])`
when the SDP carries no msid, and only reveals the stage for a *video* track (an
audio-only offer would otherwise reproduce the same black stage).

## What was verified working

Ran `./target/release/mira_sharer --config config.toml` on Linux (WSL). Observed in logs:

- `WebUI listening on http://127.0.0.1:8765`
- `GET /` serves the viewer page (title "Mira Sharer"); `GET /signaller` completes the WS upgrade (`101`)
- Browser joined → auth pending → approved (`got authentication decision: true`)
- Offer/answer exchanged; ICE reached **`connected`**

So: page serving, signalling protocol, auth flow, and NAT traversal are all good.

## Note: WSL black screen (a separate, capture-side limitation)

During the WSL test the screen was black because `xcap` → `libwayshot` can't grab frames
without `ZxdgOutputManagerV1` v3 (which WSLg lacks). The real app targets Windows / native
Linux where capture works. This is unrelated to the two bugs above: on Windows capture was
demonstrably fine (Chrome decoded 3840x2160 frames the whole time).

## The first issue — DTLS handshake fails (real bug in `webrtc-dtls 0.7.2`)

```
WARN webrtc_dtls … Unsupported Extension Type 0 43 / 45 / 51   (harmless — see note)
peer connection state changed: failed
Failed to start manager dtls: invalid named curve
```

Root cause (in the dependency, not this repo):

- `webrtc-dtls-0.7.2/src/flight/flight0.rs:113`
  → `state.named_curve = e.elliptic_curves[0];`
  The server **unconditionally takes the first** group from the browser's
  `supported_groups` (extension 10) — no check that it is one the server supports.
- `webrtc-dtls-0.7.2/src/curve/named_curve.rs` recognizes only
  `P256 (0x17)`, `P384 (0x18)`, `X25519 (0x1d)`; everything else → `NamedCurve::Unsupported`.
- If the browser's *first* advertised group is unsupported (e.g. a post-quantum group such
  as `x25519mlkem768` that recent Chrome/Edge list first), `generate_keypair()` returns
  `ErrInvalidNamedCurve` → fatal alert → DTLS fails → no SRTP keys → no media.

Correct behavior: pick the first **mutually supported** curve, not `elliptic_curves[0]`.

Note on the `Unsupported Extension Type 0 43/45/51` warnings: these are TLS 1.3-style
extensions (`supported_versions`=43, `psk_key_exchange_modes`=45, `key_share`=51) that DTLS
1.2 doesn't use; they are harmlessly dropped. `supported_groups`(10) and `use_srtp`(14)
were parsed fine, so the failure is specifically the curve selection above.

This is **browser-dependent** and pre-existing: it would also break the mirashare viewer
with the same browser.

## Follow-up (fix the DTLS bug so Chrome/Edge work everywhere)

- [x] **A. Confirm the cause cheaply (optional):** the diagnosis is confirmed
      from the source rather than from a browser run: `NamedCurve::from(u16)`
      maps everything outside `{P256, P384, X25519}` to `Unsupported`, and
      `elliptic_curves[0]` was taken unconditionally.
- [x] **B1. Check newer releases.** Every version between 0.7.2 and 0.11.0 still
      uses `elliptic_curves[0]`; the fix first appears in **0.12.0**, which
      belongs to the sans-I/O `webrtc` 0.20+/0.21 rewrite. Upgrading would mean
      migrating the whole peer-connection/transceiver/track API, so it was
      rejected.
- [x] **B2. Patch `webrtc-dtls`.** Vendored `webrtc-dtls 0.7.2` at
      `vendor/webrtc-dtls` (kept at version 0.7.2 so it still satisfies
      `webrtc`'s `^0.7.2`) and wired it up with `[patch.crates-io]` in the root
      `Cargo.toml`.

      Deviation from the plan above: instead of imposing a server order of
      preference, the code now takes the first curve *the client* offers that we
      support — which is exactly what upstream 0.12.0 does. Keeping the client's
      preference avoids second-guessing it and keeps the diff minimal. The
      `None` case (empty list, or no overlap at all) returns the same fatal
      `insufficient_security` alert the empty-list case already used.
- [x] **C. Verify on a real target:** done on Windows with `config.toml` (`room`/`password`
      pinned, `auto_start`/`auto_accept` on) and headless Chrome driven over CDP. After both
      fixes the page plays by itself: `video.srcObject` holds the `video` track,
      `readyState 4`, `videoWidth/Height 3840x2160`, `currentTime` advancing,
      `dtlsState: connected`, `framesDecoded` climbing, and the decoded panel sampled
      non-black (99.8-100% of sampled pixels above black).

      Note this also retires the earlier "black screen = WSL capture" theory: capture,
      encode, ICE, DTLS and SRTP were all healthy; the picture simply never reached the
      `<video>` element.

## Repro / run notes

- `webui/index.html` is compiled into the binary with `include_str!`, so a page fix needs a
  rebuild (`scripts/build-windows.ps1`) and an app restart — reloading the browser tab is
  not enough.
- Linux requires the `--config <path>` flag (`config_path()` in `src/main.rs` only
  handles Windows/macOS and panics otherwise).
- Run: `cargo run --release -- --config config.toml`
- Invite link appears in the GUI Invite tab:
  `http://127.0.0.1:8765/?room=<room>&pwd=<passcode>`, or the configured
  `webui.public_url` when set.
- Approve the pending viewer in the GUI Viewers tab — or set
  `auto_accept = true` to skip the prompt entirely. Add `auto_start = true` to
  remove the "Start Sharing" click as well. See `docs/deployment.md`.
- Set `webui.enabled = false` to fall back to the external (mirashare) signaller/viewer.
