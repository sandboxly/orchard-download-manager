// Access to all sites, which the extension asks for only when the user turns
// on sign-in cookies for every download. Without it, cookies go along only
// for links right-clicked on the site the tab shows (the click grants that
// site for a moment).

const ALL_SITES = { origins: ['<all_urls>'] };

export function hasAllSites() {
  return chrome.permissions.contains(ALL_SITES);
}

/** Ask for access (only from a click or a key); true if granted. */
export function requestAllSites() {
  return chrome.permissions.request(ALL_SITES);
}

export function dropAllSites() {
  return chrome.permissions.remove(ALL_SITES);
}

/** A checkbox that shows and switches the access. */
export async function bind(box, onChange = () => {}) {
  box.checked = await hasAllSites();
  box.addEventListener('change', async () => {
    try {
      box.checked = box.checked ? await requestAllSites() : !(await dropAllSites());
    } catch {
      box.checked = await hasAllSites();
    }
    onChange();
  });
  chrome.permissions.onAdded.addListener(async () => (box.checked = await hasAllSites()));
  chrome.permissions.onRemoved.addListener(async () => (box.checked = await hasAllSites()));
}
