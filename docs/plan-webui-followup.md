# Plan: WebUI — DTLS follow-up

Status: **deferred** (packed for later). Companion to `plan-local-webui.md`.

## TL;DR

The new embedded WebUI (`src/webui/`, page at `webui/index.html`) is **verified working**
end-to-end over loopback. The one real issue to fix is a **DTLS handshake bug in the
`webrtc-dtls 0.7.2` dependency** that affects the old mirashare flow identically.

The other observed problem (black screen) was purely a WSL capture limitation; the real app
runs on Windows / native Linux where `xcap` capture works, so it is **not** a concern here.

## What was verified working

Ran `./target/release/mira_sharer --config config.toml` on Linux (WSL). Observed in logs:

- `WebUI listening on http://127.0.0.1:8765`
- `GET /` serves the viewer page (title "Mira Sharer"); `GET /signaller` completes the WS upgrade (`101`)
- Browser joined → auth pending → approved (`got authentication decision: true`)
- Offer/answer exchanged; ICE reached **`connected`**

So: page serving, signalling protocol, auth flow, and NAT traversal are all good.

## Note: WSL black screen (not a concern)

During the WSL test the screen was black because `xcap` → `libwayshot` can't grab frames
without `ZxdgOutputManagerV1` v3 (which WSLg lacks). The real app targets Windows / native
Linux where capture works, so this is dropped — no action needed.

## The issue — DTLS handshake fails (real bug in `webrtc-dtls 0.7.2`)

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

- [ ] **A. Confirm the cause cheaply (optional):** on Windows/native Linux, retest the
      viewer in **Firefox** (lists x25519/P-256 first) vs Chrome/Edge. If Firefox goes
      `Live` and Chrome/Edge fail with `invalid named curve`, the curve-ordering bug is
      confirmed.
- [ ] **B. Fix the DTLS bug properly:**
  - [ ] B1. Check whether a newer `webrtc` / `webrtc-dtls` release already fixes the
        curve selection (least invasive). Current: `webrtc 0.7.3` → `webrtc-dtls 0.7.2`.
  - [ ] B2. Otherwise add a `[patch.crates-io]` for `webrtc-dtls` with a minimal fix:
        in `flight0.rs`, replace `e.elliptic_curves[0]` with the first curve in
        `[P256, P384, X25519]` (server order of preference) that is present in
        `e.elliptic_curves`; error only if none match.
  - After any change: `cargo check` && `cargo fmt` (per AGENTS.md).
- [ ] **C. Verify on a real target:** run on Windows or native Linux, share, open the
      invite link in Chrome/Edge, approve the viewer → expect `Live` video + audio.

## Repro / run notes

- Linux requires the `--config <path>` flag (`config_path()` in `src/main.rs` only
  handles Windows/macOS and panics otherwise).
- Run: `cargo run --release -- --config config.toml`
- Invite link appears in the GUI Invite tab:
  `http://127.0.0.1:8765/?room=<room>&pwd=<passcode>`
- Approve the pending viewer in the GUI Viewers tab to start the stream.
- Set `webui.enabled = false` to fall back to the external (mirashare) signaller/viewer.
