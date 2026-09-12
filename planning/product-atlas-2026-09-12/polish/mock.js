'use strict';

// Standalone design study. No application APIs, storage, or resource mutations.
const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => [...document.querySelectorAll(selector)];
const frame = $('#workbench-frame');
const scenes = ['workbench', 'review', 'setup', 'browser'];
let currentScene = 'workbench';
let savedComment = '';
let toastTimer;

function closeTransientUI() {
  $$('dialog[open]').forEach((dialog) => dialog.close());
  $('#pane-menu').hidden = true;
  $('#pane-button').setAttribute('aria-expanded', 'false');
}

function showScene(name) {
  if (!scenes.includes(name)) return;
  closeTransientUI();
  currentScene = name;
  $$('.scene').forEach((scene) => {
    const active = scene.id === `scene-${name}`;
    scene.hidden = !active;
    scene.classList.toggle('active', active);
  });
  $$('[data-scene]').forEach((button) => {
    button.setAttribute('aria-pressed', String(button.dataset.scene === name));
  });
  history.replaceState(null, '', `#${name}`);
}

function notify(message) {
  clearTimeout(toastTimer);
  const toast = $('#study-toast');
  toast.textContent = message;
  toast.hidden = false;
  toastTimer = setTimeout(() => { toast.hidden = true; }, 4500);
}

function openDialog(id) {
  closeTransientUI();
  const dialog = document.getElementById(id);
  dialog.showModal();
  if (id === 'commands-dialog') {
    $('#command-search').value = '';
    filterCommands();
    $('#command-search').focus();
  } else if (id === 'files-dialog') {
    const review = currentScene === 'review';
    $('#choose-issue strong').textContent = review ? 'CONTEXT.md' : 'Fix OMP terminal cell-width drift';
    $('#choose-issue small').textContent = review ? 'Working tree · unstaged changes' : 'nnexai/cockpit #4 · sources/provider-996e1f7…';
    $('#choose-issue > .push-right').textContent = review ? 'Modified' : 'Issue';
    $('#file-search').value = '';
    filterFiles();
    $('#file-search').focus();
  }
}

function expandDocument() {
  if (currentScene !== 'workbench') showScene('workbench');
  const expanded = frame.classList.toggle('document-expanded');
  $('#expand-document').innerHTML = `<svg aria-hidden="true"><use href="#i-expand"/></svg> ${expanded ? 'Restore split study' : 'Expand document'}`;
  $('#expand-document').setAttribute('aria-pressed', String(expanded));
  closeTransientUI();
}

function toggleSidebar() {
  if (matchMedia('(max-width: 800px)').matches) {
    frame.classList.toggle('drawer-open');
  } else {
    frame.classList.toggle('sidebar-collapsed');
  }
}

function toggleDetails() {
  const panel = $('#metadata-panel');
  panel.hidden = !panel.hidden;
  $('#document-details').setAttribute('aria-expanded', String(!panel.hidden));
  closeTransientUI();
}

function filterCommands() {
  const query = $('#command-search').value.trim().toLowerCase();
  let count = 0;
  $$('[data-command]').forEach((button) => {
    const matches = `${button.dataset.command} ${button.textContent}`.toLowerCase().includes(query);
    button.hidden = !matches;
    if (matches) count += 1;
  });
  $('#no-commands').hidden = count > 0;
}

function filterFiles() {
  const query = $('#file-search').value.trim().toLowerCase();
  const matches = $('#choose-issue').textContent.toLowerCase().includes(query);
  $('#choose-issue').hidden = !matches;
  $('#no-files').hidden = matches;
}

function openComment() {
  const dialog = $('#comment-editor');
  if (dialog.open) return $('#comment-text').focus();
  if (matchMedia('(max-width: 600px)').matches) dialog.showModal();
  else dialog.show();
  $('#comment-text').focus();
  if (!matchMedia('(max-width: 600px)').matches) dialog.scrollIntoView({ block: 'nearest' });
}

function renderDraft() {
  $('#saved-comment').hidden = !savedComment;
  $('#saved-comment-text').textContent = savedComment;
  $('#draft-count').textContent = savedComment ? '1' : '0';
  $('#open-comment').innerHTML = `<svg aria-hidden="true"><use href="#i-comment"/></svg> ${savedComment ? 'Edit comment' : 'Add comment'} <kbd>C</kbd>`;
}

function saveComment() {
  const value = $('#comment-text').value.trim();
  if (!value) {
    $('#comment-text').setCustomValidity('Write a comment before keeping a local draft.');
    $('#comment-text').reportValidity();
    return;
  }
  savedComment = value;
  renderDraft();
  $('#comment-editor').close();
  $('#saved-comment').scrollIntoView({ block: 'nearest' });
  notify('Draft kept in this page only. Nothing was saved to disk, pasted, or sent.');
}

$$('[data-scene]').forEach((button) => button.addEventListener('click', () => showScene(button.dataset.scene)));
$$('[data-switch]').forEach((button) => button.addEventListener('click', () => showScene(button.dataset.switch)));
$$('[data-dialog]').forEach((button) => button.addEventListener('click', () => openDialog(button.dataset.dialog)));
$$('[data-close]').forEach((button) => button.addEventListener('click', () => button.closest('dialog').close()));
$('#toggle-sidebar').addEventListener('click', toggleSidebar);
$('#close-sidebar').addEventListener('click', () => frame.classList.remove('drawer-open'));
$('#expand-document').addEventListener('click', expandDocument);
$('#menu-expand').addEventListener('click', expandDocument);
$('#command-expand').addEventListener('click', expandDocument);
$('#document-details').addEventListener('click', toggleDetails);
$('#menu-details').addEventListener('click', toggleDetails);
$('#pane-button').addEventListener('click', () => {
  const menu = $('#pane-menu');
  menu.hidden = !menu.hidden;
  $('#pane-button').setAttribute('aria-expanded', String(!menu.hidden));
});
$('#preview-button').addEventListener('click', () => {
  $('#document-preview').hidden = false;
  $('#document-source').hidden = true;
  $('#preview-button').setAttribute('aria-pressed', 'true');
  $('#source-button').setAttribute('aria-pressed', 'false');
});
$('#source-button').addEventListener('click', () => {
  $('#document-preview').hidden = true;
  $('#document-source').hidden = false;
  $('#preview-button').setAttribute('aria-pressed', 'false');
  $('#source-button').setAttribute('aria-pressed', 'true');
});
$('#command-search').addEventListener('input', filterCommands);
$('#file-search').addEventListener('input', filterFiles);
$('#command-search').addEventListener('keydown', (event) => {
  if (event.key === 'Enter') {
    const first = $$('[data-command]').find((button) => !button.hidden);
    first?.click();
  }
});
$('#file-search').addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && !$('#choose-issue').hidden) $('#choose-issue').click();
});
$('[data-session-command]').addEventListener('click', () => openDialog('session-dialog'));
$('#choose-issue').addEventListener('click', () => {
  closeTransientUI();
  if (currentScene === 'review') $('.diff-heading').scrollIntoView({ block: 'nearest' });
  else {
    $('#preview-button').click();
    $('#document-preview').scrollIntoView({ block: 'nearest' });
  }
});
function toggleFileList(container, tree, button) {
  const opening = getComputedStyle(tree).display === 'none';
  container.classList.toggle('tree-hidden', !opening);
  container.classList.toggle('tree-open', opening);
  button.setAttribute('aria-expanded', String(opening));
}
$('#review-files-button').addEventListener('click', () => toggleFileList($('.review-content'), $('#review-files'), $('#review-files-button')));
$('#viewer-files-button').addEventListener('click', () => toggleFileList($('#files-body'), $('#viewer-files'), $('#viewer-files-button')));
$('#viewer-tree-issue').addEventListener('click', () => {
  $('#preview-button').click();
  if ($('.context-pane').clientWidth <= 500) toggleFileList($('#files-body'), $('#viewer-files'), $('#viewer-files-button'));
});
const fileNavigationObserver = new ResizeObserver(() => {
  $('#viewer-files-button').setAttribute('aria-expanded', String(getComputedStyle($('#viewer-files')).display !== 'none'));
  $('#review-files-button').setAttribute('aria-expanded', String(getComputedStyle($('#review-files')).display !== 'none'));
});
fileNavigationObserver.observe($('.context-pane'));
fileNavigationObserver.observe($('.review-content'));

// Local metadata examples only; no provider lookup is performed by this mock.
const setupSources = new Map([
  ['https://github.com/nnexai/cockpit/issues/4', { repository: 'cockpit', branch: 'issue-4-terminal-width' }],
  ['https://github.com/nnexai/cockpit/pull/7', { repository: 'cockpit', branch: 'ui-polish' }]
]);
let setupBranch = 'atlas-app-captures';
let customSpaceName = false;
let customDestination = false;
let resolvedSetupSource = '';

function syncSetupName() {
  const existing = $('#setup-existing').getAttribute('aria-pressed') === 'true';
  const value = $('#setup-branch').value.trim();
  const defaultName = existing ? value.split('/').filter(Boolean).at(-1) || '' : value;
  if (!customSpaceName) $('#setup-name').value = defaultName;
  if (!existing && !customDestination) $('#setup-destination').value = `/tmp/ca12/worktrees/${value.replaceAll('/', '-')}`;
}

function applySetupSource() {
  let source;
  try {
    const url = new URL($('#setup-source').value.trim());
    source = `${url.origin}${url.pathname.replace(/\/$/, '')}`;
  } catch {
    return;
  }
  const metadata = setupSources.get(source);
  if (!metadata || source === resolvedSetupSource) return;
  resolvedSetupSource = source;
  $('#setup-repository').value = metadata.repository;
  setupBranch = metadata.branch;
  if ($('#setup-new').getAttribute('aria-pressed') === 'true') $('#setup-branch').value = setupBranch;
  syncSetupName();
}

function setupOperation(existing) {
  $('#setup-new').setAttribute('aria-pressed', String(!existing));
  $('#setup-existing').setAttribute('aria-pressed', String(existing));
  $('#setup-branch-label').textContent = existing ? 'Checkout' : 'Branch';
  $('#setup-branch').value = existing ? '/tmp/ca12/repos/cockpit' : setupBranch;
  $('#setup-create').textContent = existing ? 'Open Space' : 'Create Space';
  $('#setup-effects').textContent = existing ? 'Open the existing checkout; create associated context and a separate context terminal.' : 'Linked worktree, Herdr Space, companion context, separate context terminal.';
  syncSetupName();
}
$('#setup-new').addEventListener('click', () => setupOperation(false));
$('#setup-existing').addEventListener('click', () => setupOperation(true));
$('#setup-source').addEventListener('input', () => {
  if (!$('#setup-source').value.trim()) resolvedSetupSource = '';
  applySetupSource();
});
$('#setup-branch').addEventListener('input', () => {
  if ($('#setup-new').getAttribute('aria-pressed') === 'true') setupBranch = $('#setup-branch').value;
  syncSetupName();
});
$('#setup-name').addEventListener('input', () => {
  customSpaceName = $('#setup-name').value.trim() !== '' && $('#setup-name').value !== $('#setup-branch').value;
});
$('#setup-destination').addEventListener('input', () => { customDestination = true; });
applySetupSource();
$('#unified-button').addEventListener('click', () => {
  $('.extra-source').hidden = true;
  $('#unified-button').setAttribute('aria-pressed', 'true');
  $('#full-source-button').setAttribute('aria-pressed', 'false');
});
$('#full-source-button').addEventListener('click', () => {
  $('.extra-source').hidden = false;
  $('#unified-button').setAttribute('aria-pressed', 'false');
  $('#full-source-button').setAttribute('aria-pressed', 'true');
});
$('#open-comment').addEventListener('click', () => {
  if (savedComment) $('#comment-text').value = savedComment;
  openComment();
});
$('#selected-code-line').addEventListener('click', openComment);
$('#save-comment').addEventListener('click', saveComment);
$('#comment-text').addEventListener('input', () => $('#comment-text').setCustomValidity(''));
$('#comment-text').addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
    event.preventDefault();
    saveComment();
  }
});
$('#edit-comment').addEventListener('click', () => {
  $('#comment-text').value = savedComment;
  openComment();
});
$('#delete-comment').addEventListener('click', () => {
  savedComment = '';
  $('#comment-text').value = '';
  renderDraft();
  notify('Removed the local study draft. No product data was changed.');
});
$('#show-drafts').addEventListener('click', () => {
  if (savedComment) $('#saved-comment').scrollIntoView({ block: 'center' });
  else notify('No local study drafts yet. Select Add comment to try the composer.');
});
let annotationMode = 'element';
let drawing = null;
const drawingLayer = $('#drawing-layer');

function selectAnnotationMode(mode) {
  annotationMode = mode;
  drawingLayer.classList.toggle('is-drawing', mode === 'freehand' || mode === 'region');
  $('#idea-anchor').hidden = mode !== 'element';
  if (mode !== 'element') closeIdea();
}
$$('[data-mode]').forEach((button) => button.addEventListener('click', () => {
  $$('[data-mode]').forEach((mode) => {
    const active = mode === button;
    mode.classList.toggle('active-tool', active);
    mode.setAttribute('aria-pressed', String(active));
  });
  selectAnnotationMode(button.dataset.mode);
}));
const ideaNotes = [];
const ideaLocations = [];
let activeNoteLocation = { left: '', top: '', label: 'Repository health' };
let editingIdea = null;

function setNotesSidebar(open) {
  $('#notes-study').classList.toggle('notes-hidden', !open);
  $('#toggle-notes').setAttribute('aria-expanded', String(open));
}

function openIdea(index = null) {
  editingIdea = index;
  if (index !== null) activeNoteLocation = ideaLocations[index];
  $('#annotation-editor').style.left = activeNoteLocation.left;
  $('#annotation-editor').style.top = activeNoteLocation.top;
  $('#annotation-text').value = index === null ? '' : ideaNotes[index];
  $('#annotation-save').textContent = 'Done';
  $('#annotation-editor').hidden = false;
  $('#inline-note-summary').hidden = true;
  if (innerWidth <= 600) setNotesSidebar(false);
  $('#annotation-editor').scrollIntoView({ block: 'nearest' });
  $('#annotation-text').focus();
}

function closeIdea() {
  $('#annotation-editor').hidden = true;
  $('#inline-note-summary').hidden = ideaNotes.length === 0;
}

function renderIdeas() {
  $('#notes-count').textContent = String(ideaNotes.length);
  $('#sidebar-note-count').textContent = String(ideaNotes.length);
  const list = $('#notes-list');
  list.replaceChildren();
  ideaNotes.forEach((text, index) => {
    const item = document.createElement('button');
    item.className = 'note-list-item';
    item.innerHTML = '<span class="annotation-number"></span><span><strong></strong><span class="note-text"></span></span><svg aria-hidden="true"><use href="#i-chevron"/></svg>';
    item.querySelector('strong').textContent = ideaLocations[index].label;
    item.querySelector('.annotation-number').textContent = String(index + 1);
    item.querySelector('.note-text').textContent = text;
    item.addEventListener('click', () => openIdea(index));
    list.append(item);
  });
  if (!ideaNotes.length) {
    const empty = document.createElement('p');
    empty.className = 'notes-intro';
    empty.textContent = 'No notes.';
    list.append(empty);
  } else {
    const index = editingIdea ?? ideaNotes.length - 1;
    $('#inline-note-text').textContent = ideaNotes[index];
    $('#inline-note-summary .annotation-number').textContent = String(index + 1);
    $('#inline-note-summary').style.left = ideaLocations[index].left;
    $('#inline-note-summary').style.top = ideaLocations[index].top;
  }
}

function addIdea() {
  const text = $('#annotation-text').value.trim();
  if (!text) {
    $('#annotation-text').setCustomValidity('Write an idea first.');
    $('#annotation-text').reportValidity();
    return;
  }
  if (editingIdea === null) {
    ideaNotes.push(text);
    ideaLocations.push({ ...activeNoteLocation });
    editingIdea = ideaNotes.length - 1;
  } else {
    ideaNotes[editingIdea] = text;
  }
  renderIdeas();
  closeIdea();
}

$('#idea-anchor').addEventListener('click', () => {
  activeNoteLocation = { left: '', top: '', label: 'Repository health' };
  openIdea();
});
$('#inline-note-summary').addEventListener('click', () => openIdea(editingIdea ?? ideaNotes.length - 1));
$('#annotation-save').addEventListener('click', addIdea);
$('#annotation-cancel').addEventListener('click', closeIdea);
$('#toggle-notes').addEventListener('click', () => setNotesSidebar($('#notes-study').classList.contains('notes-hidden')));
$('#close-notes').addEventListener('click', () => setNotesSidebar(false));
$('#annotation-text').addEventListener('input', () => $('#annotation-text').setCustomValidity(''));
$('#annotation-text').addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
    event.preventDefault();
    addIdea();
  }
});

function drawingPoint(event) {
  const rect = drawingLayer.getBoundingClientRect();
  return {
    x: Math.max(0, Math.min(1040, (event.clientX - rect.left) * 1040 / rect.width)),
    y: Math.max(0, Math.min(388, (event.clientY - rect.top) * 388 / rect.height))
  };
}

drawingLayer.addEventListener('pointerdown', (event) => {
  if (event.button !== 0 || !['freehand', 'region'].includes(annotationMode)) return;
  event.preventDefault();
  drawingLayer.setPointerCapture(event.pointerId);
  const point = drawingPoint(event);
  const node = document.createElementNS('http://www.w3.org/2000/svg', annotationMode === 'freehand' ? 'path' : 'rect');
  drawing = { node, start: point, path: `M${point.x} ${point.y}`, mode: annotationMode };
  if (annotationMode === 'freehand') node.setAttribute('d', drawing.path);
  else {
    node.setAttribute('x', point.x);
    node.setAttribute('y', point.y);
    node.setAttribute('fill', '#619bed12');
  }
  drawingLayer.append(node);
});
drawingLayer.addEventListener('pointermove', (event) => {
  if (!drawing) return;
  const point = drawingPoint(event);
  if (drawing.mode === 'freehand') {
    drawing.path += ` L${point.x} ${point.y}`;
    drawing.node.setAttribute('d', drawing.path);
  } else {
    drawing.node.setAttribute('x', Math.min(point.x, drawing.start.x));
    drawing.node.setAttribute('y', Math.min(point.y, drawing.start.y));
    drawing.node.setAttribute('width', Math.abs(point.x - drawing.start.x));
    drawing.node.setAttribute('height', Math.abs(point.y - drawing.start.y));
  }
});
drawingLayer.addEventListener('pointerup', (event) => {
  if (!drawing) return;
  const completed = drawing;
  drawing = null;
  if (drawingLayer.hasPointerCapture(event.pointerId)) drawingLayer.releasePointerCapture(event.pointerId);
  if (completed.mode === 'region') {
    const point = drawingPoint(event);
    const left = Math.min(point.x, completed.start.x) / 1040 * 100;
    const top = Math.max(point.y, completed.start.y) / 388 * 100;
    activeNoteLocation = { left: `${Math.min(left, 50)}%`, top: `${Math.min(top + 2, 60)}%`, label: 'Region' };
    openIdea();
  }
});
drawingLayer.addEventListener('pointercancel', () => {
  drawing?.node.remove();
  drawing = null;
});
$('#clear-annotations').addEventListener('click', () => {
  drawingLayer.replaceChildren();
  ideaNotes.length = 0;
  ideaLocations.length = 0;
  editingIdea = null;
  $('#annotation-text').value = '';
  renderIdeas();
  closeIdea();
});
$('#close-toolbar').addEventListener('click', () => {
  $('#annotation-toolbar').hidden = true;
  $('#reopen-toolbar').hidden = false;
  selectAnnotationMode('browse');
  closeIdea();
});
$('#reopen-toolbar').addEventListener('click', () => {
  $('#annotation-toolbar').hidden = false;
  $('#reopen-toolbar').hidden = true;
  $('[data-mode="element"]').click();
});
renderIdeas();

document.addEventListener('click', (event) => {
  if (!event.target.closest('#pane-menu, #pane-button')) {
    $('#pane-menu').hidden = true;
    $('#pane-button').setAttribute('aria-expanded', 'false');
  }
});
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    closeTransientUI();
    frame.classList.remove('drawer-open');
    $('#review-files').classList.remove('open');
    if (currentScene === 'browser') {
      closeIdea();
      setNotesSidebar(false);
    }
  }
  const editing = event.target.closest('input, textarea, [contenteditable="true"]');
  if (currentScene === 'review' && !editing && !event.ctrlKey && !event.metaKey && event.key === 'c') {
    event.preventDefault();
    openComment();
  }
});

// Normalize study-only navigation when resizing; never model a Herdr layout change.
matchMedia('(max-width: 800px)').addEventListener('change', () => {
  frame.classList.remove('drawer-open', 'sidebar-collapsed');
  $('#review-files').hidden = false;
  $('#review-files').classList.remove('open');
});
matchMedia('(max-width: 600px)').addEventListener('change', () => {
  if ($('#comment-editor').open) {
    $('#comment-editor').close();
    openComment();
  }
});
showScene(scenes.includes(location.hash.slice(1)) ? location.hash.slice(1) : 'workbench');
