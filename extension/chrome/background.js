// The extension's worker: takes over the browser's downloads and adds the
// context menu. A download goes to the app only once the app has taken it;
// if the app can't be reached, the browser downloads it as usual.

import * as app from './common/app.js';
import * as options from './common/options.js';
import { downloadable, skipReason } from './common/rules.js';

const MENU_ONE = 'download-one';
const MENU_ALL = 'download-all';
// More links than this on a page are cut off (the app's Add dialog lists them all).
const MAX_LINKS = 2000;

// -- problems: shown on the toolbar button until the next success ----------

async function problem(message) {
  await chrome.action.setBadgeBackgroundColor({ color: '#f92672' });
  await chrome.action.setBadgeText({ text: '!' });
  await chrome.action.setTitle({ title: `Orchard Download Manager: ${message}` });
  await chrome.storage.session.set({ problem: message });
}

async function fine() {
  await chrome.action.setBadgeText({ text: '' });
  await chrome.action.setTitle({ title: 'Orchard Download Manager' });
  await chrome.storage.session.remove('problem');
}

/** Hand links to the app; false (and the problem shown) if it didn't take them. */
async function handOver(links) {
  try {
    await app.add(links);
    await fine();
    return true;
  } catch (e) {
    await problem(app.reason(e));
    return false;
  }
}

// -- the browser's downloads ------------------------------------------------

chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  (async () => {
    const opts = await options.load();
    const url = item.finalUrl || item.url;
    const why = skipReason({ url, filename: item.filename, fileSize: item.fileSize > 0 ? item.fileSize : item.totalBytes, incognito: item.incognito, byExtensionId: item.byExtensionId }, opts);
    if (why || !(await handOver([{ url, referrer: item.referrer, filename: item.filename }]))) {
      suggest();
      return;
    }
    // The app has it: the browser's copy goes, also from the browser's list.
    try {
      await chrome.downloads.cancel(item.id);
      await chrome.downloads.erase({ id: item.id });
    } catch {
      // Already gone.
    }
    suggest();
  })();
  return true; // suggest() comes later
});

// -- the context menu --------------------------------------------------------

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: MENU_ONE, title: 'Download with Orchard Download Manager', contexts: ['link', 'image', 'video', 'audio'] });
    chrome.contextMenus.create({ id: MENU_ALL, title: 'Download all links with Orchard Download Manager', contexts: ['page', 'selection'] });
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const referrer = info.frameUrl || info.pageUrl || '';
  if (info.menuItemId === MENU_ONE) {
    const url = info.linkUrl || info.srcUrl;
    if (downloadable(url)) {
      await handOver([{ url, referrer }]);
    } else {
      await problem('This link only works inside the page — the app can download web links (http and https)');
    }
    return;
  }
  if (info.menuItemId === MENU_ALL && tab?.id !== undefined) {
    let found = [];
    try {
      const results = await chrome.scripting.executeScript({ target: { tabId: tab.id, frameIds: [info.frameId || 0] }, func: pageLinks, args: [Boolean(info.selectionText)] });
      found = results.flatMap((r) => r.result || []);
    } catch {
      await problem("Can't read the links on this page — the browser doesn't allow it here");
      return;
    }
    const links = [...new Set(found)].filter(downloadable).slice(0, MAX_LINKS);
    if (links.length === 0) {
      await problem(info.selectionText ? 'No links in the selection — select text that has links in it' : 'No links on this page');
      return;
    }
    await handOver(links.map((url) => ({ url, referrer })));
  }
});

/** Runs in the page: the links in the selection, or all on the page. */
function pageLinks(selectionOnly) {
  const sel = window.getSelection();
  const inSelection = (node) => {
    for (let i = 0; i < sel.rangeCount; i++) if (sel.getRangeAt(i).intersectsNode(node)) return true;
    return false;
  };
  const urls = [];
  for (const a of document.querySelectorAll('a[href]')) {
    if (!selectionOnly || inSelection(a)) urls.push(a.href);
  }
  if (!selectionOnly) {
    for (const m of document.querySelectorAll('video[src], audio[src], source[src]')) urls.push(m.src);
  }
  return urls;
}
