// The viewer's signalling + WebRTC state machine, with no React in it.
//
// Keeping the socket, the `RTCPeerConnection` and the ICE bookkeeping out of
// the component makes every rule below unit-testable, and leaves the hook a
// thin wrapper that only owns React state.
//
// Faithful ports of the retired hand-written viewer (`webui/index.html`, in git
// history) are marked "retired viewer", because each one is a fix that cost
// real debugging time. `WEBUI.md` keeps the same checklist.

import { IceCandidateQueue, toRTCIceServers, toWireCandidate } from '#/lib/ice'
import type {
  OfferMessage,
  RemoteIceMessage,
  ServerMessage,
  ViewerOutbound,
} from '#/lib/protocol'
import {
  KEEP_ALIVE_INTERVAL_MS,
  VIEWER_NAME,
  declineReasonText,
} from '#/lib/protocol'

/**
 * `idle`             — nothing sent yet (the join form).
 * `connecting`       — socket dialling.
 * `awaitingApproval` — joined; the host has been asked.
 * `awaitingStream`   — offer accepted; media has not arrived.
 * `live`             — `ontrack` delivered a video track.
 * `lost`             — recoverable: socket dropped, offer Reconnect.
 * `failed`           — terminal: declined, or the room closed.
 */
export type SessionState =
  | 'idle'
  | 'connecting'
  | 'awaitingApproval'
  | 'awaitingStream'
  | 'live'
  | 'lost'
  | 'failed'

/** The two media surfaces. Either can arrive first. */
export type Streams = {
  video: MediaStream | null
  audio: MediaStream | null
}

export type SessionOptions = {
  room: string
  password: string
  viewerId: string
  signaller: string

  /** `debug` logs every signalling message to the console. */
  logLevel?: 'silent' | 'debug'

  onState: (state: SessionState) => void
  /** Status line under the heading; `''` hides it. */
  onMessage: (message: string) => void
  onStreams: (streams: Streams) => void
  /** Autoplay was refused; the page must offer a tap-to-enable-audio button. */
  onAudioBlocked: () => void
  /** Audio is playing; the tap-to-enable-audio button can go away. */
  onAudioPlaying: () => void
}

export type ViewerSession = {
  readonly viewerId: string
  connect: () => void
  /** Tear down and dial again: the Reconnect button. */
  reconnect: () => void
  /**
   * Leave the room and close everything. Sends `leave`, so the host drops the
   * viewer from `/admin` instead of keeping a ghost entry.
   */
  stop: () => void
  /** Re-run `audio.play()` from a user gesture. */
  enableAudio: () => void
}

export const initialState = (): SessionState => 'idle'

/** Terminal states never offer a Reconnect: there is nothing to retry. */
const isTerminal = (state: SessionState): boolean =>
  state === 'failed' || state === 'lost'

export function createViewerSession(options: SessionOptions): ViewerSession {
  const {
    room,
    password,
    viewerId,
    signaller,
    logLevel = 'silent',
    onState,
    onMessage,
    onStreams,
    onAudioBlocked,
    onAudioPlaying,
  } = options

  const debug = (message: string): void => {
    if (logLevel === 'debug') {
      console.debug('[viewer]', message)
    }
  }

  let state: SessionState = initialState()
  let socket: WebSocket | null = null
  let pc: RTCPeerConnection | null = null
  let keepAliveTimer: ReturnType<typeof setInterval> | null = null

  let streams: Streams = { video: null, audio: null }

  // Epochs guard the async edges. `socketEpoch` is bumped on every teardown, so
  // a socket that is no longer the current one cannot move the page's state;
  // `peerEpoch` does the same for the offer dance, whose `await`s can straddle a
  // reconnect.
  let socketEpoch = 0
  let peerEpoch = 0

  const iceQueue = new IceCandidateQueue()

  // The audio element lives in the DOM (see `src/routes/index.tsx`), so it is
  // reachable without threading a ref through this module.
  const audioElement = (): HTMLAudioElement | null =>
    typeof document === 'undefined'
      ? null
      : document.querySelector<HTMLAudioElement>('audio[data-viewer-audio]')

  const setState = (next: SessionState): void => {
    if (state === next) {
      return
    }
    state = next
    onState(next)
  }

  const setMessage = (text: string): void => {
    onMessage(text)
  }

  const send = (message: ViewerOutbound): boolean => {
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return false
    }
    socket.send(JSON.stringify(message))
    return true
  }

  const stopKeepAlive = (): void => {
    if (keepAliveTimer !== null) {
      clearInterval(keepAliveTimer)
      keepAliveTimer = null
    }
  }

  const closePeer = (): void => {
    const stale = pc
    pc = null
    if (stale) {
      // Detach before closing, so the state-change and track handlers of a peer
      // that is already gone cannot run; `peerEpoch` is the backstop for the
      // events that were in flight.
      stale.onicecandidate = null
      stale.ontrack = null
      stale.onconnectionstatechange = null
      stale.close()
    }
    streams = { video: null, audio: null }
    onStreams(streams)
  }

  /**
   * A terminal end: the room closed, or the host declined. Nothing to retry, so
   * no Reconnect is offered -- unlike a lost socket.
   */
  const fail = (text: string): void => {
    stopKeepAlive()
    closePeer()
    // The session is over: a candidate that never made it onto a peer is dead
    // weight, and a later `connect()` starts a new room.
    iceQueue.reset()
    socketEpoch += 1
    const closing = socket
    socket = null
    if (closing) {
      closing.onopen = null
      closing.onmessage = null
      closing.onclose = null
      closing.onerror = null
      closing.close()
    }
    setMessage(text)
    setState('failed')
  }

  /** A recoverable end: the socket went away. The page offers Reconnect. */
  const lose = (): void => {
    if (isTerminal(state)) {
      return
    }
    stopKeepAlive()
    closePeer()
    // Every candidate in the queue belonged to the peer that just went away.
    iceQueue.reset()
    socketEpoch += 1
    socket = null
    setMessage('Connection to the sharer was lost.')
    setState('lost')
  }

  const playAudio = (): void => {
    const element = audioElement()
    if (!element) {
      return
    }
    void element
      .play()
      .then(() => onAudioPlaying())
      .catch(() => {
        // Autoplay policy: the page shows a "click to enable audio" button. The
        // stream keeps flowing; only the sound waits for a gesture.
        onAudioBlocked()
      })
  }

  /** The offer dance. Mirrors the retired viewer step for step. */
  const startPeer = async (offer: OfferMessage): Promise<void> => {
    closePeer()
    // This offer's identity. Anything still in flight from an earlier offer
    // sees a different epoch and stops touching the page.
    //
    // Note what is *not* done here: resetting the ICE queue. Candidates that
    // arrived before this offer are precisely the ones the queue exists to
    // keep -- the sharer trickles them before the answer dance finishes.
    peerEpoch += 1
    const epoch = peerEpoch

    const peer = new RTCPeerConnection()
    pc = peer

    peer.setConfiguration({ iceServers: toRTCIceServers(offer.ice_servers) })

    peer.onicecandidate = (event) => {
      if (event.candidate && epoch === peerEpoch) {
        send({
          type: 'ice',
          ice: toWireCandidate(event.candidate),
          from: viewerId,
          to: room,
        })
      }
    }

    // `RTCTrackEvent` has no `kind` of its own -- the kind lives on the track.
    // Reading `event.kind` left both branches unreachable, so the video element
    // never got a source and the stage stayed black while frames decoded
    // perfectly on this side. Reveal the stage only for a *video* track.
    peer.ontrack = (event) => {
      if (epoch !== peerEpoch) {
        return
      }

      const kind = event.track.kind
      // Fall back to a stream built from the track alone: an answer can be
      // generated without an msid, and `event.streams[0]` is then undefined.
      const stream = event.streams[0] ?? new MediaStream([event.track])

      if (kind === 'video') {
        streams = { ...streams, video: stream }
        onStreams(streams)
        setMessage('')
        setState('live')
      } else if (kind === 'audio') {
        streams = { ...streams, audio: stream }
        onStreams(streams)
        playAudio()
      }
    }

    peer.onconnectionstatechange = () => {
      if (epoch !== peerEpoch) {
        return
      }
      if (
        peer.connectionState === 'failed' ||
        peer.connectionState === 'closed'
      ) {
        // ICE gave up while the socket is still up, so this is not a *lost*
        // connection: say what happened and let Reconnect redo the dance.
        stopKeepAlive()
        closePeer()
        setMessage('The video connection failed.')
        setState('lost')
      }
    }

    try {
      setMessage('Waiting for stream…')
      setState('awaitingStream')

      await peer.setRemoteDescription(offer.sdp)
      if (epoch !== peerEpoch) {
        return
      }

      // Candidates that beat the remote description can only be added now.
      await iceQueue.flush(peer)
      if (epoch !== peerEpoch) {
        return
      }

      const answer = await peer.createAnswer()
      if (epoch !== peerEpoch) {
        return
      }
      await peer.setLocalDescription(answer)
      if (epoch !== peerEpoch) {
        return
      }

      // `localDescription` carries the negotiated SDP; the retired viewer sent
      // exactly that object.
      send({
        type: 'answer',
        sdp: peer.localDescription ?? answer,
        from: viewerId,
        to: room,
      })
    } catch (error) {
      if (epoch !== peerEpoch) {
        return
      }
      closePeer()
      setMessage(
        `WebRTC error: ${
          error instanceof Error ? error.message : String(error)
        }`,
      )
      setState('lost')
    }
  }

  const handleIce = (message: RemoteIceMessage): void => {
    const candidate = message.ice

    if (!pc || !pc.remoteDescription) {
      iceQueue.push(candidate)
      return
    }

    void pc.addIceCandidate(candidate).catch((error: unknown) => {
      // A candidate can race the end of a session; never let a rejected one
      // take the page down. Keep it for the next flush instead of dropping it.
      debug(`addIceCandidate failed: ${String(error)}`)
      iceQueue.push(candidate)
    })
  }

  const handleMessage = (raw: string): void => {
    let message: ServerMessage
    try {
      message = JSON.parse(raw) as ServerMessage
    } catch {
      debug(`ignoring unparseable message: ${raw}`)
      return
    }

    debug(`recv ${message.type}`)

    switch (message.type) {
      case 'offer': {
        void startPeer(message)
        break
      }
      case 'ice': {
        handleIce(message)
        break
      }
      case 'join_declined': {
        fail(declineReasonText(message.reason))
        break
      }
      case 'room_closed': {
        fail('Session ended.')
        break
      }
      // start_response / ice_servers_response / keep_alive / leave: the viewer
      // does not act on them.
      default:
        break
    }
  }

  const open = (): void => {
    socketEpoch += 1
    const epoch = socketEpoch

    setMessage('Connecting…')
    setState('connecting')

    let ws: WebSocket
    try {
      ws = new WebSocket(signaller)
    } catch (error) {
      setMessage(
        `Could not reach the sharer: ${
          error instanceof Error ? error.message : String(error)
        }`,
      )
      setState('lost')
      return
    }
    socket = ws

    ws.onopen = () => {
      if (epoch !== socketEpoch) {
        return
      }
      setMessage('Waiting for approval…')
      setState('awaitingApproval')
      send({
        type: 'join',
        room,
        from: viewerId,
        name: VIEWER_NAME,
        auth: { type: 'password', password },
      })
      // The server reaps connections idle for more than 90 s
      // (`IDLE_TIMEOUT` in src/webui/mod.rs).
      stopKeepAlive()
      keepAliveTimer = setInterval(
        () => send({ type: 'keep_alive' }),
        KEEP_ALIVE_INTERVAL_MS,
      )
    }

    ws.onmessage = (event) => {
      if (epoch !== socketEpoch || typeof event.data !== 'string') {
        return
      }
      handleMessage(event.data)
    }

    ws.onclose = () => {
      if (epoch !== socketEpoch) {
        return
      }
      lose()
    }

    // `onclose` always follows, and it is the one that knows whether the page
    // was already terminal.
    ws.onerror = () => {}
  }

  const connect = (): void => {
    // Never dial twice: StrictMode double-invokes effects in development, and a
    // second `join` would register a second pending viewer for one tab.
    if (
      socket &&
      (socket.readyState === WebSocket.OPEN ||
        socket.readyState === WebSocket.CONNECTING)
    ) {
      debug('connect() ignored: a socket is already live')
      return
    }
    open()
  }

  const stop = (): void => {
    stopKeepAlive()
    if (socket && socket.readyState === WebSocket.OPEN) {
      send({ type: 'leave', from: viewerId })
    }
    socketEpoch += 1
    const closing = socket
    socket = null
    if (closing) {
      // Detach first: a deliberate close is not a lost connection.
      closing.onopen = null
      closing.onmessage = null
      closing.onclose = null
      closing.onerror = null
      closing.close()
    }
    closePeer()
    // Nothing is left to add a candidate to.
    iceQueue.reset()
  }

  return {
    viewerId,
    connect,
    reconnect: () => {
      stop()
      open()
    },
    stop,
    enableAudio: playAudio,
  }
}

/**
 * Wire the `beforeunload` teardown: `{type:"leave"}`, then close the socket and
 * the peer connection. A React effect cleanup does **not** run when the tab
 * closes, so without this every closed tab leaves a viewer on `/admin` until
 * the host kicks a ghost.
 *
 * Returns the remover, so the effect can detach it on a real unmount.
 */
export const attachUnloadTeardown = (
  session: ViewerSession,
  target: Pick<Window, 'addEventListener' | 'removeEventListener'>,
): (() => void) => {
  const onUnload = (): void => session.stop()
  target.addEventListener('beforeunload', onUnload)
  return () => target.removeEventListener('beforeunload', onUnload)
}
