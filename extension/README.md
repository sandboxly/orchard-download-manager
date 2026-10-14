# Browser extension

Sends downloads from the browser to Orchard Download Manager. Chrome for
now (`chrome/`); Edge, Firefox and Safari follow and are meant to reuse
`chrome/common/`.

What it does:

* **Takes over the browser's downloads** (on by default, a switch in the
  toolbar popup). The browser's copy is cancelled only once the app has
  the link. When the app can't be reached, the browser downloads the file
  as usual, so no download is lost. Downloads in private windows, from
  other extensions, `blob:` and `data:` links stay with the browser. The
  settings page can leave small files, some sites or some file types to
  the browser.
* **Context menu**: "Download with Orchard Download Manager" on links,
  images, video and audio; "Download all links…" on a page or a selection
  (the app's Add dialog lists them all).
* **Downloads behind a sign-in work**: the browser's cookies for the link,
  the referring page and the browser's user agent go along as request
  headers. Reading cookies needs access to the link's site: the extension
  asks for no site access at install (the store reviews broad access
  closely), so by default cookies go along only for links right-clicked on
  the site the tab shows (the click grants that site for a moment). "Send
  sign-in cookies with every download", in the popup and the settings page,
  asks the browser for access to all sites (an optional permission).

In the app, Settings › Browser decides whether links from the browser open
the Add dialog first (the default) or go straight into the list.

## Trying it out

1. Build and start the app once (`cargo run --release`). On start it tells
   the installed browsers where its helper is (see below).
2. Open `chrome://extensions`, turn on Developer mode, choose **Load
   unpacked** and pick `extension/chrome`.
3. The toolbar popup says "Connected to the app". Download something.

The development extension has a fixed ID, `paglhhneednmdpggkmbipgljhgjepaga`,
from the public `key` in `manifest.json` (the private key isn't in the
repository and isn't needed to load the extension). The app only lets in the
IDs in `EXTENSION_IDS` in `src/native_host.rs`.

## How it talks to the app

The browser can't open connections to local apps itself; it starts a
"native messaging host" and talks to it over stdin and stdout. The app's
own executable is that host: the browser runs
`orchard-download-manager chrome-extension://<id>/`, which the app detects
before anything else (no window). Each message is JSON after a 4-byte
length in native byte order:

| Extension sends | App answers |
| --- | --- |
| `{"type": "ping"}` | `{"ok": true, "version": "0.1.0"}` |
| `{"type": "add", "userAgent": "…", "items": [{"url", "referrer", "cookies", "filename"}]}` | `{"ok": true}` or `{"ok": false, "error": "…"}` |

The helper hands the links to the running app through the same local
connection a second start of the app uses (`src/ipc.rs`), starting the app
first if it isn't running.

The browser finds the helper through a manifest named
`io.github.sandboxly.orchard_download_manager.json`. The app writes it on
every start (only when it changed), for each installed browser:

* macOS: `~/Library/Application Support/<browser>/NativeMessagingHosts/`
* Linux: `~/.config/<browser>/NativeMessagingHosts/`
* Windows: the manifest is in the app's config folder, and
  `HKEY_CURRENT_USER\Software\<browser>\NativeMessagingHosts\io.github.sandboxly.orchard_download_manager`
  points to it.

It isn't written when `ORCHARD_DOWNLOAD_MANAGER_CONFIG_DIR` is set (a test
setup the helper wouldn't reach).

## Files

* `chrome/manifest.json`: Manifest V3.
* `chrome/background.js`: the worker (downloads, context menu, problems on
  the toolbar button). Browser-specific.
* `chrome/popup.*`, `chrome/options.*`: the popup and the settings page.
* `chrome/common/`: shared with the other browsers later: `rules.js`
  (which downloads the app takes; no browser calls), `app.js` (talking to
  the app), `options.js`, `page.css`.
* `test/`: `node --test extension/test/*.test.mjs`.
* `package.sh`: `dist/orchard-download-manager-chrome.zip` for the store
  (without the development key; the store assigns its own ID, which then
  goes into `EXTENSION_IDS`).
