// The extension's settings, kept in the browser's synced storage.

export const DEFAULTS = {
  // Hand the browser's own downloads to the app.
  takeOver: true,
  // Smaller files stay with the browser (KB, 0: all sizes).
  minSizeKB: 0,
  // Sites whose downloads stay with the browser, one per line ("example.org"
  // also covers its subdomains).
  skipSites: '',
  // File types that stay with the browser, like "pdf jpg".
  skipTypes: '',
};

export async function load() {
  const stored = await chrome.storage.sync.get(DEFAULTS);
  return { ...DEFAULTS, ...stored };
}

export async function save(changes) {
  await chrome.storage.sync.set(changes);
}
