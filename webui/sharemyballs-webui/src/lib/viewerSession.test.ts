import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SessionState, Streams } from '#/lib/viewerSession'
import { attachUnloadTeardown, createViewerSession } from '#/lib/viewerSession'

// ---------------------------------------------------------------------------
// Doubles for the browser APIs the session touches. A real `RTCPeerConnection`
// is deliberately not tested (jsdom has none, and a fake of the SDP engine
// would only assert that the fake was called): what is tested is the
// *bookkeeping* around it -- what is sent, in what order, and what the page is
// told. `src/test-setup.ts` fills in the media-element APIs jsdom lacks.
// ---------------------------------------------------------------------------

const OFFER_SDP = {
  type: 'offer' as const,
  sdp: 'v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\n',
}

type Sent = Record<string, unknown>

class FakeWebSocket {
  static instances: Array<FakeWebSocket> = []
  static readonly CONNECTING = 0
  static readonly OPEN = 1

  readonly url: string
  readyState = FakeWebSocket.CONNECTING
  sent: Array<Sent> = []

  onopen: (() => void) | null = null
  onmessage: ((event: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null

  constructor(url: string) {
    this.url = url
    FakeWebSocket.instances.push(this)
  }

  send(data: string): void {
    this.sent.push(JSON.parse(data) as Sent)
  }

  close(): void {
    this.readyState = 3
  }

  /** Test-side: the socket actually connects. */
  confirm(): void {
    this.readyState = FakeWebSocket.OPEN
    this.onopen?.()
  }

  /** Test-side: one JSON message arrives from the signaller. */
  receive(message: Sent): void {
    this.onmessage?.({ data: JSON.stringify(message) })
  }
}

type FakeTrackEvent = { streams: Array<unknown>; track: { kind: string } }

class FakePeer {
  static instances: Array<FakePeer> = []

  configuration: RTCConfiguration | null = null
  remoteDescription: unknown = null
  localDescription: unknown = null
  added: Array<RTCIceCandidateInit> = []
  closed = false

  onicecandidate: ((event: { candidate: RTCIceCandidate }) => void) | null =
    null
  ontrack: ((event: FakeTrackEvent) => void) | null = null
  onconnectionstatechange: (() => void) | null = null

  constructor() {
    FakePeer.instances.push(this)
  }

  setConfiguration(configuration: RTCConfiguration): void {
    this.configuration = configuration
  }

  addIceCandidate(candidate: RTCIceCandidateInit): Promise<void> {
    this.added.push(candidate)
    return Promise.resolve()
  }

  setRemoteDescription(description: unknown): Promise<void> {
    this.remoteDescription = description
    return Promise.resolve()
  }

  createAnswer(): Promise<{ type: 'answer'; sdp: string }> {
    return Promise.resolve({ type: 'answer', sdp: 'v=0\r\n' })
  }

  setLocalDescription(description: unknown): Promise<void> {
    this.localDescription = description
    return Promise.resolve()
  }

  close(): void {
    this.closed = true
  }
}

class FakeMediaStream {
  readonly tracks: Array<unknown>

  constructor(tracks: Array<unknown> = []) {
    this.tracks = tracks
  }
}

let states: Array<SessionState>
let messages: Array<string>
let streamEvents: Array<Streams>
let audioBlocked: number

const newSession = () =>
  createViewerSession({
    room: 'desk',
    password: 'pass',
    viewerId: 'viewer-1',
    signaller: 'ws://127.0.0.1:8765/signaller',
    onState: (state) => states.push(state),
    onMessage: (message) => messages.push(message),
    onStreams: (streams) => streamEvents.push(streams),
    onAudioBlocked: () => {
      audioBlocked += 1
    },
    onAudioPlaying: () => {},
  })

/** Let every already-resolved promise in the offer dance run. */
const drain = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

const latestSocket = (): FakeWebSocket => {
  const socket = FakeWebSocket.instances.at(-1)
  if (!socket) {
    throw new Error('no WebSocket was constructed')
  }
  return socket
}

const latestPeer = (): FakePeer => {
  const peer = FakePeer.instances.at(-1)
  if (!peer) {
    throw new Error('no RTCPeerConnection was constructed')
  }
  return peer
}

const offer = () =>
  latestSocket().receive({
    type: 'offer',
    sdp: OFFER_SDP,
    from: 'desk',
    to: 'viewer-1',
    ice_servers: [],
  })

beforeEach(() => {
  FakeWebSocket.instances = []
  FakePeer.instances = []
  states = []
  messages = []
  streamEvents = []
  audioBlocked = 0

  vi.stubGlobal('WebSocket', FakeWebSocket)
  vi.stubGlobal('RTCPeerConnection', FakePeer)
  vi.stubGlobal('MediaStream', FakeMediaStream)

  // What `ViewerStage` renders for the whole session.
  document.body.innerHTML =
    '<video autoplay playsinline muted></video><audio data-viewer-audio hidden></audio>'
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
  document.body.innerHTML = ''
})

describe('createViewerSession', () => {
  it('joins the room and starts the keep-alive once the socket opens', () => {
    const session = newSession()
    session.connect()

    expect(FakeWebSocket.instances).toHaveLength(1)
    expect(latestSocket().url).toBe('ws://127.0.0.1:8765/signaller')
    expect(states).toEqual(['connecting'])
    expect(latestSocket().sent).toEqual([])

    latestSocket().confirm()

    expect(latestSocket().sent).toContainEqual({
      type: 'join',
      room: 'desk',
      from: 'viewer-1',
      name: 'WebUI',
      auth: { type: 'password', password: 'pass' },
    })
    expect(messages.at(-1)).toBe('Waiting for approval…')
    expect(states).toEqual(['connecting', 'awaitingApproval'])
  })

  it('keeps the connection alive every 30 s', () => {
    vi.useFakeTimers()
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    vi.advanceTimersByTime(30_000)
    expect(
      latestSocket().sent.filter((m) => m.type === 'keep_alive'),
    ).toHaveLength(1)

    vi.advanceTimersByTime(60_000)
    expect(
      latestSocket().sent.filter((m) => m.type === 'keep_alive'),
    ).toHaveLength(3)
  })

  // StrictMode double-invokes effects in development. A second `join` would
  // register a second pending viewer for one tab -- and then leave a ghost.
  it('is idempotent while a socket is live', () => {
    const session = newSession()
    session.connect()
    session.connect()
    session.connect()

    expect(FakeWebSocket.instances).toHaveLength(1)
  })

  it('accepts the offer, answers it, and applies the ICE servers', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    latestSocket().receive({
      type: 'offer',
      sdp: OFFER_SDP,
      from: 'desk',
      to: 'viewer-1',
      ice_servers: [
        {
          urls: ['stun:stun.l.google.com:19302'],
          username: '',
          credential: '',
          credential_type: 'Unspecified',
        },
      ],
    })
    await drain()

    expect(latestPeer().configuration).toEqual({
      iceServers: [{ urls: ['stun:stun.l.google.com:19302'] }],
    })
    expect(latestPeer().remoteDescription).toEqual(OFFER_SDP)
    expect(latestSocket().sent).toContainEqual({
      type: 'answer',
      sdp: { type: 'answer', sdp: 'v=0\r\n' },
      from: 'viewer-1',
      to: 'desk',
    })
    expect(states.at(-1)).toBe('awaitingStream')
  })

  it('applies an empty ICE list instead of falling back to a default', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    offer()
    await drain()

    expect(latestPeer().configuration).toEqual({ iceServers: [] })
  })

  // The exact bug the retired page fixed: ICE arriving before the offer is
  // applied cannot be added, and used to be dropped on the floor.
  it('queues ICE that arrives before the remote description', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    latestSocket().receive({
      type: 'ice',
      ice: { candidate: 'early' },
      from: 'desk',
      to: 'viewer-1',
    })
    expect(FakePeer.instances).toHaveLength(0)

    offer()
    await drain()

    expect(latestPeer().added).toEqual([{ candidate: 'early' }])
  })

  it('adds ICE directly once the remote description is set', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    offer()
    await drain()

    latestSocket().receive({
      type: 'ice',
      ice: { candidate: 'late' },
      from: 'desk',
      to: 'viewer-1',
    })
    await drain()

    expect(latestPeer().added).toEqual([{ candidate: 'late' }])
  })

  it('trickles its own candidates back to the room', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    offer()
    await drain()

    latestPeer().onicecandidate?.({
      candidate: {
        toJSON: () => ({ candidate: 'local-candidate', sdpMid: '0' }),
      } as unknown as RTCIceCandidate,
    })

    expect(latestSocket().sent).toContainEqual({
      type: 'ice',
      ice: { candidate: 'local-candidate', sdpMid: '0' },
      from: 'viewer-1',
      to: 'desk',
    })
  })

  it('attaches a video track and reports live', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    offer()
    await drain()

    const track = { kind: 'video' }
    const stream = new FakeMediaStream([track])
    latestPeer().ontrack?.({ streams: [stream], track })

    expect(states.at(-1)).toBe('live')
    expect(messages.at(-1)).toBe('')
    expect(streamEvents.at(-1)?.video).toBe(stream)
  })

  // `RTCTrackEvent` carries no `kind` of its own. Branching on `event.kind`
  // left both branches unreachable and produced a black stage while the frames
  // decoded perfectly -- so the *audio* path must not claim to be live.
  it('does not reveal the stage for an audio-only track', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    offer()
    await drain()

    const track = { kind: 'audio' }
    const stream = new FakeMediaStream([track])
    latestPeer().ontrack?.({ streams: [stream], track })

    expect(states).not.toContain('live')
    expect(streamEvents.at(-1)?.audio).toBe(stream)
    expect(streamEvents.at(-1)?.video).toBeNull()
  })

  it('falls back to a stream built from the track when there is no msid', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    offer()
    await drain()

    const track = { kind: 'video' }
    latestPeer().ontrack?.({ streams: [], track })

    expect(states.at(-1)).toBe('live')
    expect(streamEvents.at(-1)?.video).toBeInstanceOf(FakeMediaStream)
  })

  it('plays the audio element as soon as the track arrives', async () => {
    const session = newSession()
    const element = document.querySelector('audio')
    if (!element) {
      throw new Error('the audio element must be in the document')
    }
    const play = vi
      .spyOn(element, 'play')
      .mockImplementation(() => Promise.resolve())

    session.connect()
    latestSocket().confirm()

    offer()
    await drain()

    const track = { kind: 'audio' }
    const stream = new FakeMediaStream([track])
    latestPeer().ontrack?.({ streams: [stream], track })
    await drain()

    expect(streamEvents.at(-1)?.audio).toBe(stream)
    expect(play).toHaveBeenCalled()
    // Autoplay was allowed, so no gesture affordance is needed.
    expect(audioBlocked).toBe(0)
  })

  it('reports a refused autoplay instead of failing silently', async () => {
    const session = newSession()
    const element = document.querySelector('audio')
    if (!element) {
      throw new Error('the audio element must be in the document')
    }
    vi.spyOn(element, 'play').mockImplementation(() =>
      Promise.reject(new DOMException('blocked')),
    )

    session.connect()
    latestSocket().confirm()

    offer()
    await drain()

    const track = { kind: 'audio' }
    latestPeer().ontrack?.({ streams: [new FakeMediaStream([track])], track })
    await drain()

    expect(audioBlocked).toBe(1)
  })

  it('maps a host decline to a terminal message', () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    latestSocket().receive({
      type: 'join_declined',
      reason: 'UserDeclined',
      to: 'viewer-1',
    })

    expect(messages.at(-1)).toBe('Declined by host.')
    expect(states.at(-1)).toBe('failed')
  })

  it('maps a wrong passcode to its own message', () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    latestSocket().receive({
      type: 'join_declined',
      reason: 'IncorrectPassword',
      to: 'viewer-1',
    })

    expect(messages.at(-1)).toBe('Incorrect passcode.')
    expect(states.at(-1)).toBe('failed')
  })

  it('ends the session when the room closes', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    offer()
    await drain()
    const peer = latestPeer()

    latestSocket().receive({
      type: 'room_closed',
      to: 'viewer-1',
      room: 'desk',
    })

    expect(messages.at(-1)).toBe('Session ended.')
    expect(states.at(-1)).toBe('failed')
    expect(peer.closed).toBe(true)
    expect(streamEvents.at(-1)).toEqual({ video: null, audio: null })
  })

  // A dropped socket is *recoverable*: the page must offer Reconnect, unlike a
  // decline or a closed room.
  it('reports a lost socket as recoverable, then reconnects', () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()
    latestSocket().onclose?.()

    expect(messages.at(-1)).toBe('Connection to the sharer was lost.')
    expect(states.at(-1)).toBe('lost')

    session.reconnect()

    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(states.at(-1)).toBe('connecting')
  })

  it('ignores a close that belongs to a socket it already replaced', () => {
    const session = newSession()
    session.connect()
    const stale = latestSocket()
    stale.confirm()

    session.reconnect()
    stale.onclose?.()

    expect(states.at(-1)).toBe('connecting')
    expect(messages.at(-1)).toBe('Connecting…')
  })

  it('leaves the room on stop', () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()
    const socket = latestSocket()

    session.stop()

    expect(socket.sent.at(-1)).toEqual({ type: 'leave', from: 'viewer-1' })
    expect(socket.readyState).not.toBe(FakeWebSocket.OPEN)
  })

  it('does not treat a deliberate stop as a lost connection', () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()
    const socket = latestSocket()

    session.stop()
    socket.onclose?.()

    expect(states).not.toContain('lost')
  })

  it('ignores a malformed message instead of failing the session', () => {
    const session = newSession()
    session.connect()
    const socket = latestSocket()
    socket.confirm()

    socket.onmessage?.({ data: 'not json' })

    expect(states.at(-1)).toBe('awaitingApproval')
  })

  it('reports a WebRTC failure when the offer cannot be applied', async () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()

    // `setRemoteDescription` is the first await of the dance; make it fail.
    const original = FakePeer.prototype.setRemoteDescription
    FakePeer.prototype.setRemoteDescription = () =>
      Promise.reject(new Error('invalid SDP'))
    try {
      offer()
      await drain()
    } finally {
      FakePeer.prototype.setRemoteDescription = original
    }

    expect(messages.at(-1)).toBe('WebRTC error: invalid SDP')
    expect(states.at(-1)).toBe('lost')
  })
})

describe('attachUnloadTeardown', () => {
  // A React effect cleanup does not run when the tab closes, so the `leave` has
  // to be sent from `beforeunload`; without it every closed tab leaves a ghost
  // on /admin until the host kicks it.
  it('stops the session on beforeunload and detaches afterwards', () => {
    const session = newSession()
    session.connect()
    latestSocket().confirm()
    const socket = latestSocket()

    const listeners = new Map<string, () => void>()
    const target = {
      addEventListener: (name: string, handler: () => void) => {
        listeners.set(name, handler)
      },
      removeEventListener: (name: string) => {
        listeners.delete(name)
      },
    }

    const detach = attachUnloadTeardown(session, target as unknown as Window)
    listeners.get('beforeunload')?.()

    expect(socket.sent.at(-1)).toEqual({ type: 'leave', from: 'viewer-1' })

    detach()
    expect(listeners.has('beforeunload')).toBe(false)
  })
})
