import { describe, expect, it } from 'vitest'
import type { WireIceServer } from '#/lib/protocol'
import { IceCandidateQueue, toRTCIceServers } from '#/lib/ice'

const server = (overrides: Partial<WireIceServer> = {}): WireIceServer => ({
  urls: ['stun:stun.l.google.com:19302'],
  username: '',
  credential: '',
  ...overrides,
})

describe('toRTCIceServers', () => {
  // The Rust enum (`IceCredentialType` in src/config.rs) serializes
  // `Unspecified` and `Twilio`/`Signaller` alongside the two values the
  // browser's WebIDL enum accepts. Passing any of them through makes
  // `setConfiguration` throw a TypeError, which takes the whole peer
  // connection down before a single frame arrives.
  it('omits credentialType for Unspecified and anything unknown', () => {
    const [unspecified] = toRTCIceServers([
      server({ credential_type: 'Unspecified' }),
    ])
    expect(unspecified).toEqual({ urls: ['stun:stun.l.google.com:19302'] })
    expect('credentialType' in unspecified).toBe(false)

    const [missing] = toRTCIceServers([server()])
    expect('credentialType' in missing).toBe(false)
  })

  it('maps only Password and Oauth onto the browser enum', () => {
    expect(
      toRTCIceServers([server({ credential_type: 'Password' })])[0]
        .credentialType,
    ).toBe('password')
    expect(
      toRTCIceServers([server({ credential_type: 'Oauth' })])[0].credentialType,
    ).toBe('oauth')
  })

  it('never forwards a type the browser would reject', () => {
    for (const credential_type of ['Twilio', 'Signaller'] as const) {
      const [mapped] = toRTCIceServers([server({ credential_type })])
      expect(mapped.credentialType).toBeUndefined()
    }
  })

  it('drops empty credentials rather than forwarding ""', () => {
    // The Rust struct defaults both to `""` and always serializes them, so a
    // bare STUN server arrives with empty strings.
    const [mapped] = toRTCIceServers([server({ credential_type: 'Password' })])
    expect('username' in mapped).toBe(false)
    expect('credential' in mapped).toBe(false)
  })

  it('keeps a real username and credential', () => {
    const [mapped] = toRTCIceServers([
      server({
        urls: ['turn:turn.example.com:3478'],
        username: 'user',
        credential: 'secret',
        credential_type: 'Password',
      }),
    ])

    expect(mapped).toEqual({
      urls: ['turn:turn.example.com:3478'],
      username: 'user',
      credential: 'secret',
      credentialType: 'password',
    })
  })

  it('treats a missing offer list as no ICE servers', () => {
    expect(toRTCIceServers(undefined)).toEqual([])
    expect(toRTCIceServers(null)).toEqual([])
  })
})

describe('IceCandidateQueue', () => {
  const candidate = (value: string) =>
    ({ candidate: value, sdpMid: '0', sdpMLineIndex: 0 }) as RTCIceCandidateInit

  it('flushes in arrival order, after the remote description lands', async () => {
    const queue = new IceCandidateQueue()
    const added: Array<string | undefined> = []
    const pc = {
      addIceCandidate: (value: RTCIceCandidateInit) => {
        added.push(value.candidate)
        return Promise.resolve()
      },
    }

    // The sharer trickles ICE before its offer is fully applied.
    queue.push(candidate('a'))
    queue.push(candidate('b'))
    expect(queue.size).toBe(2)
    expect(added).toEqual([])

    const result = await queue.flush(pc)

    expect(added).toEqual(['a', 'b'])
    expect(result).toEqual({ added: 2, remaining: 0 })
    expect(queue.size).toBe(0)
  })

  it('keeps a candidate it could not add, so a retry can still use it', async () => {
    const queue = new IceCandidateQueue()
    let attempts = 0
    const pc = {
      addIceCandidate: () => {
        attempts += 1
        return attempts === 1
          ? Promise.reject(new Error('no remote description yet'))
          : Promise.resolve()
      },
    }

    queue.push(candidate('a'))
    expect(await queue.flush(pc)).toEqual({ added: 0, remaining: 1 })

    // A later flush is the retry: order is preserved, nothing is lost.
    expect(await queue.flush(pc)).toEqual({ added: 1, remaining: 0 })
  })

  it('forgets everything on reset', () => {
    const queue = new IceCandidateQueue()
    queue.push(candidate('a'))
    queue.reset()
    expect(queue.size).toBe(0)
  })
})
