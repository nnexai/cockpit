const $ = (selector) => document.querySelector(selector);
const app = $('#app');
const atlas = '../../product-atlas-2026-09-12/workflow/runs/atlas-exhaustive-r2-20260912/evidence/';
// The order and labels are transcribed from the cited desktop frame.
const agents = ['Atlas Alpha · Context', 'Atlas Beta · Recovery', 'Atlas Beta · Delivery', 'Atlas Alpha · Review'];
$('#agent-list').innerHTML = agents.map((location, i) => `<button class="agent-row${i === 3 ? ' selected' : ''}" data-existing="Focus ${location}" ${i === 3 ? 'aria-current="true"' : ''} title="${location} · codex · idle"><span class="glyph" aria-hidden="true">○</span><span class="agent-details"><span class="agent-location">${location}</span><span class="agent-meta"><span>codex</span><span class="agent-status">idle</span></span></span></button>`).join('');
const params = new URLSearchParams(location.search);
if (params.get('capture') === '1') document.body.classList.add('capture-mode');
let drawerReturn = null;
let modalReturn = null;
let retryTimer = null;
let currentScenario = 'live';
let railWidth = 224;
function resizeRail(width) {
  railWidth = Math.max(192, Math.min(360, width));
  app.style.setProperty('--rail-width', `${railWidth}px`);
  $('#rail-resizer').setAttribute('aria-valuenow', String(railWidth));
}
function note(message) { $('#study-note').textContent = message; }
function drawerIsAvailable() { return innerWidth <= ($('#direction').value === 'drawer' ? 800 : 600); }
function closeDrawer(restore = true) {
  app.classList.remove('drawer-open');
  $('#drawer-scrim').hidden = true;
  $('#workarea').inert = false;
  $('#drawer-toggle').setAttribute('aria-expanded', 'false');
  $('#sidebar').removeAttribute('role');
  $('#sidebar').removeAttribute('aria-modal');
  if (restore) drawerReturn?.focus({preventScroll:true});
}
function openDrawer() {
  if (!drawerIsAvailable()) { app.classList.remove('rail-hidden'); $('#sidebar-collapse').focus(); return; }
  drawerReturn = document.activeElement;
  app.classList.add('drawer-open');
  $('#drawer-scrim').hidden = false;
  $('#workarea').inert = true;
  $('#sidebar').setAttribute('role', 'dialog');
  $('#sidebar').setAttribute('aria-modal', 'true');
  $('#drawer-toggle').setAttribute('aria-expanded', 'true');
  $('#drawer-close').focus({preventScroll:true});
}
function setDirection(value) {
  if (!['rail','drawer','before'].includes(value)) value = 'drawer';
  $('#direction').value = value;
  app.dataset.direction = value;
  closeDrawer(false);
  $('#before').hidden = value !== 'before';
  app.hidden = value === 'before';
  $('#scenario').disabled = value === 'before';
  updateViewport();
}
function updateViewport() {
  $('#viewport').textContent = `${innerWidth} × ${innerHeight}`;
  const width = innerWidth <= 500 ? 480 : innerWidth <= 650 ? 600 : innerWidth <= 850 ? 800 : 1440;
  const height = width === 800 ? 1000 : 900;
  $('#before-image').src = atlas + (width === 1440 ? 'fixture-r2/cockpit-atlas-1440x900.png' : `portrait-sidebar-r2/cockpit-sidebar-${width}x${height}.png`);
  if (!drawerIsAvailable()) closeDrawer(false);
}
function setScenario(value) {
  const allowed = ['live','observing','loading','empty','stale','disconnected','error','recovering'];
  if (!allowed.includes(value)) value = 'live';
  clearTimeout(retryTimer);
  currentScenario = value;
  $('#scenario').value = value;
  app.dataset.scenario = value;
  $('#ownership').hidden = value !== 'observing';
  $('#agent-list').hidden = value === 'empty';
  $('.empty-agents').hidden = value !== 'empty';
  $('#initial-state').hidden = !['loading','error'].includes(value);
  $('#initial-message').textContent = value === 'loading' ? 'Loading session…' : 'Could not load the session.';
  $('#initial-retry').hidden = value !== 'error';
  const messages = {stale:'Session state is stale. Showing the last received view.', disconnected:'Disconnected. Showing the last received view.', recovering:'Reconnecting… Showing the last received view.'};
  $('#recovery').hidden = !messages[value];
  $('#recovery-message').textContent = messages[value] || '';
  $('#retry').textContent = value === 'stale' ? 'Resync' : value === 'recovering' ? 'Reconnecting…' : 'Reconnect';
  $('#retry').disabled = value === 'recovering';
  app.querySelectorAll('[data-existing]').forEach(button => {button.disabled = ['loading','error','stale','disconnected','recovering'].includes(value);});
  note(value === 'live' ? 'Proposal only. Terminal interiors are unmodified atlas pixels, clipped to fit. No live Herdr actions.' : `Illustrative ${value} state. Agent status and terminal pixels stay unchanged. No live Herdr actions.`);
}
function retry() {
  setScenario('recovering');
  retryTimer = setTimeout(() => {setScenario('live');note('Illustrative recovery completed. A real client must wait for a fresh snapshot and terminal baseline.');}, 800);
}
function openDialog(selector, modal = true) {
  const dialog = $(selector);
  modalReturn = document.activeElement;
  if (modal) dialog.showModal(); else dialog.show();
  (dialog.querySelector('input') || dialog.querySelector('button'))?.focus({preventScroll:true});
}
function closeDialog(dialog) {dialog.close();modalReturn?.focus({preventScroll:true});}
$('#direction').addEventListener('change', event => setDirection(event.target.value));
$('#scenario').addEventListener('change', event => setScenario(event.target.value));
$('#drawer-toggle').addEventListener('click', openDrawer);
$('#drawer-close').addEventListener('click', () => closeDrawer());
$('#drawer-scrim').addEventListener('click', () => closeDrawer());
$('#sidebar-collapse').addEventListener('click', () => {app.classList.add('rail-hidden');$('#drawer-toggle').focus();});
$('#rail-resizer').addEventListener('keydown', event => {
  if (!['ArrowLeft','ArrowRight','Home'].includes(event.key)) return;
  event.preventDefault();resizeRail(event.key === 'Home' ? 224 : railWidth + (event.key === 'ArrowLeft' ? -8 : 8));
});
$('#rail-resizer').addEventListener('pointerdown', event => {
  event.preventDefault();const startX = event.clientX;const startWidth = railWidth;const handle = event.currentTarget;
  handle.setPointerCapture(event.pointerId);
  const move = next => resizeRail(startWidth + next.clientX - startX);
  const stop = () => {handle.removeEventListener('pointermove',move);handle.removeEventListener('pointerup',stop);handle.removeEventListener('pointercancel',stop);};
  handle.addEventListener('pointermove',move);handle.addEventListener('pointerup',stop);handle.addEventListener('pointercancel',stop);
});
$('#notes-button').addEventListener('click', () => {const open = $('#notes').hidden;$('#notes').hidden = !open;$('#notes-button').setAttribute('aria-expanded', String(open));});
$('#commands-button').addEventListener('click', () => openDialog('#commands-dialog'));
function resourceMenu(kind, name, x, y) {
  const items = kind === 'Space' ? ['Rename Space','Open browser','Browser feedback','Close Space'] : kind === 'tab' ? ['Rename tab','Close tab'] : ['Copy selection','Paste','Open files right','Open Context right','Open Review right','Split right','Split below','Zoom pane','Move pane','Close pane'];
  $('#resource-title').textContent = name;
  $('#resource-actions').replaceChildren(...items.map(label => {const button = document.createElement('button');button.textContent = label;button.dataset.existing = `${label} · ${name}`;return button;}));
  const dialog = $('#resource-dialog');modalReturn = document.activeElement;
  dialog.style.left = `${Math.max(8, Math.min(x, innerWidth - 248))}px`;
  dialog.style.top = `${Math.max(8, Math.min(y, innerHeight - 440))}px`;
  dialog.show();dialog.querySelector('button').focus();
}
document.querySelectorAll('[data-pane-menu]').forEach(button => button.addEventListener('click', event => resourceMenu('pane',button.dataset.paneMenu,event.clientX,event.clientY)));
app.addEventListener('contextmenu', event => {
  const target = event.target.closest('.space-row,.tab,.pane');if (!target) return;
  event.preventDefault();const kind = target.matches('.space-row') ? 'Space' : target.matches('.tab') ? 'tab' : 'pane';
  const name = kind === 'pane' ? target.querySelector('.pane-label>span').textContent : target.textContent.trim();
  resourceMenu(kind,name,event.clientX,event.clientY);
});
$('#command-search').addEventListener('input', event => {
  const query = event.target.value.toLowerCase();
  const rows = [...$('#command-actions').children,...$('.secondary-actions').children];
  rows.forEach(row => {row.hidden = !row.textContent.toLowerCase().includes(query);});
  $('#command-empty').hidden = rows.some(row => !row.hidden);
});
$('#feedback-open').addEventListener('click', () => {$('#commands-dialog').close();modalReturn = $('#commands-button');$('#feedback-dialog').show();$('#refresh-feedback').focus();});
$('#refresh-feedback').addEventListener('click', () => {$('#feedback-message').textContent = 'Reading pending feedback…';setTimeout(() => {$('#feedback-message').textContent = 'No pending annotations.';}, 600);});
$('#shortcuts-toggle').addEventListener('click', () => {const open = $('#shortcuts').hidden;$('#shortcuts').hidden = !open;$('#shortcuts-toggle').setAttribute('aria-expanded', String(open));});
document.querySelectorAll('.terminal-content').forEach(pane => pane.addEventListener('click', () => {if (currentScenario === 'observing') {setScenario('live');note('Illustrative normal selection acquires control. No second action or success toast. No Herdr request was made.');}}));
$('#retry').addEventListener('click', retry);
$('#initial-retry').addEventListener('click', retry);
document.addEventListener('click', event => {
  const close = event.target.closest('[data-close]');
  if (close) closeDialog(close.closest('dialog'));
  const action = event.target.closest('[data-existing]');
  if (!action || action.disabled) return;
  note(`Existing action: ${action.dataset.existing}. The recorded selection and pane layout remain fixed in this study.`);
  const dialog = action.closest('dialog');
  if (dialog) closeDialog(dialog);
});
document.querySelectorAll('dialog').forEach(dialog => dialog.addEventListener('cancel', event => {event.preventDefault();closeDialog(dialog);}));
document.addEventListener('keydown', event => {
  if (event.key === 'Escape') {
    if (!$('#notes').hidden) {$('#notes').hidden = true;$('#notes-button').setAttribute('aria-expanded','false');return;}
    if (app.classList.contains('drawer-open')) {event.preventDefault();closeDrawer();return;}
    for (const selector of ['#feedback-dialog','#resource-dialog']) if ($(selector).open) {event.preventDefault();closeDialog($(selector));}
  }
  if (event.key === 'Tab' && app.classList.contains('drawer-open')) {
    const controls = [...$('#sidebar').querySelectorAll('button:not(:disabled)')].filter(item => item.getClientRects().length);
    const index = controls.indexOf(document.activeElement);
    event.preventDefault();controls[(index + (event.shiftKey ? -1 : 1) + controls.length) % controls.length]?.focus();
  }
});
document.addEventListener('pointerdown', event => {const dialog = $('#resource-dialog');if (dialog.open && !dialog.contains(event.target) && !event.target.closest('[data-pane-menu]')) closeDialog(dialog);});
window.addEventListener('resize', updateViewport);
setDirection(params.get('direction') || 'drawer');
setScenario(params.get('state') || 'live');
if (params.get('panel') === 'feedback') $('#feedback-dialog').show();
if (params.get('panel') === 'commands') openDialog('#commands-dialog');
if (params.get('drawer') === 'open') openDrawer();
