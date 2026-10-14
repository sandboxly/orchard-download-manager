import * as options from './common/options.js';

const $ = (id) => document.getElementById(id);
let timer;

function saved() {
  $('saved').hidden = false;
  clearTimeout(timer);
  timer = setTimeout(() => ($('saved').hidden = true), 1500);
}

async function init() {
  const opts = await options.load();
  $('takeOver').checked = opts.takeOver;
  $('minSizeKB').value = opts.minSizeKB;
  $('skipSites').value = opts.skipSites;
  $('skipTypes').value = opts.skipTypes;
  $('takeOver').addEventListener('change', async (e) => {
    await options.save({ takeOver: e.target.checked });
    saved();
  });
  $('minSizeKB').addEventListener('change', async (e) => {
    const n = Math.max(0, Math.floor(Number(e.target.value) || 0));
    e.target.value = n;
    await options.save({ minSizeKB: n });
    saved();
  });
  for (const id of ['skipSites', 'skipTypes']) {
    $(id).addEventListener('change', async (e) => {
      await options.save({ [id]: e.target.value });
      saved();
    });
  }
}

init();
