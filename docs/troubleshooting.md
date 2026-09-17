# Troubleshooting

## Viewer shows a black screen

The stage fills the window with black and no picture appears, usually right after the
sharer approves the viewer. In most cases the media pipeline is completely healthy, so
diagnose from the browser side first — "black screen" only means the bytes never reached
the `<video>` element, not that nothing was sent.

### 1. Ask the page what it has

In the viewer tab, open DevTools → Console. `pc`, `video` and the rest are top-level
`let`/`const` bindings of a classic script, so they are reachable from the console:

```js
const s = await pc.getStats();
s.forEach(r => {
  if (r.type === 'transport') console.log('dtls', r.dtlsState);
  if (r.type === 'inbound-rtp' && r.kind === 'video')
    console.log('video', r.framesDecoded, r.frameWidth, r.frameHeight);
});
const v = document.getElementById('video');
console.log('srcObject', v.srcObject, 'size', v.videoWidth, v.videoHeight);
```

### 2. Read the answer off the table

| `dtlsState` | `framesDecoded` | `video.videoWidth` | Meaning |
|---|---|---|---|
| `connected` | climbing | > 0 | The stream works — look elsewhere (window focus, GPU/compositing). |
| `connected` | climbing | `0` | The page never attached the track — **[A]**. |
| `failed`/`new` | `0` | `0` | DTLS handshake failed — **[B]**. |
| `connected` | `0` | `0` | Nothing is being sent — check the capture/encoder side, **[C]**. |

### [A] The page never attached the video track

`framesDecoded` climbing, `video.srcObject === null`, `videoWidth === 0`, and the page has
already hidden the overlay — so it looks like a black stage rather than like
"Waiting for stream…".

Cause: `pc.ontrack` branching on `ev.kind`. `RTCTrackEvent` has no `kind` of its own (only
`receiver`, `track`, `streams`, `transceiver`), so `ev.kind` is `undefined`: neither branch
runs, `video.srcObject` is never set, and the handler hides the overlay anyway. The official
viewer at `deploy/external/viewer/src/components/App/viewmodel.ts` reads `event.track.kind`
and is correct — do the same.

Fix: branch on `ev.track.kind`, and only reveal the stage for a *video* track.

> This is a **porting hazard, not a live bug** at the moment. The viewer that carried the fix
> was retired when the page became a React SPA (`webui/sharemyballs-webui/`, see `WEBUI.md`),
> and nothing currently attaches a video track. The test that guarded it
> (`viewer_page_reads_kind_from_the_track`, which grepped the old page source) went with the
> page: grepping a minified bundle would pass for the wrong reasons. The port must bring back
> both the handler and its test; `WEBUI.md` keeps the full checklist.

### [B] DTLS handshake fails (`invalid named curve`)

`webrtc-dtls 0.7.2` (pulled in by `webrtc 0.7.3`) took the *first* curve the client
advertised. Recent Chrome/Edge advertise the post-quantum hybrid `X25519MLKEM768` ahead of
the classical curves; it decodes to `NamedCurve::Unsupported`, keypair generation fails with
`invalid named curve`, DTLS aborts with a fatal alert and no SRTP keys are derived. ICE
still reports `connected`, which makes this look like a capture problem.

Fixed by the vendored crate at `vendor/webrtc-dtls`, wired up with `[patch.crates-io]` in
`Cargo.toml`; see `vendor/README.md` and `tests/dtls_curve_selection.rs`. If it ever fails
again, check that the patch entry and `vendor/` are still present, and that the build really
recompiled it — the dep-info file in `target/<profile>/deps/webrtc_dtls-*.d` must point at
`vendor/webrtc-dtls/...`.

### [C] Nothing arrives from the sharer

`framesDecoded === 0` with DTLS connected means no RTP reached the browser. Check the
sharer's stdout (redirect it — the release build is `#![windows_subsystem = "windows"]` and
has no console) for the encoder initialising (`libx264`/`libopus` lines), then for per-peer
messages such as `PLI received` and `WebRTC peer initialized`. A `PLI received` with no
frames points back at capture/encode; no `WebRTC peer initialized` at all points at
signalling or approval.

### The page is embedded in the binary

`src/webui/mod.rs` pulls the built SPA in with `include_str!`
(`webui/sharemyballs-webui/dist/index.html`, produced by `bun run build`). After editing
anything under `webui/sharemyballs-webui/src/` you must rebuild the page *and* restart the
app (`pwsh -File scripts/build-windows.ps1`); reloading the browser tab is not enough.

If `cargo check` reports that file missing, the SPA has not been built yet — see `WEBUI.md`.

A page that loads but renders blank usually means the *dev template* got embedded instead of
a build: it references `/src/main.tsx`, which the embedded server does not serve. The
`signaller_relay_roundtrip` test asserts against exactly that mistake.

## No audio in the viewer (known bug, not fixed)

On multichannel output devices (5.1/HDMI, e.g. a 6-channel 48 kHz WASAPI device) the
system-audio capture thread panics on its first callback:

```
thread 'cpal_wasapi_in' panicked at src\capture\audio\mod.rs:60:16:
range end index 2880 out of range for slice of length 960
```

`AudioCapture::capture` builds the Opus encoder as stereo but opens the cpal input stream
with the device's own (6-channel) config, so `write_input_data` copies the device's
interleaved samples into a 2-channel frame and overruns it. Audio dies immediately; video is
unaffected, and the panic is invisible unless stderr is redirected. Needs either a downmix
to the encoder's layout or a 2-channel stream config.
