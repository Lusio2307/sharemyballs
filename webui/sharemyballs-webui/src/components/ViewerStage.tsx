import { Box, Button, HStack, Text, VStack } from '@chakra-ui/react'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { SessionState, Streams } from '#/lib/viewerSession'
import {
  canUseFullscreenApi,
  enterFullscreen,
  exitFullscreen,
  fullscreenElement,
} from '#/lib/fullscreen'

type ViewerStageProps = {
  state: SessionState
  message: string
  streams: Streams
  /** An audio track exists but `play()` was refused: offer the gesture. */
  audioBlocked: boolean
  onEnableAudio: () => void
  /** The gesture worked; the affordance can go away. */
  onAudioPlaying: () => void
  onReconnect: () => void
  canReconnect: boolean
}

const STATUS_TEXT: Partial<Record<SessionState, string>> = {
  connecting: 'Connecting…',
  awaitingApproval: 'Waiting for approval…',
  awaitingStream: 'Starting…',
}

/**
 * The media surface: one always-mounted `<video>`/`<audio>` pair inside a
 * letterboxed stage, with a status overlay while there is no picture.
 *
 * The elements stay mounted for the whole session on purpose. Attaching a
 * stream to a freshly mounted element is a pause away from a black frame, and
 * an `<audio>` that appears only once the stream does would ask for a *new*
 * autoplay gesture.
 */
export function ViewerStage({
  state,
  message,
  streams,
  audioBlocked,
  onEnableAudio,
  onAudioPlaying,
  onReconnect,
  canReconnect,
}: ViewerStageProps) {
  // `container` is what goes fullscreen, so the controls stay reachable inside
  // it; the button row below is inside it too.
  const containerRef = useRef<HTMLDivElement>(null)
  const videoRef = useRef<HTMLVideoElement>(null)
  const audioRef = useRef<HTMLAudioElement>(null)

  const [isFullscreen, setIsFullscreen] = useState(false)
  // Fallback for engines with no element fullscreen (iOS Safari only
  // fullscreens <video>): emulate it with a fixed overlay instead of leaving
  // the button dead.
  const [isPseudoFullscreen, setIsPseudoFullscreen] = useState(false)
  // Bumped by "click to enable audio". `play()` is retried whenever it changes,
  // which is what turns the gesture into sound; the element itself never
  // changes, so nothing has to be re-attached.
  const [audioAttempts, setAudioAttempts] = useState(0)

  // Feed the tracks in. `ontrack` fires for video and audio separately, so
  // this runs twice and must not assume the other stream exists yet.
  useEffect(() => {
    const video = videoRef.current
    if (!video) {
      return
    }

    video.srcObject = streams.video
    if (streams.video) {
      // Muted video autoplays everywhere; the audio track is the one the
      // autoplay policy gates, and it gets its own tap-to-hear affordance.
      video.muted = true
      void video.play().catch(() => {
        // Nothing to recover: the element is muted, so a rejection here means
        // there is no decodable picture yet, not that a gesture is missing.
      })
    }
  }, [streams.video])

  useEffect(() => {
    const audio = audioRef.current
    if (!audio) {
      return
    }

    audio.srcObject = streams.audio
    if (!audioBlocked || streams.audio === null) {
      return
    }

    // The session already tried once and lost the autoplay race. This re-arms
    // on every tap as well as on the stream arriving; `onAudioPlaying` is what
    // takes the button away again.
    void audio
      .play()
      .then(() => onAudioPlaying())
      .catch(() => {
        // Still refused. The stream is untouched, so the next tap retries.
      })
  }, [audioBlocked, audioAttempts, streams.audio, onAudioPlaying])

  useEffect(() => {
    const onChange = () => {
      setIsFullscreen(fullscreenElement() === containerRef.current)
    }

    document.addEventListener('fullscreenchange', onChange)
    document.addEventListener('webkitfullscreenchange', onChange)

    return () => {
      document.removeEventListener('fullscreenchange', onChange)
      document.removeEventListener('webkitfullscreenchange', onChange)
    }
  }, [])

  // Escape exits API-driven fullscreen by itself, but not the fallback.
  useEffect(() => {
    if (!isPseudoFullscreen) {
      return
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setIsPseudoFullscreen(false)
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [isPseudoFullscreen])

  const toggleFullscreen = useCallback(async () => {
    const container = containerRef.current
    if (!container) {
      return
    }

    if (isPseudoFullscreen) {
      setIsPseudoFullscreen(false)
      return
    }

    if (!canUseFullscreenApi(container)) {
      setIsPseudoFullscreen(true)
      return
    }

    try {
      if (fullscreenElement()) {
        await exitFullscreen()
      } else {
        await enterFullscreen(container)
      }
    } catch {
      setIsPseudoFullscreen(true)
    }
  }, [isPseudoFullscreen])

  const fullscreenActive = isFullscreen || isPseudoFullscreen
  const live = state === 'live' && streams.video !== null
  // When live, `message` is already `''` (the session clears it on the video
  // track), so "Live" is the fallback rather than the message.
  const status = message || STATUS_TEXT[state] || (live ? 'Live' : '')

  return (
    <Box
      ref={containerRef}
      h="100%"
      w="100%"
      bg="black"
      display="flex"
      flexDirection="column"
      position={isPseudoFullscreen ? 'fixed' : 'relative'}
      inset={isPseudoFullscreen ? 0 : undefined}
      zIndex={isPseudoFullscreen ? 10 : undefined}
    >
      <Box
        position="relative"
        flex="1"
        minH="0"
        bg="black"
        display="flex"
        alignItems="center"
        justifyContent="center"
        overflow="hidden"
      >
        <video
          ref={videoRef}
          autoPlay
          playsInline
          muted
          style={{
            display: live ? 'block' : 'none',
            width: '100%',
            height: '100%',
            objectFit: 'contain',
          }}
        />

        {/* Hidden, not unmounted: a mounted element is what the session's
            `play()` and the tap-to-enable gesture act on. */}
        <audio ref={audioRef} autoPlay data-viewer-audio hidden />

        {live ? null : (
          <Box position="absolute" inset="0" p="6">
            <VStack
              h="100%"
              justify="center"
              align="center"
              gap="3"
              textAlign="center"
            >
              <Text fontSize="lg" fontWeight="semibold">
                Mira Sharer
              </Text>
              {status ? (
                <Text
                  fontSize="md"
                  color={state === 'lost' ? 'fg.error' : 'fg.muted'}
                >
                  {status}
                </Text>
              ) : null}
              {state === 'awaitingApproval' ? (
                <Text fontSize="sm" color="fg.subtle">
                  The person sharing has to accept you.
                </Text>
              ) : null}
              {state === 'lost' ? (
                <Button colorPalette="blue" onClick={() => void onReconnect()}>
                  Reconnect
                </Button>
              ) : null}
            </VStack>
          </Box>
        )}
      </Box>

      <HStack
        justify="center"
        py="3"
        px="4"
        gap="3"
        flexShrink="0"
        flexWrap="wrap"
      >
        {audioBlocked ? (
          <Button
            colorPalette="green"
            onClick={() => {
              setAudioAttempts((attempts) => attempts + 1)
              onEnableAudio()
            }}
          >
            Click to enable audio
          </Button>
        ) : null}
        {live && canReconnect ? (
          <Button variant="outline" onClick={onReconnect}>
            Reconnect
          </Button>
        ) : null}
        <Button onClick={() => void toggleFullscreen()} colorPalette="blue">
          {fullscreenActive ? 'Exit fullscreen' : 'Fullscreen'}
        </Button>
      </HStack>
    </Box>
  )
}
