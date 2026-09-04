/* Planning-only prototype. All state is fixture data; no application or Herdr calls exist. */
const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => [...document.querySelectorAll(selector)];
const storageKey = 'cockpit-workflow-discussion-v1';
const escapeHtml = (text) => String(text).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
const diagram = '<svg class="diagram" viewBox="0 0 400 100" role="img" aria-label="Read, collect comments, paste to agent"><rect x="12" y="30" width="95" height="38" rx="4"/><text x="45" y="54">Read</text><path d="M107 49h38m-5-4 5 4-5 4"/><rect x="145" y="30" width="105" height="38" rx="4"/><text x="171" y="54">Collect</text><path d="M250 49h38m-5-4 5 4-5 4"/><rect x="288" y="30" width="100" height="38" rx="4"/><text x="322" y="54">Paste</text></svg>';
const files = {
  issue: { group: 'Sources / Gitea', name: '#142 · Context viewer', path: 'sources/gitea/issues/142.md', revision: 'snapshot:7b92', lines: ['---', 'provider: gitea', 'canonical_id: team/cockpit/issues/142', '---', '# Context viewer', '', 'Keep context beside the active agent.', 'References include original line numbers.', 'Collect comments across multiple files.', 'Paste only when the user requests it.', '', '```mermaid', 'flowchart LR', '  Read --> Collect --> Paste', '```'], html: '<span class="tag">Gitea · #142 · Open</span><h1>Context viewer</h1><p>Keep context beside the active agent. Read the task, inspect the source, and send references without leaving the tab.</p><details><summary>Metadata · checked 4 minutes ago</summary><p>provider: gitea<br>canonical_id: team/cockpit/issues/142<br>revision: snapshot:7b92</p></details><h2>Read, collect, then paste</h2><p>References include original line numbers. Collect comments across multiple files. Paste only when the user requests it.</p>' + diagram + '<span class="tiny muted">Illustrative Mermaid preview in this mock</span><h2>Acceptance</h2><ul><li>Read beside the terminal.</li><li>Collect full-file and line comments.</li><li>Paste without automatically submitting.</li></ul><button data-action="comment-rendered">Comment on these source lines</button>' },
  notes: { group: 'Notes', name: 'interaction.md', path: 'notes/interaction.md', revision: 'file:39ac', lines: ['# Interaction notes', '', 'Keep source references stable.', 'Store the selected excerpt with the comment.', 'If a file changes, flag the reference for review.', 'Do not silently replace the quoted lines.'], html: '<span class="tag">Personal note</span><h1>Interaction notes</h1><p>Keep source references stable. Store the selected excerpt with the comment.</p><h2>When a file changes</h2><p>Flag the reference for review. Keep the quoted original lines until I choose what to do with the comment.</p>' },
  code: { group: 'Repositories / herdr', name: 'src/review.ts', path: 'repos/herdr/src/review.ts', revision: 'snapshot:82b7', lines: ['export function formatReference(reference) {', '  const { path, startLine, endLine } = reference;', '  const location = `${path}:${startLine}-${endLine}`;', '  return [', '    location,', '    reference.excerpt,', '    reference.comment,', '  ].join("\\n");', '}'], html: '' },
  wiki: { group: 'Sources / Wiki', name: 'architecture.md', path: 'sources/wiki/architecture.md', revision: 'wiki:5c3a', lines: ['# Architecture', '', 'Herdr owns sessions, Spaces, tabs and terminal panes.', 'Cockpit owns the graphical context and review models.', 'The original extension continues independently.'], html: '<span class="tag">Wiki · last successful snapshot</span><h1>Architecture</h1><p>Herdr owns sessions, Spaces, tabs, and terminal panes.</p><p>Cockpit detects supported extension panes and renders its own graphical replacement. The original TUI continues independently.</p><p class="amber">Last refresh failed. This snapshot remains readable.</p>' },
};
const reviews = {
  review: { group: 'Unstaged', name: 'M · src/review.ts', path: 'src/review.ts', revision: 'index:a102/worktree:c830', rows: [{old:12,new:12,kind:' ',text:'export function collectComment() {'},{old:13,new:null,kind:'-',text:'  return sendImmediately();'},{old:null,new:13,kind:'+',text:'  return collectDraft();'},{old:14,new:14,kind:' ',text:'}'}] },
  transport: { group: 'Unstaged', name: 'M · src/transport.ts', path: 'src/transport.ts', revision: 'index:32cc/worktree:873a', rows: [{old:7,new:7,kind:' ',text:'export type PasteResult ='},{old:8,new:null,kind:'-',text:'  "sent";'},{old:null,new:8,kind:'+',text:'  "accepted" | "rejected" | "unknown";'},{old:9,new:9,kind:' ',text:''},{old:10,new:10,kind:' ',text:'export function pasteWithoutSubmit() {}'}] },
  guide: { group: 'Untracked', name: 'A · docs/notes.md', path: 'docs/notes.md', revision: 'worktree:d51b', rows: [{old:null,new:1,kind:'+',text:'# Review notes'},{old:null,new:2,kind:'+',text:''},{old:null,new:3,kind:'+',text:'Preserve comments until the paste result is known.'}] },
};
let saved = {};
try { saved = JSON.parse(localStorage.getItem(storageKey) || '{}'); } catch { /* storage is optional in this mock */ }
let state = {
  kind: 'context', file: 'issue', source: false, cursor: 0, anchor: null, end: null,
  drafts: Array.isArray(saved.drafts) ? saved.drafts : [], sent: saved.sent || [],
  target: 'omp', scenario: 'normal', layout: 'right', zoom: false, focus: 'gui',
  prefix: false, mode: 'normal', alwaysPreview: Boolean(saved.alwaysPreview),
  contextFile: 'issue', reviewFile: 'review', viewBeforeTerminal: 'context',
};
let editing = null;
let pendingAnchor = null;
let pickerItems = [];
let pickerIndex = 0;
let pickerKind = 'commands';
let toastTimer;
let dragSelecting = false;
let setupReviewed = false;
let activeSpace = 'Context workflow';
let activeTab = 'main';
let delivery = saved.delivery || null;
if (delivery?.status === 'pending') delivery.status = 'unknown';

function persist() {
  try { localStorage.setItem(storageKey, JSON.stringify({drafts:state.drafts, sent:state.sent, alwaysPreview:state.alwaysPreview, delivery})); } catch { $('#draft-state').textContent = 'Storage unavailable · session only'; }
}
function element(tag, text, className = '') {
  const node = document.createElement(tag); node.textContent = text; node.className = className; return node;
}
function toast(text) {
  clearTimeout(toastTimer); $('#toast').textContent = text; $('#toast').classList.remove('hidden');
  toastTimer = setTimeout(() => $('#toast').classList.add('hidden'), 3500);
}
function focusPane(pane, focusControl = true) {
  state.focus = pane;
  $('#gui-pane').classList.toggle('active-pane', pane === 'gui');
  $('#agent-pane').classList.toggle('active-pane', pane === 'agent');
  if (focusControl) (pane === 'agent' ? $('#agent-input') : $('#document')).focus();
}
function currentFile() { return state.kind === 'review' ? reviews[state.file] : files[state.file]; }
function currentRows() {
  if (state.kind === 'review') return currentFile().rows;
  return currentFile().lines.map((text, index) => ({old:null,new:index + 1,kind:' ',text}));
}
function selectedIndices() {
  if (state.anchor === null) return [];
  const low = Math.min(state.anchor, state.end ?? state.anchor);
  const high = Math.max(state.anchor, state.end ?? state.anchor);
  return Array.from({length:high - low + 1}, (_, index) => low + index);
}
function selectRange(index, extend = false) {
  state.cursor = index;
  if (!extend || state.anchor === null) state.anchor = index;
  state.end = index;
  updateSelection();
}
function updateSelection() {
  const chosen = selectedIndices();
  $$('.source-row').forEach(row => row.classList.toggle('selected', chosen.includes(Number(row.dataset.row))));
  $('#selection-bar').classList.toggle('hidden', !chosen.length);
  $('#selection-label').textContent = chosen.length + ' source row' + (chosen.length === 1 ? '' : 's') + ' selected';
}
function renderTree() {
  const root = $('#file-tree'); root.replaceChildren();
  const collection = state.kind === 'review' ? reviews : files;
  let group = '';
  Object.entries(collection).forEach(([key, file]) => {
    if (group !== file.group) { group = file.group; root.append(element('div', '⌄ ' + group, 'file-group')); }
    const button = element('button', file.name, 'file-button' + (state.file === key ? ' selected' : ''));
    button.dataset.file = key; button.title = file.path;
    button.onclick = () => openFile(key);
    button.oncontextmenu = (event) => { event.preventDefault(); openFile(key); openMenu(event.clientX, event.clientY, [['Comment on whole file', 'file-comment'], ['Show source', 'show-source'], ['Copy path', 'copy-path']]); };
    button.onkeydown = (event) => {
      const buttons = $$('.file-button'); const i = buttons.indexOf(button);
      if (['ArrowDown','ArrowUp','Home','End'].includes(event.key)) {
        event.preventDefault(); const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : Math.max(0, Math.min(buttons.length - 1, i + (event.key === 'ArrowDown' ? 1 : -1))); buttons[next].focus();
      } else if (event.key === 'ArrowRight') { event.preventDefault(); openFile(key); }
    };
    root.append(button);
  });
}
function openFile(key) {
  state.file = key; state.anchor = state.end = null; state.cursor = 0;
  if (state.kind === 'context') { state.contextFile = key; if (!files[key].html) state.source = true; }
  else state.reviewFile = key;
  renderTree(); renderDocument(); focusPane('gui');
}
function renderDocument() {
  const root = $('#document'); root.replaceChildren();
  $('#gui-title').textContent = state.kind === 'review' ? 'Reviewr' : state.kind === 'terminal' ? 'Original terminal view' : 'Context';
  $('#review-toolbar').classList.toggle('hidden', state.kind !== 'review');
  $('#root-button').textContent = state.kind === 'review' ? 'Worktree ▾' : 'Companion ▾';
  if (state.kind === 'terminal') {
    root.className = 'document'; $('#breadcrumb').textContent = 'Original extension · independent TUI';
    root.append(element('pre', 'Original extension TUI\n\nFiles / Reviewr\n\nThis is the same Herdr pane and process.\nIts selection and comments are independent.\n\nCockpit drafts remain saved.\nNo GUI actions are forwarded to this TUI.'));
    const back = element('button', 'Return to graphical view', 'primary'); back.onclick = () => switchKind(state.viewBeforeTerminal); root.append(back); return;
  }
  const file = currentFile(); $('#breadcrumb').textContent = file.path;
  const isSource = state.source || state.kind === 'review' || !file.html;
  $('#view-toggle').textContent = isSource ? (state.kind === 'review' ? 'Unified' : 'Rendered') : 'Source';
  root.className = 'document' + (isSource ? ' source' : '');
  if (!isSource) root.innerHTML = file.html;
  else {
    root.append(element('div', state.kind === 'review' ? 'Old / new lines · Shift-select a range on one side · C to comment' : '↑ ↓ navigate · Shift extends selection · C comments · double-click a gutter for a quick comment', 'source-hint'));
    currentRows().forEach((row, index) => {
      const line = element('div', '', 'source-row' + (state.kind === 'review' ? ' review-row' : '') + (row.kind === '+' ? ' add' : row.kind === '-' ? ' del' : ''));
      line.dataset.row = index;
      const gutter = (value, side) => {
        const button = element('button', value === null ? '' : String(value)); button.dataset.line = index;
        button.setAttribute('aria-label', `${side} line ${value ?? 'absent'}`); button.tabIndex = -1;
        button.onpointerdown = (event) => { event.preventDefault(); dragSelecting = true; selectRange(index, event.shiftKey); focusPane('gui'); };
        button.onpointerenter = () => { if (dragSelecting) selectRange(index, true); };
        button.ondblclick = () => { selectRange(index); openComment(false); };
        return button;
      };
      if (state.kind === 'review') { line.append(gutter(row.old, 'Old'), gutter(row.new, 'New'), element('span', row.kind)); }
      else line.append(gutter(row.new, 'Source'));
      line.append(element('span', row.text, 'line-code')); root.append(line);
    });
  }
  updateSelection();
  renderFileComments();
}
function switchKind(kind) {
  if (kind === 'terminal') { state.viewBeforeTerminal = state.kind; state.kind = kind; }
  else { state.kind = kind; state.file = kind === 'review' ? state.reviewFile : state.contextFile; state.source = kind === 'review'; }
  state.anchor = state.end = null; state.cursor = 0;
  renderTree(); renderDocument(); focusPane('gui');
  toast(kind === 'review' ? 'Mock: detected Reviewr pane; Cockpit owns this review UI.' : kind === 'context' ? 'Mock: file-viewer pane rendered as Context.' : 'Original terminal view. GUI comments remain independent.');
}
function draftAnchor(wholeFile) {
  const file = currentFile(); if (!file) return null;
  if (wholeFile) return {fileKey:state.file, kind:state.kind, path:file.path, revision:file.revision, side:'whole file', start:null, end:null, lines:[]};
  if (!state.source && state.kind === 'context') { state.source = true; state.anchor = 6; state.end = 9; renderDocument(); }
  if (state.anchor === null) selectRange(state.cursor);
  const rows = selectedIndices().map(index => currentRows()[index]);
  if (rows.some(row => row.kind === '-') && rows.some(row => row.kind === '+')) { toast('Select one side of the diff for this comment. Old and new lines keep separate anchors.'); return null; }
  const side = rows.some(row => row.kind === '-') ? 'old' : 'new';
  const numbered = rows.map(row => ({number:side === 'old' ? row.old : row.new,text:row.text})).filter(row => row.number !== null);
  if (!numbered.length) return null;
  return {fileKey:state.file,kind:state.kind,path:file.path,revision:file.revision,side:state.kind === 'review' ? side : 'source',start:numbered[0].number,end:numbered.at(-1).number,lines:numbered};
}
function openComment(wholeFile, existing = null) {
  if (state.kind === 'terminal') return toast('Return to the graphical view to collect Cockpit comments.');
  pendingAnchor = existing || draftAnchor(wholeFile); if (!pendingAnchor) return;
  editing = existing?.id || null;
  $('#comment-heading').textContent = editing ? 'Edit comment' : 'Add comment';
  $('#comment-anchor').textContent = pendingAnchor.path + (pendingAnchor.start === null ? ' · whole file' : `:${pendingAnchor.start}-${pendingAnchor.end} · ${pendingAnchor.side}`);
  $('#comment-text').value = existing?.comment || '';
  $('#comment-dialog').showModal(); $('#comment-text').focus();
}
function saveComment() {
  const comment = $('#comment-text').value.trim(); if (!comment) return $('#comment-text').focus();
  const draft = {...pendingAnchor, id:editing || crypto.randomUUID(), comment, stale:Boolean(pendingAnchor.stale), space:activeSpace, tab:activeTab};
  if (editing) state.drafts = state.drafts.map(item => item.id === editing ? draft : item);
  else state.drafts.push(draft);
  persist(); renderDrafts(); $('#comment-dialog').close(); focusPane('gui');
  $('#delivery-status').textContent = 'Comment collected. Keep reading; paste when ready.';
  toast('Comment collected · Ctrl+Shift+M overview · Ctrl+Shift+Enter paste to '+(state.target==='reviewer'?'Reviewer':'OMP'));
}
function commentCard(draft) {
  const item = element('article', '', 'draft');
  const body = element('div', draft.comment);
  body.append(element('small', draft.path + (draft.start === null ? ' · whole file' : `:${draft.start}-${draft.end} · ${draft.side}`) + (draft.stale ? ' · Source changed; captured excerpt retained' : ' · Unsent')));
  const edit = element('button', 'Edit'); edit.onclick = () => {
    if ($('#comments-dialog').open) $('#comments-dialog').close();
    if (state.kind !== draft.kind) switchKind(draft.kind);
    openFile(draft.fileKey); openComment(false, draft);
  };
  const remove = element('button', '×'); remove.setAttribute('aria-label', 'Remove comment');
  remove.onclick = () => { state.drafts = state.drafts.filter(item => item.id !== draft.id); persist(); renderDrafts(); };
  item.append(body, edit, remove); return item;
}
function renderFileComments() {
  $$('#document .file-comments').forEach(node => node.remove());
  if (state.kind === 'terminal') return;
  const local = state.drafts.filter(draft => draft.kind === state.kind && draft.fileKey === state.file && draft.space === activeSpace && draft.tab === activeTab);
  if (!local.length) return;
  const root = $('#document'); const source = root.classList.contains('source');
  const below = element('section', '', 'file-comments');
  below.append(element('h2', 'Unsent comments on this file'));
  let belowCount = 0;
  local.forEach(draft => {
    const index = source && draft.end !== null && !draft.stale ? currentRows().findIndex(row => (draft.side === 'old' ? row.old : row.new) === draft.end) : -1;
    const row = index >= 0 ? root.querySelector(`.source-row[data-row="${index}"]`) : null;
    if (row) {
      let group = row.nextElementSibling;
      if (!group?.classList.contains('inline-comments')) {
        group = element('section', '', 'file-comments inline-comments'); row.after(group);
      }
      group.append(commentCard(draft));
    } else { below.append(commentCard(draft)); belowCount++; }
  });
  if (belowCount) root.append(below);
}
function renderDrafts() {
  $('#draft-count').textContent = state.drafts.length;
  $('#comment-noun').textContent = state.drafts.length === 1 ? 'comment' : 'comments';
  $('#comments-toggle').title = `${state.drafts.length} unsent comments · target ${state.target === 'reviewer' ? 'Reviewer' : 'OMP'} · Ctrl+Shift+M opens overview`;
  const list = $('#draft-list'); list.replaceChildren();
  state.drafts.forEach(draft => list.append(commentCard(draft)));
  if (!state.drafts.length) list.append(element('p', 'No unsent comments. Select lines or comment on a whole file.', 'muted tiny'));
  $('#paste-button').disabled = !state.drafts.length || state.scenario === 'no-agent' || state.scenario === 'disconnected' || activeTab !== 'main' || activeSpace !== 'Context workflow' || Boolean(delivery?.status === 'pending');
  renderFileComments();
}
function commentsOverview() { renderDrafts(); if (!$('#comments-dialog').open) $('#comments-dialog').showModal(); }
function absolutePath(draft) { return (draft.kind === 'review' ? '/home/dev/worktrees/cockpit/context-workflow/' : '/home/dev/.local/share/cockpit/companions/cx-142/') + draft.path; }
function outgoingPayload() {
  return [...state.drafts].sort((a,b) => a.path.localeCompare(b.path) || (a.start ?? 0) - (b.start ?? 0)).map((draft, index) => {
    const location = absolutePath(draft) + (draft.start === null ? '' : `:${draft.start}-${draft.end}`);
    return `${index + 1}. ${location}\nRevision: ${draft.revision}${draft.side === 'old' ? ' · old/removed lines' : ''}${draft.stale ? ' · captured before file changed' : ''}\nComment: ${draft.comment}` + (draft.lines.length ? '\nSelected source lines:\n' + draft.lines.map(line => `${line.number} | ${line.text}`).join('\n') : '');
  }).join('\n\n');
}
function preview() {
  $('#payload').textContent = outgoingPayload() || 'No comments collected.';
  $('#payload-size').textContent = new TextEncoder().encode(outgoingPayload()).length + ' UTF-8 bytes · sample absolute paths';
  $('#preview-dialog').showModal();
}
function paste(confirmed = false) {
  if (!state.drafts.length) return toast('Collect a comment first.');
  if (activeTab !== 'main' || activeSpace !== 'Context workflow' || state.scenario === 'no-agent') return toast('No eligible agent in this tab. Your comments stay saved.');
  if (state.scenario === 'disconnected') return toast('Herdr is disconnected. Nothing was pasted.');
  if (delivery?.status === 'pending' || delivery?.status === 'unknown') return showUnknown();
  if (state.drafts.some(draft => draft.space !== activeSpace || draft.tab !== activeTab)) return toast('These drafts belong to another tab. Return there before pasting.');
  if (state.drafts.some(draft => draft.stale) && !confirmed) {
    details('Source changed', '<p>These comments still contain the exact lines you selected before the file changed.</p><div class="dialog-actions"><button data-action="keep-captured">Paste captured revision</button><button data-action="review-changed">Review source first</button></div>'); return;
  }
  if (state.alwaysPreview && !confirmed) return preview();
  const text = outgoingPayload();
  if (new TextEncoder().encode(text).length + 12 > 65536) return toast('Batch exceeds the mock 64 KiB ceiling. Remove comments or use a smaller batch.');
  if ($('#preview-dialog').open) $('#preview-dialog').close();
  if ($('#comments-dialog').open) $('#comments-dialog').close();
  delivery = {status:'pending', text, target:state.target, ids:state.drafts.map(item => item.id)};
  persist();
  $('#delivery-status').textContent = 'Mock: focusing target, checking same-tab identity, then pasting once…'; renderDrafts();
  const activeDelivery = delivery;
  setTimeout(() => {
    if (delivery !== activeDelivery) return; // Changing the lab scenario resets its simulated operation.
    if (state.scenario === 'unknown') { delivery.status = 'unknown'; persist(); showUnknown(); renderDrafts(); return; }
    if (state.scenario === 'rejected') { delivery.status = 'rejected'; persist(); $('#delivery-status').textContent = 'Rejected: mock target changed before delivery. Drafts retained.'; renderDrafts(); return; }
    acceptPaste();
  }, 220);
}
function acceptPaste(writeInput = true) {
  if (!delivery) return;
  if (writeInput) {
    $('#agent-input').value = delivery.text;
    $('#agent-input-state').textContent = 'Pasted · awaiting your Enter';
  }
  state.sent.push({...delivery,status:'accepted'});
  state.drafts = state.drafts.filter(item => !delivery.ids.includes(item.id)); delivery.status = 'accepted';
  persist(); renderDrafts(); focusPane('agent');
  toast(writeInput ? 'Pasted to '+(delivery.target==='reviewer'?'Reviewer':'OMP')+' · no Enter sent' : 'Marked pasted · no text sent again');
  $('#delivery-status').textContent = writeInput ? 'Paste accepted in mock. No Enter sent. Your agent input is ready.' : 'Marked pasted by you. Archived without sending text again.';
}
function showUnknown() {
  details('Paste result unknown', '<p>The connection dropped after the mock dispatched text. It might already be in the agent input.</p><p>Your batch is retained. Nothing retries automatically.</p><div class="dialog-actions"><button data-action="inspect-agent">Inspect agent input</button><button data-action="mark-pasted">Mark pasted</button><button data-action="repeat-paste">Paste again explicitly</button></div>');
  $('#delivery-status').textContent = 'Paste outcome unknown · inspect the target before repeating.';
}
function details(title, html) { $('#details-title').textContent = title; $('#details-body').innerHTML = html; if (!$('#details-dialog').open) $('#details-dialog').showModal(); }
function layout(position) {
  state.layout = position; state.zoom = false;
  toast('Mock: requesting Herdr pane move…');
  setTimeout(() => { $('#panes').className = 'panes layout-' + position; toast('Mock Herdr layout confirmed. Same pane, new position.'); }, 120);
}
function zoom() { state.zoom = !state.zoom; $('#panes').classList.toggle('zoomed', state.zoom); toast(state.zoom ? 'Mock Herdr pane zoomed · Ctrl+B then Z restores' : 'Mock Herdr layout restored'); }
function openMenu(x, y, entries) {
  const root = $('#context-menu'); root.replaceChildren();
  entries.forEach(([label,action]) => { const b = element('button', label); b.setAttribute('role','menuitem'); b.onclick = () => { closeMenu(); run(action); }; root.append(b); });
  root.style.left = Math.max(8, Math.min(x, innerWidth - 245)) + 'px'; root.style.top = Math.max(8, Math.min(y, innerHeight - entries.length * 36 - 18)) + 'px'; root.classList.remove('hidden'); root.querySelector('button')?.focus();
}
function closeMenu() { $('#context-menu').classList.add('hidden'); }
const paneActions = [['Move left','move-left'],['Move right','move-right'],['Move below','move-below'],['Zoom / restore · Ctrl+B Z','zoom'],['Show original terminal view','terminal-view'],['Render as Context','context'],['Render as Reviewr','review'],['Close pane…','close-pane']];
const commandEntries = [
  ['Open Context pane','context','File viewer replacement'],['Open Reviewr pane','review','Local graphical review'],['Set up task Space','setup','Repository + optional issue/review URL'],['Open a file','quick-open','Ctrl+P in GUI'],['Focus agent','agent','Return to same-tab target'],['Focus context/review','gui','Return to document'],['Collected comments','toggle-comments','Ctrl+Shift+M; all unsent references'],['Preview comments','preview','Inspect exact payload'],['Paste comments','paste','Ctrl+Shift+Enter in GUI'],['Sources','sources','Add / refresh / retry'],['Move pane left','move-left','Herdr layout'],['Move pane right','move-right','Herdr layout'],['Move pane below','move-below','Herdr layout'],['Zoom pane','zoom','Ctrl+B then Z'],['Original terminal view','terminal-view','Independent extension state'],['Remove task worktree','remove-worktree','Review exact resources'],['Keys and gestures','help','Shortcut discussion'],['All planned blocks','planned','Optional story coverage'],
];
function openPicker(kind) {
  pickerKind = kind; pickerIndex = 0;
  $('#command-query').value = ''; $('#command-query').placeholder = kind === 'files' ? 'Find a file by name or path…' : 'Type a command…';
  drawPicker(); $('#command-dialog').showModal(); $('#command-query').focus();
}
function drawPicker() {
  const query = $('#command-query').value.toLowerCase();
  const candidates = pickerKind === 'files' ? Object.entries(state.kind === 'review' ? reviews : files).map(([key,file]) => ({label:file.name,detail:file.path,execute:() => openFile(key)})) : commandEntries.map(([label,action,detail]) => ({label,detail,execute:() => run(action)}));
  pickerItems = candidates.filter(item => (item.label + ' ' + item.detail).toLowerCase().includes(query));
  pickerIndex = Math.min(pickerIndex, Math.max(0,pickerItems.length - 1));
  const root = $('#command-results'); root.replaceChildren();
  pickerItems.forEach((item,index) => { const b = element('button','','picker-result' + (index === pickerIndex ? ' selected' : '')); b.setAttribute('role','option'); b.setAttribute('aria-selected',String(index === pickerIndex)); b.append(element('span',item.label),element('small',item.detail)); b.onclick = () => { $('#command-dialog').close(); item.execute(); }; root.append(b); });
  if (!pickerItems.length) root.append(element('p','No matches.','muted'));
}
function setupReview() {
  const name = $('#setup-name').value.trim(); if (!name) return $('#setup-name').focus();
  if (!setupReviewed) {
    const repo = $('#setup-repo').value;
    $('#setup-summary').textContent = `${$('#setup-mode').value === 'create' ? 'Create' : 'Open'} ${repo} worktree: /home/dev/worktrees/${repo}/${name.toLowerCase().replaceAll(' ','-')}\nCompanion: /home/dev/.local/share/cockpit/companions/new-task\n${$('#setup-url').value ? 'Download selected issue/review context. ' : 'Local context only. '}${$('#setup-local').checked ? 'Include an independent Herdr repository snapshot. ' : ''}${$('#setup-context').checked ? 'Open a real file-viewer pane as Context.' : ''}\nExisting terminal environment is unchanged. No agent is launched.`;
    $('#setup-summary').style.whiteSpace = 'pre-line'; $('#setup-summary').classList.remove('hidden'); $('#setup-submit').textContent = 'Start setup'; setupReviewed = true;
  } else {
    $('#setup-dialog').close(); toast('Mock setup: worktree and companion created; opening Context pane…');
    setTimeout(() => { switchKind('context'); $('#delivery-status').textContent = 'Mock setup complete. Start an agent manually in a context-aware terminal.'; },180);
  }
}
function showHelp() {
  const rows = [['Ctrl+B, then ?','Existing Herdr-style command help'],['Ctrl+B, then Z','Zoom/restore the focused pane'],['Ctrl+P','Quick file open, GUI only'],['Ctrl+Shift+P','Commands, GUI only'],['↑ / ↓, Shift+↑ / ↓','Move source cursor / extend selection'],['C / Shift+C','Comment on selection / whole file, source focus only'],['Ctrl+Enter','Collect comment; review/start setup within its dialog'],['Ctrl+Shift+M','Open collected comments overview, GUI only'],['Ctrl+Shift+Enter','Paste collected comments from GUI, no submission'],['Alt+Enter','Toggle rendered/source, GUI only'],['F6 / Shift+F6','Cycle GUI regions and agent input in this mock'],['Escape','Close transient UI; preserve source/drafts'],['Mouse','Single-click file; Shift-click/drag gutter; right-click file/header; double-click header to zoom']];
  details('Keys, gestures, and interaction choices', '<p>These are discussion defaults. Terminal-focused keys stay with the terminal, except the existing Herdr prefix. Browser-reserved shortcuts need native/browser verification before implementation.</p><table class="shortcut-table">'+rows.map(([key,action])=>`<tr><td>${key}</td><td>${action}</td></tr>`).join('')+'</table><label class="check"><input id="always-preview" type="checkbox" '+(state.alwaysPreview?'checked':'')+'>Always preview before paste</label><p class="tiny muted">Default: the visible target and explicit Paste action are enough for a valid batch. Preview remains one click away. Source changes or uncertain delivery require a decision.</p>');
  $('#always-preview').onchange = event => { state.alwaysPreview = event.target.checked; persist(); };
}
function sources() {
  details('Context sources', '<div class="detail-row"><button data-action="refresh">Refresh</button><b class="green">✓ Gitea issue #142</b><small>Issue and comments · checked 4 minutes ago</small></div><div class="detail-row"><button data-action="source-retry">Retry</button><b class="amber">! Architecture wiki</b><small>Refresh failed · last successful snapshot retained</small></div><div class="detail-row"><button data-action="refresh">Refresh snapshot</button><b>Local Herdr repository</b><small>82b7a1d · independent reflink copy · source untouched</small></div><div class="dialog-actions"><button data-action="add-source">Add source</button><button data-action="conflict">Try local-edit conflict</button></div>');
}
function planned() {
  const blocks = [['CLEAN-01–05','Maintainability first','Clear code ownership; deterministic complexity/coverage/CRAP and mutation feedback for agents'],['FND-01–03','Configuration and shared contracts','Local settings, capabilities, operation recovery'],['LIFE-01–04','Project setup and teardown','Required local repo, optional artifact, worktree + companion'],['PANE-01–02','Detect and replace extension panes','No extension communication or Herdr server changes'],['CTX-01–02','Companion and local snapshots','Reflink/copy, owned paths, user file preservation'],['VIEW / REF','Read, select, comment, paste','Markdown/Mermaid, files, search, same-tab agent'],['SRC-01–03','Gitea issue source loop','Explicit downloads, cache, freshness, bounded references'],['REV-01–02','Full graphical Reviewr replacement','Own local Git/diff model; TUI stays independent'],['SRC-04–06','Review/wiki/telemetry sources','Separately selectable richer context'],['PANE-03','Possible TUI backport','Share useful Cockpit core logic later'],['OPT-01–13','Elective product blocks','Provider breadth, settings, history/inbox, opt-in automation, credentials, remote access, packaging, accessibility, search refinements']];
  details('Everything planned, without crowding everyday work', '<p>The workbench mock exercises the main workflow and full graphical review. Optional blocks stay independently selectable in the plan.</p>'+blocks.map(([id,title,body])=>`<div class="detail-row"><b>${id} · ${title}</b><small>${body}</small></div>`).join('')+'<p><a href="../README.md">Open full plan</a> · <a href="../09-maintainability.md">Code-tweak cleanup</a></p>');
}
function run(action) {
  const actions = {
    'quick-open':()=>openPicker('files'), palette:()=>openPicker('commands'), context:()=>switchKind('context'), review:()=>switchKind('review'),
    agent:()=>focusPane('agent'), gui:()=>focusPane('gui'), 'other-agent':()=>toast('That agent is in another Space. It is not a paste target for this tab.'),
    'toggle-source':()=>{ if(state.kind==='review') return toast('Unified diff is active. Exact old/new source numbers are shown.'); state.source=!state.source; renderDocument(); focusPane('gui'); },
    'show-source':()=>{state.source=true;renderDocument();focusPane('gui');},
    'selection-comment':()=>openComment(false), 'file-comment':()=>openComment(true), 'save-comment':saveComment,
    'comment-rendered':()=>{state.source=true;state.anchor=6;state.end=9;renderDocument();openComment(false);},
    'clear-selection':()=>{state.anchor=state.end=null;updateSelection();},
    'toggle-comments':commentsOverview, preview, paste:()=>paste(), 'paste-confirmed':()=>paste(true),
    'keep-captured':()=>{$('#details-dialog').close();paste(true);}, 'review-changed':()=>{$('#details-dialog').close();state.source=true;renderDocument();focusPane('gui');},
    'inspect-agent':()=>{$('#details-dialog').close();focusPane('agent');}, 'mark-pasted':()=>{$('#details-dialog').close();acceptPaste(false);},
    'repeat-paste':()=>{$('#details-dialog').close();delivery=null;state.scenario='normal';$('#scenario').value='normal';paste(true);},
    'move-left':()=>layout('left'),'move-right':()=>layout('right'),'move-below':()=>layout('below'),zoom,
    'terminal-view':()=>switchKind('terminal'), 'gui-menu':()=>{const r=$('.gui-pane .pane-header').getBoundingClientRect();openMenu(r.right-240,r.bottom,paneActions);},
    'agent-menu':()=>{const r=$('.agent-pane .pane-header').getBoundingClientRect();openMenu(r.left+10,r.bottom,[['Focus agent input','agent'],['Open Context beside agent','context'],['Open Reviewr beside agent','review']]);},
    'open-pane':()=>openPicker('commands'), 'copy-path':()=>toast('Mock path: '+absolutePath({kind:state.kind,path:currentFile().path})),
    setup:()=>{setupReviewed=false;$('#setup-summary').classList.add('hidden');$('#setup-submit').textContent='Review setup';$('#setup-dialog').showModal();$('#setup-name').focus();},
    'setup-review':setupReview, sources, refresh:()=>toast('Mock refresh complete. Source revisions checked; comments kept.'),
    'source-retry':()=>{details('Source refreshed','<p class="green">The wiki snapshot refreshed successfully. Existing comments keep their captured revision.</p>');},
    'add-source':()=>details('Add context source','<p>Choose a typed issue/review/wiki URL from a configured provider, or an already-discovered local repository.</p><label>Source<input placeholder="Gitea issue URL or local repository name"></label><div class="dialog-actions"><button data-action="source-retry">Add selected source · mock</button></div>'),
    conflict:()=>details('Local edits detected','<p>You changed the local architecture snapshot. Refresh kept your file and saved a new candidate.</p><div class="dialog-actions"><button data-action="refresh">Keep local copy</button><button data-action="refresh">Inspect fetched candidate</button></div>'),
    root:()=>details('File roots','<p>The Context pane shows the companion. A detected file-viewer pane can also show its authorized repository root.</p><button data-action="context">Companion context</button><button data-action="review">Primary repository / review</button>'),
    'remove-worktree':()=>details('Remove task worktree?','<p>Delete the owned checkout and companion only after fresh checks.</p><pre>/home/dev/worktrees/cockpit/context-workflow\n/home/dev/.local/share/cockpit/companions/cx-142</pre><p class="amber">Mock dirty check: modified and untracked files exist. Removal is blocked. The primary and reference repositories are excluded.</p><button data-action="close-space">Close Space only</button>'),
    'close-space':()=>toast('Mock: Space processes stopped; checkout, companion, and drafts remain. No real effect.'),
    'close-pane':()=>details('Close this Herdr pane?','<p>The extension process would stop. Cockpit comments remain recoverable; companion files remain.</p><button data-action="close-pane-confirmed">Close pane · mock</button>'),
    'close-pane-confirmed':()=>{$('#details-dialog').close();toast('Mock pane-close recorded. The study keeps its sample pane visible for further discussion.');},
    'main-tab':()=>{activeTab='main';activeSpace='Context workflow';$('#location-notice').classList.add('hidden');renderDrafts();},
    'other-tab':()=>{activeTab='scratch';$('#location-notice').textContent='Mock scratch tab: the shown sample is retained for discussion. Its drafts cannot be pasted here. Return to main.';$('#location-notice').classList.remove('hidden');renderDrafts();},
    'new-tab':()=>toast('Mock: request a new ordinary Herdr tab. No new workbench mode.'),
    session:()=>details('Herdr session','<p>development · current</p><p>Switching sessions detaches views and revalidates pane identities. Drafts stay recoverable and are never silently sent into another session.</p>'),
    help:showHelp, planned,
    reset:()=>{try{localStorage.removeItem(storageKey);}catch{}location.reload();},
  };
  actions[action]?.();
}

/* Event routing keeps the terminal input and GUI shortcuts separate. */
document.addEventListener('click', event => {
  const close = event.target.closest('[data-close]'); if (close) { $('#'+close.dataset.close).close(); focusPane('gui'); return; }
  const action = event.target.closest('[data-action]'); if (action) run(action.dataset.action);
  if (!event.target.closest('#context-menu') && !event.target.closest('[data-action="gui-menu"],[data-action="agent-menu"]')) closeMenu();
});
$('#gui-pane').addEventListener('pointerdown',()=>focusPane('gui',false));
$('#agent-pane').addEventListener('pointerdown',()=>focusPane('agent',false));
document.addEventListener('pointerup',()=>{dragSelecting=false;});
$$('.pane-header').forEach(header => {
  header.oncontextmenu = event => {event.preventDefault();openMenu(event.clientX,event.clientY,paneActions);};
  header.ondblclick = event => {if(!event.target.closest('button'))zoom();};
  header.ondragstart = event => {event.dataTransfer.setData('text/plain','pane');$('#drop-zones').classList.remove('hidden');};
  header.ondragend = ()=>$('#drop-zones').classList.add('hidden');
});
$$('[data-drop]').forEach(target => {target.ondragover=event=>event.preventDefault();target.ondrop=event=>{event.preventDefault();$('#drop-zones').classList.add('hidden');layout(target.dataset.drop);};target.onclick=()=>layout(target.dataset.drop);});
$$('[data-space]').forEach(button=>button.onclick=()=>{activeSpace=button.dataset.space;$$('.space').forEach(b=>b.classList.toggle('selected',b===button));$('#location-notice').classList.toggle('hidden',activeSpace==='Context workflow');$('#location-notice').textContent='Mock Space changed. The sample context is retained for discussion; paste is disabled until you return to Context workflow.';renderDrafts();});
$('#target').onchange=()=>{state.target=$('#target').value;$('#agent-title').textContent=state.target==='omp'?'OMP':'Reviewer';$('#paste-button').childNodes[0].textContent='Paste to '+(state.target==='omp'?'OMP':'Reviewer')+' ';renderDrafts();};
$('#command-query').oninput=()=>{pickerIndex=0;drawPicker();};
$('#command-query').onkeydown=event=>{if(['ArrowDown','ArrowUp'].includes(event.key)){event.preventDefault();pickerIndex=Math.max(0,Math.min(pickerItems.length-1,pickerIndex+(event.key==='ArrowDown'?1:-1)));drawPicker();}else if(event.key==='Enter'){event.preventDefault();const selected=pickerItems[pickerIndex];$('#command-dialog').close();selected?.execute();}};
$('#review-scope').onchange=()=>toast('Mock comparison scope changed to '+$('#review-scope').selectedOptions[0].textContent+'. Diff rows remain the illustrative fixture.');
$('#scenario').onchange=()=>{
  state.scenario=$('#scenario').value;delivery=null;
  $('#connection').textContent=state.scenario==='disconnected'?'Disconnected':'Connected';
  $('#connection').className='status '+(state.scenario==='disconnected'?'red':'green');
  $('#target').innerHTML=state.scenario==='no-agent'?'<option value="">No agent in this tab</option>':'<option value="omp">OMP · main · Blocked</option>'+(state.scenario==='two-agents'?'<option value="reviewer">Reviewer · main · Working</option>':'');
  state.target='omp';
  $('#paste-button').textContent='Paste to OMP ⇧ Ctrl ↵';
  persist();
  if(state.scenario==='changed'){state.drafts=state.drafts.map(draft=>({...draft,stale:true}));persist();toast(state.drafts.length?'Draft sources marked changed. Original excerpts kept.':'Collect a comment, then choose this scenario again to mark its source changed.');}
  renderDrafts();
};
$('#setup-dialog').addEventListener('input',()=>{setupReviewed=false;$('#setup-submit').textContent='Review setup';$('#setup-summary').classList.add('hidden');});
$('#agent-input').onkeydown=event=>{if(event.key==='Enter'&&!event.shiftKey&&!event.ctrlKey&&!event.metaKey){event.preventDefault();if(!$('#agent-input').value)return;$('#terminal-output').append(element('p','Mock user submitted '+$('#agent-input').value.length+' characters.','green'));$('#agent-input').value='';$('#agent-input-state').textContent='Submitted by your Enter · mock only';}};
document.addEventListener('keydown',event=>{
  const editable=event.target.matches('input,textarea,select,[contenteditable=true]');
  const terminal=event.target.closest('#agent-pane');
  if(event.ctrlKey&&!event.shiftKey&&event.key.toLowerCase()==='b'){event.preventDefault();state.prefix=true;$('#prefix-indicator').classList.remove('hidden');return;}
  if(state.prefix){event.preventDefault();state.prefix=false;$('#prefix-indicator').classList.add('hidden');if(event.key==='?')showHelp();else if(event.key.toLowerCase()==='z')zoom();else if(event.key==='Escape')return;else toast('Existing Herdr prefix action: '+event.key+' · not simulated in this study');return;}
  if(event.key==='F6'){event.preventDefault();const regions=[$('.space.selected'),$('.tab.active'),$('#document'),$('#comments-toggle'),$('#agent-input')];const i=regions.findIndex(node=>node===document.activeElement);const next=(i+(event.shiftKey?-1:1)+regions.length)%regions.length;regions[next].focus();focusPane(regions[next]===$('#agent-input')?'agent':'gui',false);return;}
  if(event.key==='Escape'){if(!$('#context-menu').classList.contains('hidden')){closeMenu();event.preventDefault();focusPane('gui');}return;}
  if($('#comment-dialog').open&&event.ctrlKey&&event.key==='Enter'){event.preventDefault();saveComment();return;}
  if($('#setup-dialog').open&&event.ctrlKey&&event.key==='Enter'){event.preventDefault();setupReview();return;}
  if ($('#comments-dialog').open && event.ctrlKey && event.shiftKey && event.key === 'Enter') { event.preventDefault(); paste(); return; }
  if($$('dialog[open]').length||terminal)return;
  if(event.ctrlKey&&event.shiftKey&&event.key.toLowerCase()==='m'){event.preventDefault();commentsOverview();return;}
  if(event.ctrlKey&&event.shiftKey&&event.key.toLowerCase()==='p'){event.preventDefault();openPicker('commands');return;}
  if(event.ctrlKey&&!event.shiftKey&&event.key.toLowerCase()==='p'){event.preventDefault();openPicker('files');return;}
  if(event.ctrlKey&&event.shiftKey&&event.key==='Enter'){event.preventDefault();paste();return;}
  if(event.altKey&&event.key==='Enter'){event.preventDefault();run('toggle-source');return;}
  if(editable)return;
  if(event.target===$('#document')&&(state.source||state.kind==='review')){
    if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)){event.preventDefault();const length=currentRows().length;const next=event.key==='Home'?0:event.key==='End'?length-1:Math.max(0,Math.min(length-1,state.cursor+(event.key==='ArrowDown'?1:-1)));selectRange(next,event.shiftKey);$('.source-row[data-row="'+next+'"]')?.scrollIntoView({block:'nearest'});}
    else if(event.key.toLowerCase()==='c'&&!event.ctrlKey&&!event.metaKey){event.preventDefault();openComment(event.shiftKey);}
  }
});
$$('dialog').forEach(dialog=>dialog.addEventListener('close',()=>{if(!$$('dialog[open]').length&&state.focus==='gui')$('#document').focus();}));
renderTree();renderDocument();renderDrafts();focusPane('gui');
