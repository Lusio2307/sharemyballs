import type { WireIceServer } from '#/lib/protocol'

/**
 * `RTCIceServer` plus the one field TypeScript's `lib.dom` omits.
 *
 * `credentialType` is part of the WebRTC spec and accepted by every browser,
 * but the DOM lib declares only `urls`/`username`/`credential`. Widening it
 * here keeps the mapping type-checked instead of reaching for `as any`.
 */
type MappedIceServer = RTCIceServer & { credentialType?: 'password' | 'oauth' }

/**
 * Map the sharer's `ice_servers` onto `RTCIceServer`.
 *
 * Two rules, both learned the hard way:
 *
 * 1. **Omit `credentialType` for `Unspecified`.** The Rust enum serializes
 *    `Unspecified`, `Twilio` and `Signaller` alongside the two types the
 *    browser implements, and the WebIDL enum only accepts `password` and
 *    `oauth`. Passing the raw value makes `setConfiguration` throw a
 *    `TypeError`, which kills the whole peer connection.
 * 2. **Omit an empty `username`/`credential`.** The Rust struct defaults both
 *    to `""` and always serializes them, so a plain STUN server arrives as
 *    `{"urls":["stun:..."],"username":"","credential":""}`. An empty string is
 *    not the same as "absent" to every browser.
 *
 * An empty result is meaningful and must be applied: it tells the browser to
 * use its default (host candidates only), which is what makes a loopback or
 * same-LAN viewer work with no STUN at all.
 */
export const toRTCIceServers = (
  servers: Array<WireIceServer> | null | undefined,
): Array<MappedIceServer> =>
  (servers ?? []).flatMap((server): Array<MappedIceServer> => {
    const urls = Array.isArray(server.urls) ? server.urls : []
    const mapped: MappedIceServer = { urls }

    if (server.username) {
      mapped.username = server.username
    }
    if (server.credential) {
      mapped.credential = server.credential
    }
    if (server.credential_type === 'Password') {
      mapped.credentialType = 'password'
    } else if (server.credential_type === 'Oauth') {
      mapped.credentialType = 'oauth'
    }

    return [mapped]
  })

/**
 * Candidates the sharer sent before `setRemoteDescription` had resolved.
 *
 * A peer connection can be replaced (a second offer) or torn down (a lost
 * socket) between the moment a candidate is queued and the moment it can be
 * added, so `flush` puts back anything it could not add instead of dropping it
 * and reports both outcomes.
 */
export class IceCandidateQueue {
  private readonly candidates: Array<RTCIceCandidateInit> = []

  get size(): number {
    return this.candidates.length
  }

  /** Buffer a candidate for later. */
  push(candidate: RTCIceCandidateInit): void {
    this.candidates.push(candidate)
  }

  /** Forget everything: called when the peer connection is replaced. */
  reset(): void {
    this.candidates.length = 0
  }

  /**
   * Add every queued candidate, in arrival order. Returns how many were added
   * and how many had to be kept for a later flush.
   */
  async flush(
    pc: Pick<RTCPeerConnection, 'addIceCandidate'>,
  ): Promise<{ added: number; remaining: number }> {
    const pending = this.candidates.splice(0, this.candidates.length)
    let added = 0

    for (const candidate of pending) {
      try {
        await pc.addIceCandidate(candidate)
        added += 1
      } catch {
        // Keep it at the head of the rest: order still matters, and the next
        // flush (a retry, or the real remote description) can succeed.
        this.candidates.push(candidate)
      }
    }

    return { added, remaining: this.candidates.length }
  }
}

/**
 * `RTCIceCandidate.toJSON()` where it exists, and the plain init otherwise.
 * The serialized form is what the sharer's `RTCIceCandidateInit` expects.
 */
export const toWireCandidate = (
  candidate: RTCIceCandidate,
): RTCIceCandidateInit =>
  typeof candidate.toJSON === 'function' ? candidate.toJSON() : candidate
