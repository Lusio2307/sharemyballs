import { ChakraProvider, createSystem, defaultConfig } from '@chakra-ui/react'
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ViewerStage } from '#/components/ViewerStage'
import type { SessionState, Streams } from '#/lib/viewerSession'

// The stage owns the two DOM media elements, so this is where "the stream is
// actually attached" and "the tap-to-enable-audio fallback works" are pinned.
// The session only decides *what* the streams are; a real `RTCPeerConnection`
// is not involved and is verified by hand (see WEBUI.md).

class FakeMediaStream {
  readonly tracks: Array<unknown>

  constructor(tracks: Array<unknown> = []) {
    this.tracks = tracks
  }
}

/**
 * A `MediaStream` stand-in. The real one is not constructible in jsdom, and the
 * stage only ever stores it on a media element, so identity is all that
 * matters.
 */
const fakeStream = (kind: string): MediaStream =>
  new FakeMediaStream([{ kind }]) as unknown as MediaStream

const system = createSystem(defaultConfig)

let container: HTMLDivElement
let root: Root

const noop = () => {}

const render = (props: {
  state?: SessionState
  message?: string
  streams?: Streams
  audioBlocked?: boolean
  onEnableAudio?: () => void
  onAudioPlaying?: () => void
  onReconnect?: () => void
  canReconnect?: boolean
}) => {
  act(() => {
    root.render(
      <ChakraProvider value={system}>
        <ViewerStage
          state={props.state ?? 'live'}
          message={props.message ?? ''}
          streams={props.streams ?? { video: null, audio: null }}
          audioBlocked={props.audioBlocked ?? false}
          onEnableAudio={props.onEnableAudio ?? noop}
          onAudioPlaying={props.onAudioPlaying ?? noop}
          onReconnect={props.onReconnect ?? noop}
          canReconnect={props.canReconnect ?? true}
        />
      </ChakraProvider>,
    )
  })
}

const video = () => container.querySelector('video')
const audio = () => container.querySelector('audio')

beforeEach(() => {
  vi.stubGlobal('MediaStream', FakeMediaStream)
  container = document.createElement('div')
  document.body.appendChild(container)
  act(() => {
    root = createRoot(container)
  })
})

afterEach(() => {
  act(() => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('ViewerStage', () => {
  it('attaches the video stream and plays it muted, inline and on autoplay', () => {
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play')
    const videoStream = fakeStream('video')

    render({ state: 'live', streams: { video: videoStream, audio: null } })

    const element = video()
    expect(element).not.toBeNull()
    expect(element?.autoplay).toBe(true)
    expect(element?.playsInline).toBe(true)
    // Muted autoplay is the only combination every browser allows without a
    // gesture, and the track is what the user came for.
    expect(element?.muted).toBe(true)
    expect(element?.srcObject).toBe(videoStream)
    expect(play).toHaveBeenCalled()
  })

  it('keeps the stage hidden until a video track actually arrives', () => {
    render({ state: 'awaitingStream', streams: { video: null, audio: null } })

    expect(video()?.style.display).toBe('none')
    expect(container.textContent).toContain('Starting…')
  })

  it('attaches the audio stream to the audio element', () => {
    const audioStream = fakeStream('audio')

    render({
      state: 'live',
      streams: { video: fakeStream('video'), audio: audioStream },
    })

    expect(audio()?.srcObject).toBe(audioStream)
  })

  // Autoplay policy: sound is the only thing a gesture can be required for, so
  // the page has to offer one rather than failing silently.
  it('offers a gesture when audio is blocked, and retries on tap', () => {
    const onEnableAudio = vi.fn()
    const onAudioPlaying = vi.fn()

    render({
      state: 'live',
      streams: { video: fakeStream('video'), audio: fakeStream('audio') },
      audioBlocked: true,
      onEnableAudio,
      onAudioPlaying,
    })

    const button = [...container.querySelectorAll('button')].find((element) =>
      element.textContent.includes('enable audio'),
    )
    expect(button).toBeDefined()

    act(() => {
      button?.click()
    })

    expect(onEnableAudio).toHaveBeenCalled()
  })

  it('hides the audio affordance when nothing is blocked', () => {
    render({
      state: 'live',
      streams: { video: fakeStream('video'), audio: null },
    })

    expect(container.textContent).not.toContain('enable audio')
  })

  it('always offers fullscreen on the stage container', () => {
    render({
      state: 'live',
      streams: { video: fakeStream('video'), audio: null },
    })

    const button = [...container.querySelectorAll('button')].find((element) =>
      element.textContent.includes('Fullscreen'),
    )
    expect(button).toBeDefined()
  })

  it('shows the reconnect affordance while a stream is live', () => {
    const onReconnect = vi.fn()

    render({
      state: 'live',
      streams: { video: fakeStream('video'), audio: null },
      onReconnect,
      canReconnect: true,
    })

    const button = [...container.querySelectorAll('button')].find(
      (element) => element.textContent.trim() === 'Reconnect',
    )
    expect(button).toBeDefined()

    act(() => {
      button?.click()
    })
    expect(onReconnect).toHaveBeenCalled()
  })

  it('reports the terminal message for a failed join', () => {
    render({
      state: 'failed',
      message: 'Declined by host.',
      canReconnect: false,
    })

    expect(container.textContent).toContain('Declined by host.')
    expect(container.textContent).not.toContain('Reconnect')
  })

  it('offers Reconnect after a lost connection', () => {
    render({
      state: 'lost',
      message: 'Connection to the sharer was lost.',
      canReconnect: true,
    })

    expect(container.textContent).toContain(
      'Connection to the sharer was lost.',
    )
    expect(container.textContent).toContain('Reconnect')
  })
})
