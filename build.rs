extern crate embed_resource;

use std::path::Path;
use std::process::Command;

/// The built web UI, relative to the crate root. `src/webui/mod.rs` inlines it
/// with `include_str!`, so it must exist before the crate is compiled.
const WEBUI_INDEX: &str = "webui/sharemyballs-webui/dist/index.html";

/// The web UI project, for the messages below.
const WEBUI_DIR: &str = "webui/sharemyballs-webui";

fn main() {
    ensure_webui_built();

    if cfg!(target_os = "windows") {
        embed_resource::compile("mira-manifest.rc", embed_resource::NONE);
    }
}

/// `dist/` is generated and gitignored, so a fresh checkout (and CI) has to
/// build it before cargo can read it. Point at the exact command rather than
/// letting `include_str!` emit `couldn't read webui/.../index.html`.
///
/// The SPA build is only run from here when `MIRA_WEBUI_AUTOBUILD` is set: a
/// build script that silently invokes a package manager (and possibly the
/// network) makes `cargo check` slow and non-hermetic.
fn ensure_webui_built() {
    // Declaring any `rerun-if-changed` narrows cargo's default "rerun on any
    // change in the package", so every input this script cares about has to be
    // listed explicitly.
    println!("cargo:rerun-if-changed={WEBUI_INDEX}");
    println!("cargo:rerun-if-changed=mira-manifest.rc");

    if Path::new(WEBUI_INDEX).exists() {
        return;
    }

    if std::env::var_os("MIRA_WEBUI_AUTOBUILD").is_some() {
        let status = Command::new("bun")
            .args(["run", "build"])
            .current_dir(WEBUI_DIR)
            // Inherit stdio: the bundler's own output is the useful one.
            .status();

        match status {
            Ok(status) if status.success() => {}
            Ok(status) => panic!("`bun run build` in {WEBUI_DIR} failed: {status}"),
            Err(error) => {
                panic!("could not run `bun` ({error}); install bun >= 1.2 from https://bun.sh")
            }
        }
    }

    if !Path::new(WEBUI_INDEX).exists() {
        panic!(
            "{WEBUI_INDEX} is missing: the web UI is embedded into the binary with \
             include_str!, so it has to exist before compiling.\n\
             Build it with:\n\n    \
             cd {WEBUI_DIR} && bun install --frozen-lockfile && bun run build\n\n\
             Requires bun >= 1.2. Set MIRA_WEBUI_AUTOBUILD=1 to have this build run \
             it automatically."
        );
    }
}
