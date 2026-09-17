// Element-fullscreen helpers, with the webkit fallbacks typed explicitly rather
// than cast to `any`, so the fallback path stays type-checked.
//
// iOS Safari only fullscreens `<video>`, and some engines expose the API only
// under the `webkit` prefix -- hence the pseudo-fullscreen fallback in the
// viewer, which lays the stage over the page instead of leaving a dead button.

type FullscreenDocument = Document & {
  webkitFullscreenElement?: Element | null
  webkitFullscreenEnabled?: boolean
  webkitExitFullscreen?: () => Promise<void> | void
}

type FullscreenElement = HTMLElement & {
  webkitRequestFullscreen?: () => Promise<void> | void
}

/** The element currently fullscreen, or `null`. */
export const fullscreenElement = (): Element | null => {
  const doc = document as FullscreenDocument
  return doc.fullscreenElement ?? doc.webkitFullscreenElement ?? null
}

/** Whether the element-fullscreen API is usable for `element`. */
export const canUseFullscreenApi = (element: HTMLElement): boolean => {
  const doc = document as FullscreenDocument
  const enabled =
    doc.fullscreenEnabled === true || doc.webkitFullscreenEnabled === true
  const canRequest =
    typeof element.requestFullscreen === 'function' ||
    typeof (element as FullscreenElement).webkitRequestFullscreen === 'function'

  return enabled && canRequest
}

export const enterFullscreen = async (element: HTMLElement): Promise<void> => {
  const webkit = (element as FullscreenElement).webkitRequestFullscreen

  if (typeof element.requestFullscreen === 'function') {
    await element.requestFullscreen()
  } else if (webkit) {
    await webkit.call(element)
  }
}

/**
 * Leave fullscreen. Safe to call when nothing is fullscreen: engines differ on
 * whether that resolves or rejects, and neither is an error here.
 */
export const exitFullscreen = async (): Promise<void> => {
  const doc = document as FullscreenDocument

  if (typeof doc.exitFullscreen === 'function') {
    await doc.exitFullscreen()
  } else if (doc.webkitExitFullscreen) {
    await doc.webkitExitFullscreen()
  }
}
