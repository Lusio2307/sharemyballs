# Deployment

## Topology

```
Windows box (the machine whose screen is shared)
┌────────────────────────────────────────────────────────────┐
│  mira_sharer.exe                                           │
│    ├─ capture + encode                                     │
│    ├─ WebRTC peer connections ───────────────┐             │
│    └─ embedded webui on 127.0.0.1:8765       │             │
│           ▲                                  │             │
│  Caddy ───┘  https://stream.example.com      │             │
│    (TLS + reverse proxy to 8765)             │             │
└──────────────────────────────────────────────┼─────────────┘
                                               │
                        WebRTC media (SRTP), direct where possible
                                               │
                          ┌────────────────────▼─────────────┐
                          │ viewers' browsers                │
                          │ https://stream.example.com/?room=…│
                          └────────────────────┬─────────────┘
                                               │ only if a direct
                                               │ path cannot be found
                          ┌────────────────────▼─────────────┐
                          │ coturn on a host with a public IP │
                          │ STUN/TURN, ports 3478 + relay     │
                          └───────────────────────────────────┘
```

Video goes browser-to-sharer directly whenever NAT traversal succeeds. Media
only traverses coturn when it does not, which is what keeps a small server
sufficient for a handful of viewers.

## What runs where

| Component | Where | Why |
| --- | --- | --- |
| `mira_sharer` | the machine with the display | it is the capture source |
| Caddy | same machine | it proxies to `127.0.0.1:8765`, so it must be local |
| coturn | any host with a stable public address | viewers must reach it directly |

## 1. Configuration

```powershell
Copy-Item config.toml.example config.toml
```

Set at minimum:

```toml
room = "desk"
password = "<a real secret>"
auto_accept = true
auto_start = true

[webui]
enabled = true
bind = "127.0.0.1"
public_url = "https://stream.example.com/"
admin_password = "<a different secret>"
```

`bind` stays on loopback: Caddy is what faces the network, and it terminates TLS
for you. Set `public_url` to the address viewers use, so the invite link in the
GUI is correct.

Set `admin_password` to drive the session from a browser: `https://<domain>/admin`
then serves the operator page (start/stop sharing, accept or decline viewers).
Use a **different** secret from the viewer passcode — viewers are given that one.
It travels as an `Authorization` header on every admin request, so serve the
admin page over TLS; over plain HTTP on a LAN anyone on that network can read it.
Leaving the key unset does not just disable a button: the admin routes are not
registered, so `/admin` and `/api/admin/*` return 404.

Add your coturn instance to `[[ice_servers]]` (see `config.toml.example`). Leave
it empty if every viewer is on the LAN.

## 2. TLS termination

Install Caddy, copy `deploy/Caddyfile` next to it, and replace
`stream.example.com` with your domain. Point that domain's A/AAAA record at the
sharer machine and forward 443 to it if it is behind NAT.

```sh
caddy run --config deploy/Caddyfile
```

Caddy obtains and renews the certificate automatically. The viewer page derives
its WebSocket URL from its own origin, so it switches to `wss://` with no
further configuration.

## 3. TURN (only for off-LAN viewers)

On the host with the public address, edit `deploy/coturn/turnserver.conf`
(`external-ip`, `realm`, and the `user=` password) and start it:

```sh
docker compose -f deploy/docker-compose.yml up -d
```

Open in the firewall: `3478/udp`, `3478/tcp`, and the relay range
`49152-65535/udp` (or whatever you set `min-port`/`max-port` to).

Then mirror the same username/password into `config.toml`:

```toml
[[ice_servers]]
urls = ["stun:stream.example.com:3478"]

[[ice_servers]]
urls = ["turn:stream.example.com:3478"]
username = "mira"
credential = "<same password as turnserver.conf>"
credential_type = "Password"
```

## 4. Start automatically

With `auto_start = true` and `auto_accept = true`, the app needs no interaction
after launch. Register it as a logon task (Task Scheduler) or a service so it
comes back after a reboot. Note that screen capture requires an interactive
session, so a logon task is the right shape on Windows rather than a true
service.

## Watching from another device on the same network

No domain, no Caddy and no coturn are needed. Three things are:

**1. Listen on the LAN.** In `config.toml`:

```toml
[webui]
bind = "0.0.0.0"
# So the Invite tab shows a link other devices can actually open:
public_url = "http://192.168.1.20:8765/"
# Optional: drive the session from a phone at http://192.168.1.20:8765/admin.
# Sent in the clear over plain HTTP -- see the TLS note in the section above.
# admin_password = "<a different secret>"
```

Find the address with `ipconfig`. Without `public_url` the invite link keeps
pointing at `127.0.0.1`, which only resolves on the sharer itself.

**2. Allow it through Windows Firewall.** Both the page (inbound TCP 8765) and
the WebRTC media (inbound UDP on ephemeral ports) must get in, so allow the
program rather than a single port. From an elevated PowerShell:

```powershell
New-NetFirewallRule -DisplayName "Mira Sharer" -Direction Inbound `
  -Program "D:\home\Luciano\repository\sharemyballs\target\release\mira_sharer.exe" `
  -Action Allow -Profile Private
```

**3. Open `http://<sharer-ip>:8765/?room=<room>&pwd=<password>`** on the other
device.

Notes:

- **Plain HTTP is fine here.** `RTCPeerConnection` and `WebSocket` are not
  restricted to secure contexts -- only capture APIs are. The viewer page
  deliberately avoids secure-context-only APIs: `crypto.randomUUID` is
  reimplemented on top of `crypto.getRandomValues`, which carries no such
  restriction. Serving over `wss://` via Caddy is still the right answer for
  viewers outside the LAN, but it is not required on it.
- Audio playback needs one tap on the page, per the browser autoplay policy.
- If the sharer has extra network adapters (VPN, WSL's vEthernet), it may
  advertise host candidates the viewer cannot reach. ICE tries them all, so an
  unreachable one only costs a little connection time.
- Anyone on the network who knows the room id and password can watch. With
  `auto_accept = true` the password is the only gate, so make it a strong one.

## Verifying

1. **It works at all** — open the invite link in Chrome/Edge. The status should
   go `Connecting…` → `Waiting for stream…` → `Live`.
2. **TURN actually works** — open `chrome://webrtc-internals` while a session is
   live and look at the selected candidate pair. A pair of type `relay`
   (`relay` ↔ `relay`, or `relay` ↔ `srflx`) means the stream is going through
   coturn; `host`/`srflx` means it found a direct path and TURN was never
   exercised. To force the relay path, test from a network that cannot reach the
   sharer directly — that is the case coturn exists for.
3. **No third-party services** — confirm the running config lists only your own
   hosts:
   ```sh
   grep -riE "stun\.google|openrelay|twilio|mirashare\.app" config.toml
   ```
   It should print nothing.

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| Page loads, status stalls at `Waiting for approval…` | `auto_accept` is false and nobody clicked Accept in the GUI |
| Page loads, status reaches `Live`, but the video is black | DTLS failed — check the log for `invalid named curve` and see `vendor/README.md` |
| `join_declined` | wrong password, or the room id does not match a running session |
| Works on the LAN but not from outside | no TURN configured, or its ports are closed |
| `room_closed` | the sharer stopped or the session was kicked |
| Encoder creation panics on startup | an invalid `[encoder.options]` value; unknown options make `avcodec_open2` fail. See `configs/config.nvenc.toml` for a fixed NVENC preset |
