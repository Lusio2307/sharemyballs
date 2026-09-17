import { useCallback, useEffect, useRef, useState } from 'react'
import { attachUnloadTeardown, createViewerSession } from '#/lib/viewerSession'
import type {
  SessionOptions,
  SessionState,
  Streams,
  ViewerSession,
} from '#/lib/viewerSession'

export type ViewerSessionConfig = {
  room: string
  password: string
  /**
   * One id per page load. Generating it at module scope (or here, via `useRef`)
   * matters because React 19's StrictMode double-invokes effects in
   * development, and a per-connect id would register two pending viewers for a
   * single tab.
   */
  viewerId: string
  signaller: string
}

export type ViewerSessionHandle = {
  state: SessionState
  message: string
  streams: Streams
  /** Autoplay refused the audio element; the page must offer a gesture. */
  audioBlocked: boolean
  /** Clear `audioBlocked` once a gesture really started playback. */
  audioStarted: () => void
  /** False while idle or after a terminal failure, where Retry means nothing. */
  canReconnect: boolean
  reconnect: () => void
  enableAudio: () => void
}

/**
 * React binding for `createViewerSession`.
 *
 * The session is built once per `config` change and lives outside React: the
 * socket and the peer connection are not render state, and re-creating them on
 * every render would re-join the room. The effect that creates it tears it down
 * on unmount, while the `beforeunload` handler covers the tab-close case a
 * cleanup never sees.
 */
export const useViewerSession = (
  config: ViewerSessionConfig | null,
): ViewerSessionHandle => {
  const [state, setState] = useState<SessionState>('idle')
  const [message, setMessage] = useState('')
  const [streams, setStreams] = useState<Streams>({
    video: null,
    audio: null,
  })
  const [audioBlocked, setAudioBlocked] = useState(false)

  const sessionRef = useRef<ViewerSession | null>(null)

  const room = config?.room ?? ''
  const password = config?.password ?? ''
  const viewerId = config?.viewerId ?? ''
  const signaller = config?.signaller ?? ''
  const enabled = config !== null

  useEffect(() => {
    if (!enabled) {
      return
    }

    const options: SessionOptions = {
      room,
      password,
      viewerId,
      signaller,
      onState: setState,
      onMessage: setMessage,
      onStreams: setStreams,
      onAudioBlocked: () => setAudioBlocked(true),
      onAudioPlaying: () => setAudioBlocked(false),
    }

    const session = createViewerSession(options)
    sessionRef.current = session

    const detachUnload = attachUnloadTeardown(session, window)
    session.connect()

    return () => {
      detachUnload()
      session.stop()
      sessionRef.current = null
    }
  }, [enabled, room, password, viewerId, signaller])

  const reconnect = useCallback(() => {
    setAudioBlocked(false)
    sessionRef.current?.reconnect()
  }, [])

  // Deliberately *not* optimistic: the flag clears only when `play()` really
  // resolves, so a refused gesture leaves the button up instead of stranding
  // the user with no way to hear the stream.
  const enableAudio = useCallback(() => {
    sessionRef.current?.enableAudio()
  }, [])

  const audioStarted = useCallback(() => setAudioBlocked(false), [])

  return {
    state,
    message,
    streams,
    audioBlocked,
    audioStarted,
    canReconnect: state !== 'idle' && state !== 'failed',
    reconnect,
    enableAudio,
  }
}
