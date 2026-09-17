import { Box, Button, Heading, Input, Text, VStack } from '@chakra-ui/react'
import { createFileRoute } from '@tanstack/react-router'
import { useCallback, useMemo, useState } from 'react'
import { ViewerStage } from '#/components/ViewerStage'
import type { ViewerSessionConfig } from '#/hooks/useViewerSession'
import { useViewerSession } from '#/hooks/useViewerSession'
import { signallerUrl } from '#/lib/protocol'
import { makeViewerId } from '#/lib/uuid'

export const Route = createFileRoute('/')({ component: ViewerPage })

// One id per *page load*, at module scope on purpose.
//
// `crypto.randomUUID()` is deliberately not used -- see `src/lib/uuid.ts`. And
// because a per-render id would re-join the room on every render, and a
// per-effect id would send two `join`s under React 19's StrictMode double
// invocation, this is the only place the id is minted.
const VIEWER_ID = makeViewerId()

/**
 * The invite URL contract is fixed by the Rust side: `Capturer::get_invite_link`
 * emits `{public_url or http://127.0.0.1:<port>/}?room=<room>&pwd=<password>`.
 * `signaller` is an optional development override; without it the socket URL is
 * derived from the page's own origin, which is what makes the
 * `webui.public_url` / reverse-proxy path work.
 */
function readInvite(search: string) {
  const params = new URLSearchParams(search)
  return {
    room: params.get('room')?.trim() ?? '',
    password: params.get('pwd') ?? '',
    signaller: params.get('signaller'),
  }
}

function ViewerPage() {
  const invite = useMemo(() => readInvite(window.location.search), [])

  // Present when the URL carries no invite: the same fallback the retired page
  // had, so a bare `http://host:8765/` is still usable.
  const [room, setRoom] = useState(invite.room)
  const [password, setPassword] = useState(invite.password)
  const [config, setConfig] = useState<ViewerSessionConfig | null>(() =>
    invite.room && invite.password
      ? {
          room: invite.room,
          password: invite.password,
          viewerId: VIEWER_ID,
          signaller: signallerUrl(window.location, invite.signaller),
        }
      : null,
  )

  const session = useViewerSession(config)

  const join = useCallback(() => {
    const trimmedRoom = room.trim()
    if (!trimmedRoom || !password) {
      return
    }
    setConfig({
      room: trimmedRoom,
      password,
      viewerId: VIEWER_ID,
      signaller: signallerUrl(window.location, invite.signaller),
    })
  }, [room, password, invite.signaller])

  if (!config) {
    return (
      <Box
        h="100%"
        display="flex"
        alignItems="center"
        justifyContent="center"
        p="6"
      >
        <form
          onSubmit={(event) => {
            event.preventDefault()
            join()
          }}
        >
          <VStack gap="4" align="stretch" minW={{ base: '260px', sm: '320px' }}>
            <Heading size="lg">Mira Sharer</Heading>
            <Text fontSize="sm" color="fg.muted">
              Paste the invite link, or type the room and passcode from it.
            </Text>
            <Box>
              <Text fontSize="xs" color="fg.muted" mb="1">
                Room
              </Text>
              <Input
                autoFocus
                autoComplete="off"
                spellCheck={false}
                placeholder="Room"
                value={room}
                onChange={(event) => setRoom(event.target.value)}
              />
            </Box>
            <Box>
              <Text fontSize="xs" color="fg.muted" mb="1">
                Passcode
              </Text>
              <Input
                autoComplete="off"
                placeholder="Passcode"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
              />
            </Box>
            <Button
              type="submit"
              colorPalette="blue"
              disabled={!room.trim() || !password}
            >
              Join
            </Button>
          </VStack>
        </form>
      </Box>
    )
  }

  return (
    <ViewerStage
      state={session.state}
      message={session.message}
      streams={session.streams}
      audioBlocked={session.audioBlocked}
      onEnableAudio={session.enableAudio}
      onAudioPlaying={session.audioStarted}
      onReconnect={session.reconnect}
      canReconnect={session.canReconnect}
    />
  )
}
