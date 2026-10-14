import * as app from './common/app.js';
import * as options from './common/options.js';
import * as access from './common/access.js';

const $ = (id) => document.getElementById(id);

async function init() {
  const opts = await options.load();
  $('takeOver').checked = opts.takeOver;
  $('takeOver').addEventListener('change', (e) => options.save({ takeOver: e.target.checked }));
  await access.bind($('allSites'));
  $('settings').addEventListener('click', (e) => {
    e.preventDefault();
    chrome.runtime.openOptionsPage();
  });
  $('version').textContent = 'Extension ' + chrome.runtime.getManifest().version;

  try {
    const version = await app.ping();
    $('status').textContent = `Connected to the app (version ${version})`;
    $('status').className = 'ok';
    const { problem } = await chrome.storage.session.get('problem');
    if (problem) show(problem);
  } catch (e) {
    $('status').textContent = "Can't reach the app";
    $('status').className = 'err';
    show('Install Orchard Download Manager and open it once — that connects it to the browser. Until then, the browser downloads files itself.');
  }
}

function show(text) {
  $('help').textContent = text;
  $('help').hidden = false;
}

init();
