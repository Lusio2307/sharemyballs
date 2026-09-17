/**
 * A v4 UUID, without `crypto.randomUUID()`.
 *
 * `randomUUID()` is a secure-context-only API. Reaching the page from another
 * device on the LAN means a plain-HTTP origin (`http://192.168.1.20:8765/`),
 * where the method is undefined and the viewer would throw before it connected
 * anything -- so the invite link would appear dead on exactly the device it is
 * meant for. `getRandomValues()` carries no such restriction, and a
 * `Math.random` fallback keeps the page working where even that is missing.
 *
 * The browser cannot be trusted with entropy here in any case: the id only has
 * to be unique per page load, because the server keys viewers by it.
 */

type RandomSource = {
  getRandomValues?: (bytes: Uint8Array) => Uint8Array | void
}

const HEX = Array.from({ length: 256 }, (_, byte) =>
  byte.toString(16).padStart(2, '0'),
)

/** Uint8Array -> `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`. */
const formatV4 = (bytes: Uint8Array): string => {
  // version 4, variant 10xx -- cosmetic to the signaller, but it keeps the id
  // shaped like the v4 UUIDs the Rust side generates for rooms.
  bytes[6] = (bytes[6] & 0x0f) | 0x40
  bytes[8] = (bytes[8] & 0x3f) | 0x80

  return (
    HEX[bytes[0]] +
    HEX[bytes[1]] +
    HEX[bytes[2]] +
    HEX[bytes[3]] +
    '-' +
    HEX[bytes[4]] +
    HEX[bytes[5]] +
    '-' +
    HEX[bytes[6]] +
    HEX[bytes[7]] +
    '-' +
    HEX[bytes[8]] +
    HEX[bytes[9]] +
    '-' +
    HEX[bytes[10]] +
    HEX[bytes[11]] +
    HEX[bytes[12]] +
    HEX[bytes[13]] +
    HEX[bytes[14]] +
    HEX[bytes[15]]
  )
}

/**
 * Build an id from `randomSource` when it can fill a buffer, and from
 * `Math.random` otherwise. Exported so the fallback is testable without
 * deleting `crypto` from the test environment.
 */
export const makeId = (randomSource?: RandomSource | null): string => {
  const bytes = new Uint8Array(16)

  if (randomSource && typeof randomSource.getRandomValues === 'function') {
    randomSource.getRandomValues(bytes)
  } else {
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Math.floor(Math.random() * 256)
    }
  }

  return formatV4(bytes)
}

/** One per page load; see the StrictMode note in `src/routes/index.tsx`. */
export const makeViewerId = (): string =>
  makeId(globalThis.crypto as RandomSource | undefined)
