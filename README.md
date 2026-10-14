# Orchard Download Manager

A themeable, keyboard-friendly download manager from the Orchard family.
It is drawn as a character grid on the GPU, like Orchard Commander: same
renderer (`oc-grid`), same color schemes, same frame look.

* Segmented downloads: up to 32 connections per file; a connection that
  finishes takes over half of the biggest part left, so all of them keep
  working until the end.
* Pause and resume, also after quitting; a changed file on the server is
  noticed and downloaded again. A new address for an expired link.
* A queue with a limit on downloads at once; reorder it.
* Speed modes (low, medium, high), each with its own limit; a limit per
  download.
* Add one link, many pasted at once, or a pattern like `img[001-120].jpg`.
* Watches the clipboard for links to files (by type) and offers them.
* Sorts files into a folder per type, if you like.
* Schedule: start at night, pause in the morning, on chosen days.
* Checksums (MD5, SHA-1, SHA-256, SHA-512): checked right after
  downloading when given, or compared by hand.
* Keeps the computer awake while downloading; quit or sleep when done.
* Proxies (HTTP, SOCKS5), the system's certificates.
* Links from the command line go to the running app.
* A browser extension (Chrome for now): the browser's downloads, links
  and media from the context menu go to the app, with the cookies and
  referring page, so downloads behind a sign-in work. See
  [extension/README.md](extension/README.md).
* Settings for all of it (F9), and the color schemes shared with Orchard
  Commander.

## Install

Downloads for macOS (Apple silicon), Windows and Linux are on the
[website](https://sandboxly.github.io/orchard-download-manager/) and the
[releases page](https://github.com/sandboxly/orchard-download-manager/releases).
On macOS also:

```sh
brew install --cask sandboxly/tap/orchard-download-manager
```

## Building

```sh
cargo build --release
./target/release/orchard-download-manager [LINK]...
```

Packages (macOS dmg, Windows installer, deb, rpm, AppImage) and the release
steps: [docs/RELEASING.md](docs/RELEASING.md).

## Browser extension

`extension/chrome/` is the Chrome extension (plain JavaScript, no build
step). The app is its helper: the browser starts
`orchard-download-manager chrome-extension://…/`, which hands the links to
the running app (starting it if needed). The app tells the installed
browsers where it is each time it starts. To try it, open the app once,
then load `extension/chrome` unpacked in `chrome://extensions`. Details in
[extension/README.md](extension/README.md).

## Keys

| key                   | does                                              |
|-----------------------|---------------------------------------------------|
| F2, ⌘N                | add a download                                    |
| ⌘V                    | add the links on the clipboard                    |
| Enter, double-click   | open a finished file; start or pause the others   |
| Space, F5, F6         | start or pause · start · pause                    |
| S, P                  | start all · pause all                             |
| F3, F4                | open the file · show it in its folder             |
| Del, ⇧Del             | remove from the list · delete the files too       |
| R                     | download again from the start                     |
| E                     | properties: address, name, folder, limits         |
| H                     | checksums                                         |
| ⌥↑ ⌥↓                 | move in the queue                                 |
| Tab, 1–5              | all · active · paused · completed · failed        |
| C, /                  | filter by type · search                           |
| F7                    | speed mode                                        |
| I                     | details                                           |
| F9, ⌘,                | settings                                          |

## Files

Settings and the list live in the platform config folder
(`~/Library/Application Support/Orchard Download Manager` on macOS,
`~/.config/orchard-download-manager` on Linux), or in
`$ORCHARD_DOWNLOAD_MANAGER_CONFIG_DIR`.
