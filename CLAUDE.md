# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Orchard Download Manager: a download manager from the Orchard family (segmented, resumable HTTP(S) downloads, a queue, speed limits, a scheduler, clipboard monitoring, checksums). Like Orchard Commander and Orchard Music Player it is a GPU-drawn desktop app (winit + wgpu) styled as a character grid, not a TUI. It reuses Commander's crates (`oc-grid` renderer, `oc-config` color schemes) as git dependencies pinned to a Commander release tag; to work on them together, add a `[patch]` section pointing at a local checkout (`../orchard-commander`).

## Commands

```sh
cargo run --release -- [LINK]...                                   # run (links go to a running instance)
cargo test                                                         # all tests (a local HTTP server, no network)
ODM_TEST_URL=https://… cargo test real_server -- --nocapture       # plus one real server
cargo clippy --all-targets -- -D warnings                          # lint
node --test extension/test/*.test.mjs                              # the extension's rules
extension/package.sh                                               # dist/orchard-download-manager-chrome.zip for the store
cargo run -- --screenshot /tmp/x.png --size 120x40 --dialog settings:speed   # look at the UI headless
```

- `ORCHARD_DOWNLOAD_MANAGER_CONFIG_DIR=<empty dir>` runs with default settings and an empty list, without touching the real config.
- `--screenshot` dialogs: `add`, `batch`, `settings[:PAGE]`, `remove`, `properties`, `help`; `--empty` for the empty list, `--cursor N` selects a row.

## Architecture

- `app.rs` owns all state and logic (the list, the queue and schedule, engine events, keys); `ui/` draws it as a pure function into an `oc_grid::Frame` and holds the dialogs; `main.rs` owns only the window, input conversion and wake-ups.
- `engine/`: one controller thread per running download plus a thread per connection (plain threads, `ureq`, no async). Segments are split dynamically: a connection that finishes takes over half of the largest remaining segment. Bytes go to "<name>.part" by offset; `Shared` holds live progress, which the UI reads. Events come back through a callback that posts to the UI thread. `engine/tests.rs` has the local test server (ranges, throttling, broken connections, statuses) that the app tests reuse.
- `ui/form.rs`: the form controls shared by all dialogs (text, check, combo, spin, button; focus and keys). `ui/settings.rs` is data-driven: a setting is an `Id` with a label, hint, type and get/set; add one there and to `Settings`.
- Blocking work (probing a link, checksums, deleting files) goes through `tasks.rs`, never the UI thread.
- `native_host.rs` + `extension/chrome/`: the browser extension talks to the app's helper mode (started by the browser with a `chrome-extension://…/` argument, detected first thing in `main`): length-prefixed JSON on stdin/stdout, links with their headers (cookies, referrer, user agent) forwarded through `ipc.rs` as a `Handoff`. The app writes the helper's manifest for installed browsers on every start (Windows: registry), except with `ORCHARD_DOWNLOAD_MANAGER_CONFIG_DIR` set. The extension's shared logic is in `extension/chrome/common/` (no browser calls in `rules.js`; `node --test extension/test/*.test.mjs`). Cookies are kept in `downloads.json` (owner-only) and must never be logged or shown.
- `ipc.rs`: a second start hands its links to the running app over a local socket (`instance.json` with a token in the config folder).

## Gotchas

- **UI copy** follows `docs/ai-microcopy-guide.md`: sentence case, front-loaded and short, errors that say what went wrong, why and how to fix it, empty states that say how to start, no periods on fragments. Developer details go to the log, not into messages.
- **No third-party product names** in user-visible text, docs or code comments (describe the behaviour), except where a license requires it.
- Glyphs must exist in the grid font: emoji-presentation symbols fall back to an emoji font and break the layout.
- Tests must never touch the real config: `App::new(…, false)` doesn't read or write it.
