# Chrome Web Store listing

What goes into the store's developer dashboard. The upload is
`dist/orchard-download-manager-chrome.zip` (`extension/package.sh`); the
images are made from the app's screenshots (1280×800 screenshots, a 440×280
promo tile).

## Store listing

**Name:** Orchard Download Manager

**Summary** (132 characters at most):
Send the browser's downloads to Orchard Download Manager: several connections per file, resumable, in your queue.

**Category:** Tools · **Language:** English

**Description:**

Orchard Download Manager is a free, open source download manager for macOS, Windows and Linux. This extension hands the browser's downloads over to it.

What you get
• Faster downloads: the app loads a file over several connections at once
• Downloads that don't start over: after a broken connection, the app picks up where it stopped
• Your queue and schedule: limit how many downloads run at once, set speed limits, or let the queue run at night
• Checksums: paste an SHA-256, SHA-1 or MD5 sum and the file is checked when it's done

How it works
• Downloads you start in the browser go to the app instead. When the app isn't running or can't be reached, the browser downloads the file as usual, so no download is lost.
• Right-click a link, image, video or audio to send it to the app, or send every link on a page or in a selection.
• Downloads behind a sign-in work: the extension passes the site's cookies for that link to the app.
• Leave small files, some sites or some file types to the browser. Downloads in private windows always stay with the browser.

You need the app
Install Orchard Download Manager from https://sandboxly.github.io/orchard-download-manager/ and open it once; that connects it to the browser.

Privacy
The extension talks only to the app on your computer, never to a server. Nothing is collected or shared. Details: https://sandboxly.github.io/orchard-download-manager/privacy.html

Source code: https://github.com/sandboxly/orchard-download-manager

**Website:** https://sandboxly.github.io/orchard-download-manager/
**Support:** https://github.com/sandboxly/orchard-download-manager/issues

## Privacy practices

**Single purpose:** Hand downloads from the browser over to the Orchard Download Manager app installed on the same computer.

**Permission justifications:**

| Permission | Why |
| --- | --- |
| `downloads` | To see downloads the browser starts, hand them to the app, and cancel the browser's copy once the app has it. |
| `contextMenus` | The right-click items "Download with Orchard Download Manager" and "Download all links with Orchard Download Manager". |
| `cookies` | To pass the site's cookies for a download's address to the app, so files behind a sign-in download. Read only for links handed to the app, sent only to the local app. |
| `nativeMessaging` | The only way the extension talks to the app installed on the computer. |
| `storage` | The extension's own settings (on or off, sites and file types to skip). |
| `scripting` | To read the links on the current page or selection when the user chooses "Download all links". Runs only on that click. |
| `activeTab` | Access to the current tab when the user chooses "Download all links". |
| Host permission `<all_urls>` | Cookies are read for the download's address, which can be on any site. No content scripts run on pages. |

**Remote code:** No. All code is in the package.

**Data usage:** Collects "Authentication information" (cookies) and "Website content" (the links on a page, on request) only to provide the single purpose; the data goes only to the app on the same computer. Not sold, not used for unrelated purposes, not used for creditworthiness or lending.

**Privacy policy:** https://sandboxly.github.io/orchard-download-manager/privacy.html

## The store's ID

The store gave the extension the ID `dhebpclkehhbkajeefceephmdhchaeai`.
It is in `EXTENSION_IDS` in `src/native_host.rs`, so the app lets the
store version in.
