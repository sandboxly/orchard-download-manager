// node --test extension/test/*.test.mjs
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { DEFAULTS } from '../chrome/common/options.js';
import { extension, list, siteMatches, skipReason } from '../chrome/common/rules.js';

const d = (over) => ({ url: 'https://files.example.org/a/b.zip', filename: '/Users/me/Downloads/b.zip', fileSize: 5_000_000, ...over });

test('lists and file types', () => {
  assert.deepEqual(list('example.org\n *.cdn.org, .PDF ;jpg'), ['example.org', 'cdn.org', 'pdf', 'jpg']);
  assert.equal(extension('C:\\x\\Report.PDF'), 'pdf');
  assert.equal(extension('.bashrc'), '');
  assert.equal(extension('archive.tar.gz'), 'gz');
  assert.ok(siteMatches('dl.example.org', ['example.org']));
  assert.ok(!siteMatches('notexample.org', ['example.org']));
});

test('the app takes ordinary downloads', () => {
  assert.equal(skipReason(d(), DEFAULTS), null);
  assert.equal(skipReason(d({ fileSize: -1 }), { ...DEFAULTS, minSizeKB: 100 }), null, 'unknown sizes go to the app');
});

test('what stays with the browser', () => {
  assert.equal(skipReason(d(), { ...DEFAULTS, takeOver: false }), 'off');
  assert.equal(skipReason(d({ url: 'blob:https://x.org/1' }), DEFAULTS), 'not a web link');
  assert.equal(skipReason(d({ url: 'data:text/plain,hi' }), DEFAULTS), 'not a web link');
  assert.equal(skipReason(d({ incognito: true }), DEFAULTS), 'private window');
  assert.equal(skipReason(d({ byExtensionId: 'abc' }), DEFAULTS), 'started by an extension');
  assert.equal(skipReason(d(), { ...DEFAULTS, skipSites: 'example.org' }), 'skipped site');
  assert.equal(skipReason(d(), { ...DEFAULTS, skipTypes: 'pdf zip' }), 'skipped file type');
  assert.equal(skipReason(d({ filename: '' }), { ...DEFAULTS, skipTypes: 'zip' }), 'skipped file type', 'the type from the link');
  assert.equal(skipReason(d({ fileSize: 10 * 1024 }), { ...DEFAULTS, minSizeKB: 100 }), 'small file');
});
