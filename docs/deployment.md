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
```

`bind` stays on loopback: Caddy is what faces the network, and it terminates TLS
for you. Set `public_url` to the address viewers use, so the invite link in the
GUI is correct.

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

## LAN-only deployment

If every viewer is on the same network you need none of the above: no domain, no
Caddy, no coturn. Set `bind = "0.0.0.0"`, open port 8765, and browse to
`http://<sharer-ip>:8765/?room=<room>&pwd=<password>`.

Browsers only expose WebRTC on secure origins, but `http://localhost` and
plain-HTTP LAN origins are treated as secure enough in practice for this to
work; `wss://` via Caddy is the robust path if you hit trouble.

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
