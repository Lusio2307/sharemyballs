# AGENTS.md

## Post-implementation checks

After any implementation or code edits, run these commands to verify the changes:

```sh
cargo check
cargo fmt
```

## Web UI

`src/webui/mod.rs` inlines `webui/sharemyballs-webui/dist/index.html` with
`include_str!`, and that `dist/` is gitignored. `cargo check` therefore fails
until the SPA has been built, so build it first whenever you touch `webui/` or
`src/webui/`:

```sh
cd webui/sharemyballs-webui && bun install --frozen-lockfile && bun run build
```

Requires bun >= 1.2. Setting `MIRA_WEBUI_AUTOBUILD=1` lets `build.rs` run that
command itself.

The page is a client-only React SPA (Vite + TanStack Router + Chakra UI) bundled
into a single file by `vite-plugin-singlefile`. Keep it to one file: no dynamic
`import()`, no `React.lazy`, no manual chunks, and no new route without a
matching fallback on the Rust side (the embedded server only serves `/`). See
`WEBUI.md`.
