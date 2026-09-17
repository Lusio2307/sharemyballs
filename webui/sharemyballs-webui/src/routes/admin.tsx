import {
  Badge,
  Box,
  Button,
  HStack,
  Input,
  Text,
  VStack,
} from '@chakra-ui/react'
import { createFileRoute } from '@tanstack/react-router'
import { useCallback, useEffect, useState } from 'react'
import type { AdminState, Viewer } from '#/lib/adminApi'
import {
  UnauthorizedError,
  acceptViewer,
  declineViewer,
  fetchState,
  kickViewer,
  startSession,
  stopSession,
} from '#/lib/adminApi'

export const Route = createFileRoute('/admin')({ component: AdminPage })

// Kept in sessionStorage so a refresh does not lose the login, but closing the
// tab does. The secret itself lives only on the server (`webui.admin_password`).
const TOKEN_KEY = 'mira.admin.token'

// The server has no push channel yet, so the page polls (roadmap M2B.3).
const POLL_INTERVAL_MS = 2000

function readToken(): string | null {
  try {
    return sessionStorage.getItem(TOKEN_KEY)
  } catch {
    return null
  }
}

/**
 * `navigator.clipboard` only exists in a secure context, and this page is
 * normally reached over plain HTTP on the LAN, so fall back to the old
 * `execCommand` route rather than leaving a dead button.
 */
async function copyText(text: string): Promise<void> {
  // Widened to `| undefined` on purpose: lib.dom declares
  // `navigator.clipboard` as always present, but outside a secure context
  // (plain http on the LAN) the property does not exist at runtime.
  const clipboard = navigator.clipboard as Clipboard | undefined

  if (clipboard?.writeText) {
    try {
      await clipboard.writeText(text)
      return
    } catch {
      // fall through to the textarea fallback
    }
  }

  const area = document.createElement('textarea')
  area.value = text
  area.style.position = 'fixed'
  area.style.opacity = '0'
  document.body.appendChild(area)
  area.select()

  try {
    document.execCommand('copy')
  } finally {
    document.body.removeChild(area)
  }
}

function AdminPage() {
  const [token, setToken] = useState<string | null>(readToken)
  const [password, setPassword] = useState('')
  const [state, setState] = useState<AdminState | null>(null)
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState(false)

  const forgetToken = useCallback((reason: string) => {
    try {
      sessionStorage.removeItem(TOKEN_KEY)
    } catch {
      // storage can be unavailable; the in-memory state still resets
    }
    setToken(null)
    setState(null)
    setMessage(reason)
  }, [])

  const refresh = useCallback(
    async (activeToken: string) => {
      try {
        setState(await fetchState(activeToken))
        setMessage('')
      } catch (error) {
        if (error instanceof UnauthorizedError) {
          forgetToken('That admin password was rejected.')
        } else {
          setMessage(
            error instanceof Error
              ? error.message
              : 'Could not reach the sharer',
          )
        }
      }
    },
    [forgetToken],
  )

  // Poll while logged in. Cleared on unmount and whenever the token changes.
  useEffect(() => {
    if (!token) {
      return
    }

    void refresh(token)
    const timer = window.setInterval(
      () => void refresh(token),
      POLL_INTERVAL_MS,
    )

    return () => window.clearInterval(timer)
  }, [token, refresh])

  const run = useCallback(
    async (action: (activeToken: string) => Promise<void>) => {
      if (!token) {
        return
      }

      setBusy(true)
      try {
        await action(token)
      } catch (error) {
        if (error instanceof UnauthorizedError) {
          forgetToken('That admin password was rejected.')
        } else {
          setMessage(error instanceof Error ? error.message : 'Command failed')
        }
      } finally {
        setBusy(false)
      }

      // Refetch either way: a failed command is usually a stale list.
      await refresh(token)
    },
    [token, refresh, forgetToken],
  )

  if (!token) {
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
            const value = password.trim()
            if (!value) {
              return
            }
            try {
              sessionStorage.setItem(TOKEN_KEY, value)
            } catch {
              // ignore: the token is still used for this page's lifetime
            }
            setMessage('')
            setToken(value)
          }}
        >
          <VStack gap="4" align="stretch" minW="280px">
            <Text fontSize="lg" fontWeight="semibold">
              Mira Sharer admin
            </Text>
            <Text fontSize="sm" color="fg.muted">
              Enter the password from <code>webui.admin_password</code>.
            </Text>
            <Input
              type="password"
              autoFocus
              placeholder="Admin password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
            <Button type="submit" colorPalette="blue">
              Sign in
            </Button>
            {message ? (
              <Text fontSize="sm" color="fg.error">
                {message}
              </Text>
            ) : null}
          </VStack>
        </form>
      </Box>
    )
  }

  const running = state?.running ?? false

  return (
    <Box h="100%" overflowY="auto" p={{ base: '4', md: '8' }}>
      <VStack gap="6" align="stretch" maxW="720px" mx="auto">
        <HStack justify="space-between">
          <HStack gap="3">
            <Text fontSize="xl" fontWeight="bold">
              Mira Sharer admin
            </Text>
            <Badge colorPalette={running ? 'green' : 'gray'}>
              {running ? 'sharing' : 'idle'}
            </Badge>
          </HStack>
          <HStack gap="2">
            <Button
              colorPalette="blue"
              disabled={busy || running}
              onClick={() => void run(startSession)}
            >
              Start sharing
            </Button>
            <Button
              colorPalette="red"
              variant="outline"
              disabled={busy || !running}
              onClick={() => void run(stopSession)}
            >
              Stop
            </Button>
            <Button variant="ghost" onClick={() => forgetToken('Signed out.')}>
              Sign out
            </Button>
          </HStack>
        </HStack>

        {message ? (
          <Text fontSize="sm" color="fg.error">
            {message}
          </Text>
        ) : null}

        <Box borderWidth="1px" borderColor="border" borderRadius="md" p="4">
          <Text fontWeight="semibold" mb="3">
            Invite
          </Text>
          {running ? (
            <VStack gap="3" align="stretch">
              <CopyRow label="Invite link" value={state?.inviteLink ?? ''} />
              <CopyRow label="Room" value={state?.room ?? ''} />
              <CopyRow label="Passcode" value={state?.password ?? ''} />
            </VStack>
          ) : (
            <Text fontSize="sm" color="fg.muted">
              Start sharing to get an invite link.
            </Text>
          )}
        </Box>

        <Box borderWidth="1px" borderColor="border" borderRadius="md" p="4">
          <HStack justify="space-between" mb="3">
            <Text fontWeight="semibold">Waiting for a decision</Text>
            {state?.autoAccept ? (
              <Text fontSize="sm" color="fg.muted">
                auto_accept is on — viewers are admitted without asking
              </Text>
            ) : null}
          </HStack>
          {state?.pending.length ? (
            <VStack gap="2" align="stretch">
              {state.pending.map((viewer) => (
                <ViewerRow key={viewer.uuid} viewer={viewer}>
                  <Button
                    size="sm"
                    colorPalette="green"
                    disabled={busy}
                    onClick={() =>
                      void run((activeToken) =>
                        acceptViewer(activeToken, viewer.uuid),
                      )
                    }
                  >
                    Accept
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={busy}
                    onClick={() =>
                      void run((activeToken) =>
                        declineViewer(activeToken, viewer.uuid),
                      )
                    }
                  >
                    Decline
                  </Button>
                </ViewerRow>
              ))}
            </VStack>
          ) : (
            <Text fontSize="sm" color="fg.muted">
              No one is waiting.
            </Text>
          )}
        </Box>

        <Box borderWidth="1px" borderColor="border" borderRadius="md" p="4">
          <Text fontWeight="semibold" mb="3">
            Viewing
          </Text>
          {state?.viewing.length ? (
            <VStack gap="2" align="stretch">
              {state.viewing.map((viewer) => (
                <ViewerRow key={viewer.uuid} viewer={viewer}>
                  <Button
                    size="sm"
                    variant="outline"
                    colorPalette="red"
                    disabled={busy}
                    onClick={() =>
                      void run((activeToken) =>
                        kickViewer(activeToken, viewer.uuid),
                      )
                    }
                  >
                    Kick
                  </Button>
                </ViewerRow>
              ))}
            </VStack>
          ) : (
            <Text fontSize="sm" color="fg.muted">
              Nobody is watching.
            </Text>
          )}
        </Box>
      </VStack>
    </Box>
  )
}

function CopyRow({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false)

  return (
    <HStack gap="3">
      <Text fontSize="sm" color="gray.400" minW="90px">
        {label}
      </Text>
      <Input readOnly value={value} fontFamily="mono" fontSize="sm" />
      <Button
        size="sm"
        variant="outline"
        disabled={!value}
        onClick={() => {
          void copyText(value).then(() => {
            setCopied(true)
            window.setTimeout(() => setCopied(false), 1500)
          })
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </Button>
    </HStack>
  )
}

function ViewerRow({
  viewer,
  children,
}: {
  viewer: Viewer
  children: React.ReactNode
}) {
  return (
    <HStack justify="space-between" gap="3">
      <Text fontSize="sm" truncate>
        {viewer.name || 'unnamed viewer'}{' '}
        <Text as="span" color="fg.subtle" fontSize="xs">
          {viewer.uuid.slice(0, 8)}
        </Text>
      </Text>
      <HStack gap="2">{children}</HStack>
    </HStack>
  )
}
