# Vendored crates

## `webrtc-dtls` 0.7.2 — DTLS curve-selection fix

Vendored because `webrtc 0.7.3` depends on `webrtc-dtls 0.7.2`, which selects
the *first* elliptic curve the client advertises without checking that it is one
this implementation supports:

```rust
// vendor/webrtc-dtls/src/flight/flight0.rs (before)
state.named_curve = e.elliptic_curves[0];
```

`NamedCurve` implements only `P256`, `P384` and `X25519`; every other value
decodes to `NamedCurve::Unsupported`. Current Chrome and Edge advertise the
post-quantum hybrid `X25519MLKEM768` ahead of the classical curves, so the
server selected an unsupported curve, keypair generation failed with
`invalid named curve`, DTLS aborted with a fatal alert, no SRTP keys were
derived, and no media ever flowed.

The failure is easy to misdiagnose: ICE still reports `connected`, the capture
and encoder pipelines look healthy, and the browser simply shows nothing.

### The fix

The unconditional `offered[0]` is replaced by the first *supported* curve in the
client's list, extracted into `select_supported_curve` in
`src/curve/named_curve.rs`:

```rust
pub fn select_supported_curve(offered: &[NamedCurve]) -> Option<NamedCurve> {
    offered
        .iter()
        .copied()
        .find(|curve| *curve != NamedCurve::Unsupported)
}
```

`flight0.rs` converts `None` into the same fatal `insufficient_security` alert
that the empty-list case already used, rather than silently continuing with a
curve the client never offered.

Regression tests live in `tests/dtls_curve_selection.rs` and run as part of
`cargo test` via the `webrtc-dtls` dev-dependency in the root `Cargo.toml`.

### Why not just upgrade?

Upstream fixed this only in `webrtc-dtls 0.12.0`. Versions 0.8.0, 0.9.0 and
0.11.0 still contain the bug, and 0.12.0 belongs to the sans-I/O `webrtc`
0.20+/0.21 rewrite. Adopting it would mean migrating the entire peer-connection,
transceiver and track API for a one-line fix. Vendoring 0.7.2 keeps the project
on `webrtc 0.7.3`.

### Updating this crate

Everything except the change above is identical to the crates.io 0.7.2 release.
If the crate is ever re-vendored, re-apply the fix and keep
`version = "0.7.2"` so the `[patch.crates-io]` entry in the root `Cargo.toml`
still satisfies `webrtc`'s `^0.7.2` requirement.
