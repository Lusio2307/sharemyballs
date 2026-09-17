// Client for the Rust-side admin control plane (`src/webui/admin.rs`).
//
// Every call carries the admin secret as a Bearer token; the server rejects
// anything else with 401, and does not even register these routes when
// `webui.admin_password` is unset.

export type Viewer = {
  uuid: string
  name: string
}

export type AdminState = {
  running: boolean
  room: string | null
  password: string | null
  inviteLink: string | null
  autoAccept: boolean
  pending: Viewer[]
  viewing: Viewer[]
}

/** The token was rejected: the caller should ask for the password again. */
export class UnauthorizedError extends Error {}

const authorizedFetch = async (
  token: string,
  path: string,
  init?: RequestInit,
): Promise<Response> => {
  const response = await fetch(path, {
    ...init,
    headers: { ...(init?.headers ?? {}), Authorization: `Bearer ${token}` },
  })

  if (response.status === 401) {
    throw new UnauthorizedError('The admin password was rejected')
  }

  return response
}

export const fetchState = async (token: string): Promise<AdminState> => {
  const response = await authorizedFetch(token, '/api/admin/state')

  if (!response.ok) {
    throw new Error(
      `Could not read the session state (HTTP ${response.status})`,
    )
  }

  return (await response.json()) as AdminState
}

/**
 * Fire a command. A 404 is not surfaced as an error: it means the viewer was
 * gone (a stale page), and the caller refreshes the state anyway.
 */
const command = async (token: string, path: string): Promise<void> => {
  const response = await authorizedFetch(token, path, { method: 'POST' })

  if (!response.ok && response.status !== 404) {
    throw new Error(`Command failed (HTTP ${response.status})`)
  }
}

export const startSession = (token: string) =>
  command(token, '/api/admin/session/start')

export const stopSession = (token: string) =>
  command(token, '/api/admin/session/stop')

export const acceptViewer = (token: string, uuid: string) =>
  command(token, `/api/admin/viewers/${encodeURIComponent(uuid)}/accept`)

export const declineViewer = (token: string, uuid: string) =>
  command(token, `/api/admin/viewers/${encodeURIComponent(uuid)}/decline`)

export const kickViewer = (token: string, uuid: string) =>
  command(token, `/api/admin/viewers/${encodeURIComponent(uuid)}/kick`)
