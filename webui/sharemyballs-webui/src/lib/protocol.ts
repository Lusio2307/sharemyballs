// Hand-written TypeScript mirror of the signalling protocol.
//
// The Rust types live in `src/signaller/mod.rs` (`SignallerMessage`,
// `AuthenticationPayload`, `DeclineReason`, `IceServer`) and the router that
// relays them in `src/webui/mod.rs::route`. This file mirrors only the subset a
// *viewer* sends and receives. Keep the names in sync: every message is JSON
// tagged by `type` with `snake_case` values, and the field names are serde's
// defaults, so `credential_type` on the wire is snake_case even though the
// browser-side `RTCIceServer` field is `credentialType`.
//
// Wire contract, verbatim:
//   viewer -> server  {"type":"join","room":..,"from":..,"name":"WebUI","auth":..}
//   viewer -> server  {"type":"keep_alive"}
//   viewer -> server  {"type":"answer","sdp":..,"from":..,"to":<room>}
//   viewer -> server  {"type":"ice","ice":..,"from":..,"to":<room>}
//   viewer -> server  {"type":"leave","from":..}            (on beforeunload)
//   sharer -> viewer  {"type":"offer","sdp":..,"from":<room>,"to":..,"ice_servers":[..]}
//   sharer -> viewer  {"type":"ice","ice":..,"from":<room>,"to":..}
//   sharer -> viewer  {"type":"join_declined","reason":0-3,"to":..}
//   sharer -> viewer  {"type":"room_closed","to":..,"room":..}
//
// A viewer's `to` on `answer`/`ice` is the **room id** (the router resolves
// viewer UUIDs first, then rooms); its `from` is always its own viewer UUID.
// Note that `Answer` has no `room` field: the room is read from the invite URL.

/** The viewer name the sharer sees on `/admin`. */
export const VIEWER_NAME = 'WebUI'

/** The server reaps connections idle for more than 90 s (`IDLE_TIMEOUT`). */
export const KEEP_ALIVE_INTERVAL_MS = 30_000

/**
 * `IceCredentialType` in `src/config.rs`. The variants serialize as written
 * (PascalCase), and only `Password`/`Oauth` map onto a credential type the
 * browser accepts -- see `src/lib/ice.ts`.
 */
export type WireCredentialType =
  'Unspecified' | 'Password' | 'Oauth' | 'Twilio' | 'Signaller'

/** `IceServer` in `src/config.rs`, as it arrives inside an `offer`. */
export type WireIceServer = {
  urls: Array<string>
  username?: string
  credential?: string
  credential_type?: WireCredentialType
}

/** `RTCIceCandidateInit`, as the sharer serializes it. */
export type WireIceCandidate = {
  candidate?: string
  sdpMid?: string | null
  sdpMLineIndex?: number | null
  usernameFragment?: string | null
}

/** `RTCSessionDescription`, as the sharer serializes it. */
export type WireSessionDescription = {
  type: 'offer' | 'answer' | 'pranswer' | 'rollback'
  sdp?: string
}

// ---- viewer -> server -------------------------------------------------------

export type JoinMessage = {
  type: 'join'
  room: string
  from: string
  name: string
  auth: { type: 'password'; password: string }
}

export type KeepAliveMessage = { type: 'keep_alive' }

export type AnswerMessage = {
  type: 'answer'
  sdp: RTCSessionDescriptionInit
  from: string
  to: string
}

export type IceMessage = {
  type: 'ice'
  ice: RTCIceCandidateInit
  from: string
  to: string
}

export type LeaveMessage = { type: 'leave'; from: string }

/** Everything a viewer is allowed to send. */
export type ViewerOutbound =
  JoinMessage | KeepAliveMessage | AnswerMessage | IceMessage | LeaveMessage

// ---- sharer -> viewer -------------------------------------------------------

export type OfferMessage = {
  type: 'offer'
  sdp: WireSessionDescription
  from: string
  to: string
  ice_servers: Array<WireIceServer>
}

export type RemoteIceMessage = {
  type: 'ice'
  ice: WireIceCandidate
  from: string
  to: string
}

/**
 * `DeclineReason` in `src/signaller/mod.rs`. The sharer serializes the variant
 * (not the discriminant number) over the socket: `"Unknown"`,
 * `"IncorrectPassword"`, `"NoCredentials"`, `"UserDeclined"`.
 */
export type DeclineReason =
  'Unknown' | 'IncorrectPassword' | 'NoCredentials' | 'UserDeclined'

export type JoinDeclinedMessage = {
  type: 'join_declined'
  reason: DeclineReason
  to: string
}

export type RoomClosedMessage = {
  type: 'room_closed'
  to: string
  room: string
}

/**
 * Messages a viewer understands. Anything else (`start_response`,
 * `ice_servers_response`, ...) is ignored on arrival: the embedded router
 * relays raw JSON between the sharer and every viewer in a room, so a viewer
 * can observe traffic addressed to other peers.
 */
export type ServerMessage =
  OfferMessage | RemoteIceMessage | JoinDeclinedMessage | RoomClosedMessage

/**
 * The human-readable text for a `join_declined`. The retired viewer mapped the
 * numeric discriminant, but the sharer sends the variant name; both spellings
 * are accepted so a protocol drift degrades to a sensible message instead of a
 * silent hang.
 */
export const declineReasonText = (reason: unknown): string => {
  switch (reason) {
    case 'IncorrectPassword':
    case 1:
      return 'Incorrect passcode.'
    case 'NoCredentials':
    case 2:
      return 'No credentials provided.'
    case 'UserDeclined':
    case 3:
      return 'Declined by host.'
    case 'Unknown':
    case 0:
      return 'Join was declined.'
    default:
      return 'Join was declined.'
  }
}

/**
 * The signaller to dial. `signaller` in the query string is an escape hatch for
 * development; otherwise the URL is derived from the page's own origin, which is
 * what makes the `webui.public_url`/Caddy path work (Caddy terminates TLS on
 * `stream.example.com` and proxies the upgrade through to 127.0.0.1:8765).
 */
export const signallerUrl = (
  location: Pick<Location, 'protocol' | 'host'>,
  override?: string | null,
): string => {
  const trimmed = override?.trim()
  if (trimmed) {
    return trimmed
  }

  const scheme = location.protocol === 'https:' ? 'wss://' : 'ws://'
  return `${scheme}${location.host}/signaller`
}
