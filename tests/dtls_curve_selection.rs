//! Regression tests for the DTLS curve-selection fix in `vendor/webrtc-dtls`.
//!
//! `webrtc 0.7.3` pins `webrtc-dtls 0.7.2`, which selected the client's *first*
//! advertised elliptic curve without checking that it was one this
//! implementation supports. Current Chrome/Edge advertise the post-quantum
//! hybrid `X25519MLKEM768` ahead of the classical curves; that decodes to
//! `NamedCurve::Unsupported`, so keypair generation failed with
//! `invalid named curve`. The failure is fatal to the handshake: no SRTP keys
//! are derived and no media ever flows, while ICE still reports `connected`.
//!
//! Upstream only fixed this in `webrtc-dtls 0.12.0`, which belongs to the
//! sans-I/O `webrtc` 0.20+/0.21 rewrite. We vendor 0.7.2 with the curve
//! selection corrected instead; see `vendor/README.md`.
//!
//! These tests exercise the extracted selection logic directly, so they fail if
//! anyone regresses it back to `offered[0]`.

use webrtc_dtls::curve::named_curve::{select_supported_curve, NamedCurve};

/// The offering that broke the handshake: groups we do not implement come
/// first. `NamedCurve::Unsupported` is what `X25519MLKEM768` (0x11ec) and every
/// other unrecognised group decodes to.
#[test]
fn skips_unsupported_curves_ahead_of_supported_ones() {
    let offered = [
        NamedCurve::Unsupported,
        NamedCurve::Unsupported,
        NamedCurve::X25519,
        NamedCurve::P256,
    ];
    assert_eq!(select_supported_curve(&offered), Some(NamedCurve::X25519));
}

/// A peer that leads with a group we do support must still get that group.
#[test]
fn takes_the_first_curve_when_it_is_already_supported() {
    let offered = [NamedCurve::P256, NamedCurve::X25519];
    assert_eq!(select_supported_curve(&offered), Some(NamedCurve::P256));
}

/// No overlap at all is a clean `None`, which the caller turns into a fatal
/// `insufficient_security` alert rather than proceeding with a bogus curve.
#[test]
fn no_common_curve_returns_none() {
    let offered = [NamedCurve::Unsupported, NamedCurve::Unsupported];
    assert_eq!(select_supported_curve(&offered), None);
}

/// An empty extension is equally unusable.
#[test]
fn empty_offering_returns_none() {
    assert_eq!(select_supported_curve(&[]), None);
}

/// The whole point: whatever gets selected must be a curve that can actually
/// produce a keypair. This is the step that failed with `invalid named curve`,
/// so it is the strongest local proxy for "the browser will negotiate DTLS".
#[test]
fn selected_curve_can_always_produce_a_keypair() {
    for first in [
        NamedCurve::Unsupported,
        NamedCurve::P256,
        NamedCurve::P384,
        NamedCurve::X25519,
    ] {
        let offered = [first, NamedCurve::X25519];
        let picked = select_supported_curve(&offered).expect("a supported curve is present");
        assert_ne!(
            picked,
            NamedCurve::Unsupported,
            "selected an unsupported curve for lead-in {first:?}"
        );
        assert!(
            picked.generate_keypair().is_ok(),
            "selected curve {picked:?} cannot produce a keypair"
        );
    }
}
