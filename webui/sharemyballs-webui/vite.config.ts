import { defineConfig } from 'vite'
import viteReact from '@vitejs/plugin-react'
import { viteSingleFile } from 'vite-plugin-singlefile'

// Plain client-only SPA built into a single self-contained dist/index.html.
//
// The Rust server embeds that one file with include_str! and serves it at
// `GET /`, so the build must not emit a second asset (JS/CSS chunk, font,
// source map) that nobody would serve. `viteSingleFile()` inlines all of them;
// never add dynamic import()/React.lazy or manual chunks, because split chunks
// cannot be inlined and would be silently missing at runtime.
const config = defineConfig({
  // Relative asset URLs, so the page keeps working behind the
  // `webui.public_url` reverse proxy as well as from the loopback server.
  base: './',
  resolve: { tsconfigPaths: true },
  plugins: [viteReact(), viteSingleFile()],
  server: {
    proxy: {
      // The viewer derives its signalling URL from `location.host`, so on the
      // Vite dev server (:3000) a running sharer has to be proxied. Point it at
      // `webui.port` from config.toml (8765 by default).
      '/signaller': {
        target: 'ws://127.0.0.1:8765',
        ws: true,
      },
    },
  },
  preview: {
    proxy: {
      '/signaller': {
        target: 'ws://127.0.0.1:8765',
        ws: true,
      },
    },
  },
})

export default config
