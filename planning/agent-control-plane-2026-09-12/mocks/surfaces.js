const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => [...document.querySelectorAll(selector)];
const mock = $('.mock');
const surface = $('#surface');
const state = $('#state');
const note = $('#state-message');
const labStatus = $('#lab-status');
const files = {
  'src/App.tsx': 'export function App({ client }: Props) {\n  const [state, dispatch] = useReducer(sessionReducer, initialSessionState);\n  const selection = authoritativeSelection(state.snapshot);\n  return <Workbench client={client} state={state}\n    selection={selection} />;\n}',
  'src/app/styles.css': '.workbench {\n  display: grid;\n  grid-template-columns: 224px minmax(0, 1fr);\n}\n\n.pane-canvas { min-width: 0; }',
  'docs/decisions.md': '# UI decisions\n\nKeep Herdr selection and pane layout authoritative.\n\nGraphical panes retain local drafts.',
  'README.md': '# Cockpit\n\nA compact control console for Herdr sessions.\n\nRun focused verification before delivery.',
};
const diffs = {
  app: ['src/app/App.tsx', '<span class="hunk">@@ -38,3 +38,4 @@</span><span> export function App({ client }: Props) {</span><span class="deleted">-  return &lt;LegacyShell /&gt;;</span><span class="added">+  const selection = authoritativeSelection(snapshot);</span><span class="added">+  return &lt;Workbench client={client} selection={selection} /&gt;;</span><span> }</span>'],
  decision: ['docs/decision.md', '<span class="hunk">@@ -0,0 +1,3 @@</span><span class="added">+# UI decisions</span><span class="added">+Keep Herdr selection authoritative.</span><span class="added">+Graphical panes retain drafts.</span>'],
  styles: ['src/app/styles.css', '<span class="hunk">@@ -14,3 +14,3 @@</span><span> .pane-canvas {</span><span class="deleted">-  min-width: auto;</span><span class="added">+  min-width: 0;</span><span> }</span>'],
};
const notices = { ready: '', loading: 'Loading the selected resource. Local drafts and selection remain available.', empty: '', stale: 'Showing the last displayed resource. Refresh when the connection recovers.', disconnected: 'Disconnected. You can still read the last displayed resource; writes are disabled.', error: 'The selected resource could not be read. Retry when it is available.' };
function lab(message) { labStatus.textContent = message; }
function setState() {
  mock.dataset.state = state.value;
  note.textContent = notices[state.value];
  $$('[data-write]').forEach((button) => { button.disabled = state.value === 'disconnected' || state.value === 'error'; });
}
function showSurface() {
  $$('[data-surface]').forEach((item) => { item.hidden = item.dataset.surface !== surface.value; });
}
surface.addEventListener('change', showSurface);
state.addEventListener('change', setState);
$('#narrow').addEventListener('click', () => {
  const active = mock.classList.toggle('narrow');
  $('#narrow').setAttribute('aria-pressed', String(active));
  $('#narrow').textContent = active ? 'Full pane' : 'Split narrow';
});

const collapsedFolders = new Set();
function filterFiles() {
  const query = $('#file-search').value.trim().toLowerCase();
  $$('[data-file]').forEach((button) => {
    const match = !query || (button.dataset.file + ' ' + files[button.dataset.file]).toLowerCase().includes(query);
    button.hidden = !match || collapsedFolders.has(button.dataset.folderFile);
  });
  $$('[data-folder]').forEach((folder) => {
    const child = $$('[data-folder-file="' + folder.dataset.folder + '"]').some((item) => !item.hidden);
    folder.hidden = Boolean(query) && !child;
  });
}
$('#file-search').addEventListener('input', filterFiles);
$('#tree-toggle').addEventListener('click', () => {
  const panel = $('.context');
  const narrow = mock.getBoundingClientRect().width <= 620;
  const open = narrow ? panel.classList.toggle('tree-open') : !panel.classList.toggle('tree-collapsed');
  $('#tree-toggle').setAttribute('aria-expanded', String(open));
});
$$('[data-folder]').forEach((button) => button.addEventListener('click', () => {
  const key = button.dataset.folder;
  const expanded = !collapsedFolders.has(key);
  if (expanded) collapsedFolders.add(key); else collapsedFolders.delete(key);
  button.setAttribute('aria-expanded', String(!expanded));
  button.textContent = (expanded ? '› ' : '⌄ ') + key + '/';
  filterFiles();
}));
$$('[data-file]').forEach((button) => button.addEventListener('click', () => {
  $$('[data-file]').forEach((item) => item.classList.remove('selected'));
  button.classList.add('selected');
  const path = button.dataset.file;
  $('#file-path').textContent = path;
  $('#file-content').textContent = files[path];
  $('#context-load-status').textContent = files[path].split('\n').length + ' lines · read-only';
  if (mock.getBoundingClientRect().width <= 620) { $('.context').classList.remove('tree-open'); $('#tree-toggle').setAttribute('aria-expanded','false'); }
}));
function reload(buttonId, statusId) {
  let timer = null;
  const button = $(buttonId);
  const status = $(statusId);
  button.addEventListener('click', () => {
    if (timer) {
      window.clearTimeout(timer);
      timer = null;
      button.textContent = 'Reload';
      status.textContent = 'Load canceled. Last displayed resource retained.';
      return;
    }
    button.textContent = 'Cancel';
    status.textContent = 'Loading…';
    timer = window.setTimeout(() => {
      timer = null;
      button.textContent = 'Reload';
      status.textContent = 'Read-only · refreshed';
    }, 850);
  });
}
reload('#context-load', '#context-load-status');
reload('#review-load', '#review-load-status');
$$('[data-preview]').forEach((button) => button.addEventListener('click', () => {
  const id = button.dataset.preview;
  const value = $('#' + id + '-draft').value.trim();
  const output = $('#' + id + '-preview');
  output.textContent = value || 'Write a comment before previewing it.';
  output.hidden = false;
}));

$('#review-scope').addEventListener('change', (event) => { $('#base-field').hidden = event.currentTarget.value !== 'branch'; });
$$('[data-diff]').forEach((button) => button.addEventListener('click', () => {
  $$('[data-diff]').forEach((item) => item.classList.remove('selected'));
  button.classList.add('selected');
  const next = diffs[button.dataset.diff];
  $('#diff-path').textContent = next[0];
  $('#diff-content').innerHTML = next[1];
  numberDiff();
}));
$('#review-draft').addEventListener('input', (event) => {
  const count = event.currentTarget.value.trim() ? 1 : 0;
  $('#draft-count').textContent = count + ' draft' + (count === 1 ? '' : 's') + ' · preview before delivery';
});
$('#prepare-paste').addEventListener('click', () => { $('#paste-result').textContent = 'Prepared for ' + $('#paste-target').value + '. Enter does not deliver.'; });
$('#review-refresh').addEventListener('click', () => lab('Review refresh is a local design demonstration.'));

function updateSetupPreview() {
  const valid = $('#repo').value && $('#trust').checked && $('#branch').value.trim() && $('#path').value.trim();
  $('#setup-preview').hidden = !valid;
  if (!valid) return;
  $('#effect-path').textContent = $('#path').value.trim();
  $('#effect-branch').textContent = $('#branch').value.trim();
  $('#effect-repo').textContent = $('#repo').selectedOptions[0].text;
}
['#repo', '#trust', '#branch', '#path'].forEach((selector) => { $(selector).addEventListener('input', updateSetupPreview); $(selector).addEventListener('change', updateSetupPreview); });
$('#setup-form').addEventListener('submit', (event) => {
  event.preventDefault();
  updateSetupPreview();
  const invalid = $('#setup-preview').hidden;
  $('#setup-error').hidden = !invalid;
  $('#setup-error').textContent = 'Choose a repository, branch, path, and trust local configuration before review.';
  if (!invalid) lab('Space setup preview updated locally.');
});
$('#setup-cancel').addEventListener('click', () => { $('#setup-preview').hidden = true; $('#setup-error').hidden = true; });

$('#feedback-refresh').addEventListener('click', () => lab('Browser feedback refresh is a local design demonstration.'));
$('#acknowledge').addEventListener('click', () => { $('#delivery').textContent = 'Sample annotations are marked acknowledged in this mock.'; });
$('#deliver').addEventListener('click', () => { $('#delivery').textContent = 'Delivery outcome unknown. Keep drafts and inspect the target before retrying.'; });

const captureWork = $('#capture-work');
const captureOpen = $('#capture-open');
const gestures = $('#gesture-tools');
const editor = $('#annotation-editor');
const annotationPanel = $('#annotation-panel');
let gesture = 'select';
let selectedAnnotation = 'Save changes';
function updateAnnotationCount() { $('#annotation-count').textContent = String($$('.annotation-item').length); }
function chooseGesture(next) {
  gesture = next;
  $$('[data-gesture]').forEach((item) => item.classList.toggle('selected', item.dataset.gesture === next));
}
function browse() {
  captureWork.classList.remove('annotating');
  gestures.hidden = true;
  $$('.hovered').forEach((item) => item.classList.remove('hovered'));
}
function openEditor(label, kind) {
  selectedAnnotation = label;
  editor.hidden = false;
  $('#annotation-kind').value = kind || 'Element';
  $$('.annotation-item').forEach((item) => item.classList.toggle('selected', item.dataset.annotation === label));
  $('#capture-result').textContent = 'Editing ' + label + '.';
}
function addAnnotation(label, kind) {
  const existing = $$('.annotation-item').find(item => item.dataset.annotation === label);
  if (existing) { openEditor(label, kind); return; }
  const item = document.createElement('button');
  item.className = 'annotation-item';
  item.dataset.annotation = label;
  item.textContent = ($$('.annotation-item').length + 1) + ' · ' + label;
  annotationPanel.insertBefore(item, editor);
  updateAnnotationCount();
  openEditor(label, kind);
}
captureOpen.addEventListener('click', () => {
  captureWork.classList.add('annotating');
  gestures.hidden = false;
  chooseGesture('select');
  $('#gesture-tools [data-gesture="select"]').focus();
});
$('#capture-close').addEventListener('click', () => { editor.hidden = true; browse(); captureOpen.focus(); });
$$('[data-gesture]').forEach((button) => button.addEventListener('click', () => chooseGesture(button.dataset.gesture)));
$$('[data-element]').forEach((item) => {
  item.addEventListener('pointerenter', () => { if (captureWork.classList.contains('annotating') && gesture === 'element') item.classList.add('hovered'); });
  item.addEventListener('pointerleave', () => item.classList.remove('hovered'));
  item.addEventListener('click', (event) => {
    if (!captureWork.classList.contains('annotating') || gesture !== 'element') return;
    event.preventDefault();
    addAnnotation(item.dataset.element, 'Element');
    $('#capture-draft').focus();
  });
});
$('#page-preview').addEventListener('click', (event) => {
  const existing = event.target.closest('[data-annotation]');
  if (existing && captureWork.classList.contains('annotating')) {
    openEditor(existing.dataset.annotation);
    return;
  }
  if (!captureWork.classList.contains('annotating') || event.target.closest('[data-element]')) return;
  if (gesture === 'region' || gesture === 'freehand') {
    const kind = gesture[0].toUpperCase() + gesture.slice(1);
    addAnnotation(kind + ' mark ' + ($$('.annotation-item').length + 1), kind);
    $('#capture-draft').focus();
  }
});
annotationPanel.addEventListener('click', (event) => {
  const item = event.target.closest('[data-annotation]');
  if (item) { if (!captureWork.classList.contains('annotating')) captureOpen.click(); openEditor(item.dataset.annotation); }
});
$('#delete-annotation').addEventListener('click', () => {
  const item = $$('.annotation-item').find((candidate) => candidate.dataset.annotation === selectedAnnotation);
  if (item) item.remove();
  $$('#page-preview [data-annotation]').filter(pin => pin.dataset.annotation === selectedAnnotation).forEach(pin => pin.remove());
  updateAnnotationCount();
  editor.hidden = true;
  $('#capture-result').textContent = 'Annotation removed from this mock.';
});
$('#save-capture').addEventListener('click', () => { editor.hidden = true; $('#capture-result').textContent = 'Annotation draft retained locally.'; });
document.addEventListener('keydown', (event) => {
  const editable = event.target instanceof HTMLElement && event.target.matches('input,textarea,select,[contenteditable=true]');
  if (surface.value === 'capture' && captureWork.classList.contains('annotating') && !editable && !event.isComposing && !event.ctrlKey && !event.metaKey && !event.altKey && ['v', 'e', 'r', 'f'].includes(event.key.toLowerCase())) {
    chooseGesture({ v: 'select', e: 'element', r: 'region', f: 'freehand' }[event.key.toLowerCase()]);
    event.preventDefault();
    return;
  }
  if (event.key !== 'Escape') return;
  if (!editor.hidden && (editor.contains(event.target) || !editable)) {
    event.preventDefault();
    editor.hidden = true;
    captureOpen.focus();
    return;
  }
  if (editable) return;
  if (captureWork.classList.contains('annotating')) {
    browse();
    captureOpen.focus();
    return;
  }
  const tree = $('.context.tree-open');
  if (tree) {
    tree.classList.remove('tree-open');
    $('#tree-toggle').setAttribute('aria-expanded', 'false');
    $('#tree-toggle').focus();
  } else surface.focus();
});
setState();
showSurface();
filterFiles();
updateSetupPreview();

$('#state-retry').addEventListener('click', () => { state.value = 'loading'; setState(); setTimeout(() => {state.value = 'ready'; setState(); lab('Illustrative refresh completed.');}, 650); });

function numberDiff() {
  const hunk = $('#diff-content .hunk').textContent.match(/-(\d+),\d+ \+(\d+),/);
  let oldLine = Number(hunk[1]);
  let newLine = Number(hunk[2]);
  $$('#diff-content>span:not(.hunk)').forEach(line => {
    line.dataset.old = line.classList.contains('added') ? '' : String(oldLine++);
    line.dataset.new = line.classList.contains('deleted') ? '' : String(newLine++);
  });
}
numberDiff();

const studyParams = new URLSearchParams(location.search);
if ([...surface.options].some(option => option.value === studyParams.get('surface'))) surface.value = studyParams.get('surface');
if ([...state.options].some(option => option.value === studyParams.get('state'))) state.value = studyParams.get('state');
showSurface();setState();
