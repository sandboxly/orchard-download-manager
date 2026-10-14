// Which of the browser's downloads go to the app. Plain functions without
// browser calls, so every browser's extension can share them (and the tests
// run them in Node: `node --test extension/test/*.test.mjs`).

/** "a.org\nb.org" or "pdf, .jpg" → trimmed, lower-case entries. */
export function list(text) {
  return String(text || '')
    .split(/[\s,;]+/)
    .map((s) => s.trim().toLowerCase().replace(/^\*?\./, ''))
    .filter(Boolean);
}

/** Whether `host` is one of `sites` or below one. */
export function siteMatches(host, sites) {
  host = String(host || '').toLowerCase();
  return sites.some((s) => host === s || host.endsWith('.' + s));
}

/** The file type of a name or path ("" without one). */
export function extension(name) {
  const base = String(name || '').split(/[/\\]/).pop();
  const dot = base.lastIndexOf('.');
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : '';
}

/** Links the app can download. */
export function downloadable(url) {
  return /^https?:\/\//i.test(String(url || ''));
}

/**
 * Whether the app takes this download: `null` if so, otherwise why not.
 * `d`: { url, filename, fileSize, incognito, byExtensionId }.
 */
export function skipReason(d, options) {
  if (!options.takeOver) return 'off';
  if (d.incognito) return 'private window';
  if (d.byExtensionId) return 'started by an extension';
  if (!downloadable(d.url)) return 'not a web link';
  let host = '';
  try {
    host = new URL(d.url).hostname;
  } catch {
    return 'not a web link';
  }
  if (siteMatches(host, list(options.skipSites))) return 'skipped site';
  const type = extension(d.filename) || extension(new URL(d.url).pathname);
  if (type && list(options.skipTypes).includes(type)) return 'skipped file type';
  const min = Number(options.minSizeKB) || 0;
  if (min > 0 && d.fileSize > 0 && d.fileSize < min * 1024) return 'small file';
  return null;
}
