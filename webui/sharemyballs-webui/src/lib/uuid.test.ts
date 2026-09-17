import { describe, expect, it } from 'vitest'
import { makeId, makeViewerId } from '#/lib/uuid'

const V4 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/

describe('makeId', () => {
  it('formats a v4 UUID', () => {
    expect(makeId(null)).toMatch(V4)
  })

  // `crypto.randomUUID` is secure-context-only, so a LAN viewer on
  // http://192.168.x.x:8765/ never reaches it. `getRandomValues` is the path
  // that actually runs in the field; this pins the version/variant nibbles.
  it('sets the version and variant bits from getRandomValues', () => {
    const source = {
      getRandomValues: (bytes: Uint8Array) => bytes.fill(0xff),
    }

    const id = makeId(source)

    expect(id).toMatch(V4)
    expect(id).toBe('ffffffff-ffff-4fff-bfff-ffffffffffff')
  })

  it('falls back to Math.random when there is no crypto at all', () => {
    const id = makeId(null)
    expect(id).toMatch(V4)
  })

  it('falls back when the source cannot fill a buffer', () => {
    // An empty object is what `globalThis.crypto` looks like in an engine with
    // getRandomValues removed; the old code called it and threw.
    expect(makeId({})).toMatch(V4)
  })

  it('does not collide across calls', () => {
    const ids = new Set(Array.from({ length: 200 }, () => makeId(null)))
    expect(ids.size).toBe(200)
  })

  it('produces one usable id via makeViewerId', () => {
    expect(makeViewerId()).toMatch(V4)
  })
})
