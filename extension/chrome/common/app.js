// Talking to the app through its helper (native messaging): the browser
// starts the helper, which hands the links to the running app and starts
// the app if needed.

export const HOST = 'io.github.sandboxly.orchard_download_manager';

function send(message) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendNativeMessage(HOST, message, (reply) => {
      const err = chrome.runtime.lastError;
      if (err) reject(new Error(err.message || 'no connection'));
      else if (!reply) reject(new Error('no answer'));
      else resolve(reply);
    });
  });
}

/** The app's version, or an error if it can't be reached. */
export async function ping() {
  const reply = await send({ type: 'ping' });
  if (!reply.ok) throw new Error(reply.error || 'no answer');
  return reply.version;
}

/** The Cookie header the browser would send to `url` ("" without any). */
export async function cookiesFor(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    return cookies.map((c) => `${c.name}=${c.value}`).join('; ');
  } catch {
    return '';
  }
}

/**
 * Hand links to the app. `links`: [{ url, referrer, filename }]. Cookies and
 * the browser's name go along, so downloads behind a sign-in work.
 */
export async function add(links) {
  const items = [];
  for (const l of links) {
    items.push({ url: l.url, referrer: l.referrer || '', filename: l.filename || '', cookies: await cookiesFor(l.url) });
  }
  const reply = await send({ type: 'add', items, userAgent: navigator.userAgent });
  if (!reply.ok) throw new Error(reply.error || 'the app refused the links');
}

/** What to tell the user about an error from the helper or the browser. */
export function reason(e) {
  const text = String(e?.message || e || '');
  if (/native messaging host|native host|not found|forbidden/i.test(text)) {
    return "Can't reach the app — install Orchard Download Manager and open it once, then try again";
  }
  return text || "The app didn't answer — open it, then try again";
}
