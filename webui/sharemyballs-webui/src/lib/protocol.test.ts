import { describe, expect, it } from 'vitest'
import { declineReasonText, signallerUrl } from '#/lib/protocol'

describe('declineReasonText', () => {
  it('names every reason the sharer can send', () => {
    expect(declineReasonText('Unknown')).toBe('Join was declined.')
    expect(declineReasonText('IncorrectPassword')).toBe('Incorrect passcode.')
    expect(declineReasonText('NoCredentials')).toBe('No credentials provided.')
    expect(declineReasonText('UserDeclined')).toBe('Declined by host.')
  })

  // `DeclineReason` is a plain enum in src/signaller/mod.rs, so a serde
  // configuration change would put the numbers back on the wire. Accept both
  // spellings rather than showing a blank error.
  it('accepts the numeric discriminant as well', () => {
    expect(declineReasonText(0)).toBe('Join was declined.')
    expect(declineReasonText(1)).toBe('Incorrect passcode.')
    expect(declineReasonText(2)).toBe('No credentials provided.')
    expect(declineReasonText(3)).toBe('Declined by host.')
  })

  it('degrades to a generic message for anything unrecognised', () => {
    expect(declineReasonText(undefined)).toBe('Join was declined.')
    expect(declineReasonText(9)).toBe('Join was declined.')
    expect(declineReasonText('SomethingNew')).toBe('Join was declined.')
  })
})

describe('signallerUrl', () => {
  it('derives ws:// from an http origin', () => {
    expect(signallerUrl({ protocol: 'http:', host: '192.168.1.20:8765' })).toBe(
      'ws://192.168.1.20:8765/signaller',
    )
  })

  // Behind the `webui.public_url` reverse proxy the page is https, and the
  // WebSocket has to be wss:// or the browser blocks it as mixed content.
  it('derives wss:// from an https origin', () => {
    expect(
      signallerUrl({ protocol: 'https:', host: 'stream.example.com' }),
    ).toBe('wss://stream.example.com/signaller')
  })

  it('prefers an explicit signaller override', () => {
    expect(
      signallerUrl(
        { protocol: 'http:', host: 'localhost:3000' },
        'ws://127.0.0.1:8765/signaller',
      ),
    ).toBe('ws://127.0.0.1:8765/signaller')
  })

  it('ignores a blank override', () => {
    expect(
      signallerUrl({ protocol: 'http:', host: 'localhost:3000' }, '   '),
    ).toBe('ws://localhost:3000/signaller')
  })
})
