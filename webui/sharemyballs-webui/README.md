# Mira Sharer web UI

The viewer page served by the `mira_sharer` binary at `GET /`. It is a
client-only React SPA (Vite + TanStack Router + Chakra UI) built into a **single
self-contained `dist/index.html`**, which the Rust side embeds with
`include_str!` (`src/webui/mod.rs`) and serves from the in-process axum server.

Right now the page is a canvas with a fullscreen control. The WebRTC viewer that
used to live in a hand-written `webui/index.html` has been retired; see
`WEBUI.md` in the repository root for the porting checklist that has to come
back with it.

## Requirements

- [bun](https://bun.sh) >= 1.2 (the lockfile is the text-based `bun.lock`)

## Commands

```bash
bun install --frozen-lockfile   # install dependencies
bun run dev                     # dev server on http://localhost:3000 (HMR)
bun run build                   # -> dist/index.html (single file)
bun run preview                 # serve the production build
bun run typecheck               # tsc --noEmit
bun run lint                    # eslint
bun run format                  # prettier --write + eslint --fix
```

`bun test` would run bun's own test runner, not this package's `test` script —
there is no test script; use the commands above.

## Build constraints

The build output is embedded in the binary and served as one HTML response, so:

- `vite-plugin-singlefile` inlines every asset. The build **must** keep
  producing `dist/index.html` with no sibling `assets/` directory.
- Never introduce dynamic `import()` / `React.lazy()` or manual chunks: split
  chunks cannot be inlined and would be requested from a server that does not
  have them.
- `base: './'` keeps asset URLs relative, so the page works both on loopback and
  behind the `webui.public_url` reverse proxy.
- `class="dark"` on `<html>` in `index.html` is **load-bearing**: Chakra's
  semantic tokens (`fg`, `bg`, `border`, …) resolve to their _light_ values
  without it, so text renders near-black on the black background. There is no
  runtime colour-mode toggle — the app is dark-only — so leave the class alone.
  (TanStack Start used to inject it with a script; the SPA declares it instead.)

After changing anything under `src/`, rebuild (`bun run build`) before running
`cargo check` — the Rust test asserts on the built page, and `include_str!`
needs `dist/index.html` to exist.

## Routing

File-based routing via TanStack Router. Routes live in `src/routes`; the root
route (`src/routes/__root.tsx`) wraps them in the Chakra provider. Adding a file
means regenerating the route tree, which `bun run dev` and `bun run build`
already do via `tsr generate`. The embedded server has no SPA fallback: `/` and
`/admin` are registered explicitly in `src/webui/mod.rs`, so any further route
needs its own Rust route serving the same document.

## Toolchain notes

`typescript` is pinned to `^6`, not `^7`: the scaffold installed TS 7, but the
`typescript-eslint` that `@tanstack/eslint-config` uses only implements the TS 6
API, so `bun run lint` and `bun run format` aborted with _"typescript-eslint
does not support TS 7.0"_. TS 6 typechecks this project without changes.
