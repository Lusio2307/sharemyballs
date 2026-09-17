// jsdom implements the media *elements* but not the media *API*: `srcObject`,
// `muted` and a real `play()` are missing or throw "not implemented". The
// viewer's contract is exactly those properties, so without this the tests
// would assert on `undefined` and pass for the wrong reason.
//
// This is a test-harness detail, not production code: it makes jsdom behave
// like a browser instead of making the viewer defensive about a missing API.

// React only warns-free `act()` when it is told it is in a test environment.
// (`globalThis` is augmented by `@types/react-dom`'s act type, so this is a
// plain assignment rather than a hand-written `declare global`.)
export const reactActEnvironment = true
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = reactActEnvironment

const sources = new WeakMap<HTMLMediaElement, MediaStream | null>()
const muted = new WeakMap<HTMLMediaElement, boolean>()

Object.defineProperty(HTMLMediaElement.prototype, 'srcObject', {
  configurable: true,
  get(this: HTMLMediaElement) {
    return sources.get(this) ?? null
  },
  set(this: HTMLMediaElement, value: MediaStream | null) {
    sources.set(this, value ?? null)
  },
})

Object.defineProperty(HTMLMediaElement.prototype, 'muted', {
  configurable: true,
  get(this: HTMLMediaElement) {
    return muted.get(this) ?? false
  },
  set(this: HTMLMediaElement, value: boolean) {
    muted.set(this, value)
  },
})

/**
 * Resolves, like a browser playing a muted/gesture-approved element. Tests
 * replace it with `vi.fn()` when they need the refusal path.
 */
HTMLMediaElement.prototype.play = function play(this: HTMLMediaElement) {
  return Promise.resolve()
}

HTMLMediaElement.prototype.pause = function pause() {}

HTMLMediaElement.prototype.load = function load() {}
