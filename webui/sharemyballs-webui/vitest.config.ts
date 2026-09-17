import { fileURLToPath } from 'node:url'
import { defineConfig, mergeConfig } from 'vitest/config'
import viteConfig from './vite.config.ts'

// The frontend has no DOM test runner of its own: everything worth testing
// (the ICE mapping, the candidate queue, the decline-reason text, the UUID
// fallback) is pure, and `#/` is a package import that Vitest cannot resolve on
// its own -- hence the explicit alias, mirroring `imports` in package.json.
//
// A real `RTCPeerConnection` is deliberately not tested: jsdom has none, and a
// fake one would only assert that the fake was called. The media plumbing stays
// thin and is verified by hand (see WEBUI.md).
export default mergeConfig(
  viteConfig,
  defineConfig({
    resolve: {
      alias: {
        '#': fileURLToPath(new URL('./src', import.meta.url)),
      },
    },
    test: {
      environment: 'jsdom',
      include: ['src/**/*.test.{ts,tsx}'],
      setupFiles: ['./src/test-setup.ts'],
    },
  }),
)
