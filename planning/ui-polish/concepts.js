const concepts = {
  desk: {
    name: '01 Agent desk',
    benefit: 'Agents are the first thing you can reach. Review files use tabs instead of a second sidebar. Setup opens entirely beside the terminal.',
    tradeoff: 'Spaces move below the agent queue. File tabs work best for a small change set; overflow would need a file picker.'
  },
  map: {
    name: '02 Project map',
    benefit: 'An agent sits directly under its Space, so you do not have to connect two lists mentally. Review is one continuous, collapsible document.',
    tradeoff: 'Grouping agents by Space gives up the global attention order. This is an intentional alternative to compare, not a proposed change to Herdr authority.'
  },
  search: {
    name: '03 Search desk',
    benefit: 'Filter Spaces and agents in one place. In the file pane, source and Markdown sit side by side, so checking the rendered result does not require switching views.',
    tradeoff: 'Source and Markdown each get less width. The full-width Source and Markdown controls remain available.'
  },
  navigator: {
    name: '04 Pane navigator',
    benefit: 'One navigation rail contains agents and the current pane’s files. Removing the inner file tree gives the file or review content its full pane width. Setup uses a bottom sheet with the fields side by side.',
    tradeoff: 'The lower rail follows the active pane, rather than keeping every file list visible. Spaces are reached through the Space picker.'
  }
};
const initial = new URLSearchParams(location.search);
let concept = Object.hasOwn(concepts, initial.get('concept')) ? initial.get('concept') : 'desk';
let scene = ['review', 'files', 'workspace'].includes(initial.get('scene')) ? initial.get('scene') : 'review';
let setupOpen = scene === 'workspace';
let setupStep = 0;
let setupReview = false;
let pickedFile = 0;
let fileMode = concept === 'search' ? 'both' : 'markdown';
let filterQuery = '';
let overlay = null;
let showOriginal = false;
let projectExpanded = true;
const form = { task: '', repository: '', branch: 'main', checkout: 'new', companion: true, artifact: '' };
const agents = [
  { name: 'cockpit · secondary', tab: 'secondary', provider: 'codex', state: '◐', active: true },
  { name: 'gui-ideas · 1', tab: '1', provider: 'omp', state: '○' },
  { name: 'cockpit · main', tab: 'main', provider: 'omp', state: '○' },
  { name: 'cockpit · meta', tab: 'meta', provider: 'omp', state: '○' },
  { name: 'system · 1', tab: '1', provider: 'omp', state: '○' }
];
const spacesList = ['system', 'lilygo-t3', 'brave-forest-7518', 'brave-stone-13f0', '~', 'cockpit', 'feat/gui-ideas'];
const changedNames = ['ReviewPane.test.tsx', 'ReviewPane.tsx'];
const board = document.querySelector('#concept-artboard');
const escapeHTML = value => String(value).replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[char]));
const matches = value => value.toLowerCase().includes(filterQuery.trim().toLowerCase());
const disabled = 'disabled title="Static reference: live Herdr actions are not connected"';

function agentRow(agent, nested = false) {
  return `<div class="agent-row ${agent.active ? 'selected' : ''} ${nested ? 'nested' : ''}">
    <span class="state ${agent.active ? 'working' : ''}" role="img" aria-label="${agent.active ? 'Working' : 'Idle'}" title="${agent.active ? 'Working' : 'Idle'}">${agent.state}</span>
    <span class="agent-label">${nested ? agent.tab : agent.name}<small>${agent.provider}</small></span>
  </div>`;
}
function agentsSection() {
  return `<section class="agents-section"><header class="section-label">agents</header>${agents.filter(a => matches(a.name + ' ' + a.provider)).map(a => agentRow(a)).join('')}</section>`;
}
function spaceRows() {
  return spacesList.filter(matches).map(name => {
    const child = ['brave-forest-7518', 'brave-stone-13f0', 'feat/gui-ideas'].includes(name);
    const repository = ['lilygo-t3', 'cockpit'].includes(name);
    return `<div class="space-row ${child ? 'child' : ''} ${name === 'cockpit' ? 'selected' : ''}"><span class="state">${name === 'cockpit' ? '◐' : name === 'system' ? '○' : '·'}</span><span>${name}${repository ? '<small>main</small>' : ''}</span>${repository ? '<span class="chevron">⌄</span>' : ''}</div>`;
  }).join('');
}
function spacesSection() {
  return `<section class="spaces-section"><header class="section-label">spaces<button class="icon-button" data-action="setup" aria-label="Set up a workspace">+</button></header>${spaceRows()}</section>`;
}
function changedRows() {
  return changedNames.map((name, index) => `<button class="change-row ${pickedFile === index ? 'selected' : ''}" data-change="${index}"><span class="file-state">M</span><span>${name}</span><span class="delta">${index ? '+17 −4' : '+29'}</span></button>`).join('');
}
function fileRows() {
  return filenames.map((name, index) => `<${name === 'CONTEXT.md' ? 'button' : 'div'} class="file-entry ${name === 'CONTEXT.md' ? 'selected' : ''}" ${name === 'CONTEXT.md' ? 'data-action="choose-context"' : ''} title="${name}"><span class="file-symbol">${index < 12 ? '›' : '·'}</span><span>${name}</span></${name === 'CONTEXT.md' ? 'button' : 'div'}>`).join('');
}
function projectMap() {
  return `<header class="section-label">spaces<button class="icon-button" data-action="setup" aria-label="Set up a workspace">+</button></header>
    <div class="map-root">○ <span>system</span></div>${agentRow(agents[4], true)}
    <div class="map-root">· <span>lilygo-t3<small>main</small></span><span class="chevron">⌄</span></div>
    <div class="map-child">brave-forest-7518</div><div class="map-child">brave-stone-13f0</div>
    <div class="map-root">· <span>~</span></div>
    <button class="map-root current-project" data-action="toggle-project" aria-expanded="${projectExpanded}">◐ <span>cockpit<small>main</small></span><span class="chevron">${projectExpanded ? '⌄' : '›'}</span></button>
    ${projectExpanded ? `<div class="project-children">${[agents[2], agents[0], agents[3]].map(a => agentRow(a, true)).join('')}<div class="map-child branch-child">feat/gui-ideas</div>${agentRow(agents[1], true)}</div>` : ''}`;
}
function sidebar() {
  if (concept === 'map') return `<aside class="navigation project-map">${projectMap()}</aside>`;
  if (concept === 'navigator') return `<aside class="navigation pane-navigation">
    <header class="space-picker"><button data-action="spaces" aria-expanded="${overlay === 'spaces'}">cockpit <span>⌄</span></button><button class="icon-button" data-action="setup" aria-label="Set up a workspace">+</button></header>
    ${agentsSection()}
    <section class="pane-files"><header class="section-label">${scene === 'files' ? 'files' : 'reviewr'}</header>
    ${scene === 'files' ? fileRows() : '<div class="folder-path">src/app/review/</div>' + changedRows()}</section></aside>`;
  if (concept === 'search') return `<aside class="navigation search-navigation"><div class="navigation-search"><span aria-hidden="true">⌕</span><input id="navigation-filter" aria-label="Find a Space or agent" placeholder="Find a Space or agent" value="${escapeHTML(filterQuery)}"><kbd>/</kbd></div><div id="navigation-results">${spacesSection()}${agentsSection()}${filterQuery && !spacesList.some(matches) && !agents.some(a => matches(a.name + ' ' + a.provider)) ? '<p class="no-results">No matches</p>' : ''}</div></aside>`;
  return `<aside class="navigation agent-desk"><header class="section-label">agents</header>${agents.map(a => agentRow(a)).join('')}<div class="rail-divider"></div>${spacesSection()}</aside>`;
}
function tabStrip() {
  return `<nav class="tab-strip" aria-label="Space tabs">${['main', 'secondary', 'meta', '', ''].map((name, index) => `<div class="tab ${index === (scene === 'files' ? 4 : 1) ? 'selected' : ''}"><span>${index + 1}</span>${name}</div>`).join('')}<button ${disabled} class="icon-button">+</button><div class="tab-actions"><button ${disabled}>Pane</button><button ${disabled}>Commands</button></div></nav>`;
}
function tokenize(line) {
  const pattern = /"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\b(?:const|let|async|await|return|function|export|if|true|false|null)\b/g;
  let result = '', offset = 0;
  for (const token of line.matchAll(pattern)) {
    result += escapeHTML(line.slice(offset, token.index));
    result += `<span class="${/^["']/.test(token[0]) ? 'syntax-string' : 'syntax-keyword'}">${escapeHTML(token[0])}</span>`;
    offset = token.index + token[0].length;
  }
  return result + escapeHTML(line.slice(offset));
}
function diffLines(index) {
  return `<div class="hunk">@@ −180 +180 @@</div><div class="diff-lines">${codeSamples[index].map((line, i) => `<div class="diff-line ${i > 1 ? 'addition' : ''}"><span class="line-number">${i < 2 ? 180 + i : ''}</span><span class="line-number">${180 + i}</span><span class="sign">${i > 1 ? '+' : ''}</span><code>${tokenize(line)}</code></div>`).join('')}</div>`;
}
function diffDocument(index, collapsible = false) {
  const head = `<span class="file-state">M</span><span>src/app/review/<strong>${changedNames[index]}</strong></span><span class="delta">${index ? '+17 −4' : '+29'}</span>`;
  return collapsible
    ? `<details open class="review-document" id="change-${index}"><summary>${head}</summary>${diffLines(index)}</details>`
    : `<div class="review-document"><div class="review-document-title">${head}</div>${diffLines(index)}</div>`;
}
function reviewPane() {
  let navigation = '';
  if (concept === 'desk') navigation = `<nav class="review-file-tabs">${changedNames.map((name, index) => `<button class="${pickedFile === index ? 'selected' : ''}" data-change="${index}"><span class="file-state">M</span>${name}</button>`).join('')}</nav>`;
  if (concept === 'map') navigation = `<div class="review-jump">${changedNames.map((name, index) => `<button data-jump="${index}">${name}</button>`).join('')}</div>`;
  if (concept === 'search') navigation = `<div class="review-file-picker"><button data-action="changes" aria-expanded="${overlay === 'changes'}"><span class="file-state">M</span>${changedNames[pickedFile]} <span>⌄</span></button><span class="delta">${pickedFile ? '+17 −4' : '+29'}</span></div>`;
  return `<img class="terminal-reference" src="assets/terminal.png" alt="Unchanged terminal screenshot">
    <section class="review-pane pane" aria-label="Review pane"><div class="pane-title">reviewr</div>
      <div class="pane-toolbar"><select aria-label="Comparison" ${disabled}><option>All local changes</option></select><div class="arrow-group">${['←', '→', '↑', '↓'].map(symbol => `<button ${disabled}>${symbol}</button>`).join('')}</div><span class="grow"></span><button ${disabled}>Refresh</button></div>
      ${navigation}<div class="review-content">${concept === 'map' ? changedNames.map((_, index) => diffDocument(index, true)).join('') : diffDocument(pickedFile)}</div>
      <footer class="pane-footer"><span>Select a line</span><span>C comment</span><span>Shift+C file</span><span>0 comments</span><span>Alt+↑↓ hunk</span></footer>
    </section>`;
}
function markdownSource() {
  const template = document.createElement('template');
  template.innerHTML = documentHTML;
  const walk = node => {
    if (node.nodeType === Node.TEXT_NODE) return node.textContent;
    const content = [...node.childNodes].map(walk).join('');
    switch (node.nodeName) {
      case 'H1': return '# ' + content + '\n\n';
      case 'H2': return '## ' + content + '\n\n';
      case 'H3': return '### ' + content + '\n\n';
      case 'P': return content + '\n\n';
      case 'LI': return '- ' + content + '\n';
      case 'UL': return content + '\n';
      case 'STRONG': return '**' + content + '**';
      case 'CODE': return '`' + content + '`';
      default: return content;
    }
  };
  return walk(template.content).trim();
}
const sourceMarkdown = markdownSource();
function sourceViewer() {
  return `<div class="markdown-source">${sourceMarkdown.split('\n').map((line, index) => `<div><span>${index + 1}</span><code class="${line.startsWith('#') ? 'md-heading' : ''}">${escapeHTML(line)}</code></div>`).join('')}</div>`;
}
function filePane() {
  const split = concept === 'search' && fileMode === 'both';
  const tree = concept === 'map';
  return `<section class="file-pane pane" aria-label="File pane">
    <div class="file-toolbar">${concept !== 'navigator' && !tree ? '<button class="file-picker-button" data-action="files">Files <span>⌄</span></button>' : '<span>cockpit</span>'}<span class="path-separator">/</span><strong>CONTEXT.md</strong><span class="read-only">read-only</span><span class="grow"></span><div class="view-controls">${['source', 'markdown'].map(mode => `<button data-view="${mode}" aria-pressed="${fileMode === mode}">${mode === 'source' ? 'Source' : 'Markdown'}</button>`).join('')}${concept === 'search' ? `<button data-view="both" aria-pressed="${split}" title="Show source and Markdown side by side">Both</button>` : ''}</div><button ${disabled}>Refresh</button><button ${disabled}>Show terminal</button></div>
    <div class="discovery-warning"><button data-action="warning" aria-expanded="${overlay === 'warning'}">Repository discovery entry limit reached <span>⌄</span></button></div>
    <div class="file-content ${tree ? 'with-tree' : ''} ${split ? 'split-view' : ''}">${tree ? `<aside class="inline-file-tree">${fileRows()}</aside>` : ''}
    ${split ? `<section class="source-half"><header>Source</header>${sourceViewer()}</section><section class="rendered-half"><header>Markdown</header><article>${documentHTML}</article></section>` : fileMode === 'source' ? sourceViewer() : `<article>${documentHTML}</article>`}</div>
    <footer class="pane-footer"><span>Select a line</span><span>C comment</span><span>Shift+C file</span><span>0 comments</span></footer></section>`;
}
function field(label, key, placeholder = '', type = 'text') {
  return `<label class="setup-field"><span>${label}</span><input type="${type}" data-field="${key}" value="${escapeHTML(form[key])}" placeholder="${placeholder}"></label>`;
}
function repositoryChoice(search = true) {
  return `${search ? field('Repository', 'repository', 'Name, path, or repository ID') : ''}<div class="repository-result">${repositoryResult()}</div>`;
}
function repositoryResult() {
  return 'cockpit /home/nnex/dev/prj/cockpit'.includes(form.repository.trim().toLowerCase()) ? '<div class="repository-choice"><strong>cockpit</strong><code>/home/nnex/dev/prj/cockpit</code><small>main</small></div>' : '<p class="no-results">No matching repositories</p>';
}
function worktreeFields() {
  return `<label class="setup-field"><span>Worktree</span><select data-field="checkout"><option value="new" ${form.checkout === 'new' ? 'selected' : ''}>Create a task worktree</option><option value="existing" ${form.checkout === 'existing' ? 'selected' : ''}>Use existing checkout</option></select></label>${form.checkout === 'new' ? field('Base branch', 'branch', 'main') : ''}`;
}
function contextFields() {
  return `<label class="setup-checkbox"><input type="checkbox" data-field="companion" ${form.companion ? 'checked' : ''}>Create companion context</label>${field('Artifact URL <em>optional</em>', 'artifact', 'Issue or review URL')}`;
}
function rootDetails() {
  return `<details class="setup-details"><summary>Configured roots</summary><dl><dt>Repositories</dt><dd><code>/home/nnex/dev/prj/cockpit</code></dd><dt>Worktrees</dt><dd><code>~/.local/state/cockpit/worktrees</code></dd><dt>Companions</dt><dd><code>~/.local/state/cockpit/companions</code></dd></dl></details>`;
}
function setupWarning() {
  return '<details class="setup-details warning"><summary>Repository discovery entry limit reached</summary><p><code>catalog_entries_bounded</code><br>/home/nnex/dev/prj/cockpit<br>2 occurrences</p></details>';
}
function summaryFields() {
  return `<dl class="setup-summary"><dt>Repository</dt><dd>cockpit</dd><dt>Task</dt><dd>${escapeHTML(form.task) || 'Not specified'}</dd><dt>Worktree</dt><dd>${form.checkout === 'new' ? 'Create from ' + escapeHTML(form.branch) : 'Use existing checkout'}</dd><dt>Context</dt><dd>${form.companion ? 'Create companion directory' : 'No companion directory'}</dd>${form.artifact ? `<dt>Artifact</dt><dd>${escapeHTML(form.artifact)}</dd>` : ''}<dt>Session</dt><dd>default</dd></dl>`;
}
function setupFooter(review = false, stepper = false) {
  return `<footer class="setup-footer"><span>Session <code>default</code></span><button data-action="${review ? 'edit-setup' : stepper && setupStep ? 'setup-back' : 'close-setup'}">${review || (stepper && setupStep) ? 'Back' : 'Cancel'}</button><button class="primary" ${review ? 'disabled title="Design prototype only. No resources are created."' : 'data-action="' + (stepper ? 'setup-next' : 'review-setup') + '"'}>${review ? 'Preview only' : stepper ? 'Continue' : 'Review'}</button></footer>`;
}
function setupHeader() {
  return '<header class="setup-header"><h1>Set up a workspace</h1><button data-action="close-setup" aria-label="Close workspace setup">Close</button></header>';
}
function setupDialog() {
  if (!setupOpen) return '';
  let body;
  if (concept === 'desk') {
    const steps = ['Repository', 'Worktree', 'Context', 'Review'];
    const content = [() => repositoryChoice() + field('Task name <em>optional</em>', 'task', 'Short task description') + rootDetails() + setupWarning(), worktreeFields, contextFields, summaryFields][setupStep]();
    body = setupHeader() + `<nav class="setup-steps">${steps.map((name, i) => `<button data-step="${i}" aria-current="${setupStep === i ? 'step' : 'false'}"><span>${i + 1}</span>${name}</button>`).join('')}</nav><div class="setup-body">${content}</div>` + setupFooter(setupStep === 3, true);
  } else if (concept === 'map') {
    body = setupHeader() + `<div class="setup-columns"><aside class="repository-column">${repositoryChoice()}${rootDetails()}</aside><div class="setup-body">${setupReview ? summaryFields() : field('Task name <em>optional</em>', 'task', 'Short task description') + worktreeFields() + contextFields()}${setupWarning()}</div></div>` + setupFooter(setupReview);
  } else if (concept === 'search') {
    body = setupHeader() + `<div class="setup-body">${setupReview ? summaryFields() : `<div class="quick-fields">${repositoryChoice()}${field('Task name <em>optional</em>', 'task', 'Short task description')}</div><details class="setup-details quick-options"><summary>Worktree & context</summary><div class="quick-options-grid"><div>${worktreeFields()}</div><div>${contextFields()}</div></div></details>${setupWarning()}`}</div>` + setupFooter(setupReview);
  } else {
    body = setupHeader() + `<div class="setup-body">${setupReview ? summaryFields() : `<div class="sheet-columns"><section>${repositoryChoice()}${setupWarning()}</section><section>${field('Task name <em>optional</em>', 'task', 'Short task description')}${worktreeFields()}</section><section>${contextFields()}${rootDetails()}</section></div>`}</div>` + setupFooter(setupReview);
  }
  return `<div class="setup-layer"><section class="setup-dialog setup-${concept}" role="dialog" aria-modal="true" aria-label="Set up a workspace">${body}</section></div>`;
}
function overlayContent() {
  if (!overlay) return '';
  if (overlay === 'spaces') return `<section class="local-popover spaces-popover" role="dialog" aria-label="Spaces"><header class="section-label">spaces<button data-action="dismiss-overlay" aria-label="Close Space picker">×</button></header>${spaceRows()}</section>`;
  if (overlay === 'files') return `<section class="local-popover files-popover" role="dialog" aria-label="Files"><header class="section-label">files<button data-action="dismiss-overlay" aria-label="Close file picker">×</button></header>${fileRows()}</section>`;
  if (overlay === 'changes') return `<section class="local-popover changes-popover" role="dialog" aria-label="Changed files"><header class="section-label">Changed files<button data-action="dismiss-overlay" aria-label="Close changed file picker">×</button></header>${changedRows()}</section>`;
  return '<section class="local-popover warning-popover" role="dialog" aria-label="Repository diagnostic"><header class="section-label">Repository discovery<button data-action="dismiss-overlay" aria-label="Close diagnostic">×</button></header><p><code>catalog_entries_bounded</code><br>Repository discovery entry limit reached<br>/home/nnex/dev/prj/cockpit<br>2 occurrences</p></section>';
}
function fit() {
  const viewport = document.querySelector('.preview-viewport').getBoundingClientRect();
  board.style.transform = `translate(-50%, -50%) scale(${Math.min(viewport.width / 1568, viewport.height / 992)})`;
}
function render() {
  board.dataset.concept = concept;
  board.innerHTML = `<div class="concept-shell" ${setupOpen ? 'inert' : ''}>${sidebar()}${tabStrip()}${scene === 'files' ? filePane() : reviewPane()}${overlayContent()}</div>${setupDialog()}${showOriginal ? `<img class="original-reference" src="assets/${scene}-original.png" alt="Original supplied screenshot">` : ''}`;
  document.querySelector('#concept-choices').innerHTML = Object.entries(concepts).map(([key, value]) => `<button data-concept="${key}" aria-pressed="${concept === key}">${value.name}</button>`).join('');
  document.querySelectorAll('[data-scene]').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.scene === scene)));
  document.querySelector('#original-toggle').setAttribute('aria-pressed', String(showOriginal));
  document.querySelector('#original-toggle').textContent = showOriginal ? 'Back to concept' : 'Original';
  document.querySelector('#concept-notes').innerHTML = `<p>${concepts[concept].benefit}</p><p><strong>Tradeoff.</strong> ${concepts[concept].tradeoff}</p><small>Unselected visual concepts. All interactions use static reference data. No live sessions or resources are changed.</small>`;
  history.replaceState(null, '', `?concept=${concept}&scene=${scene}`);
  const nav = document.querySelector('#navigation-filter');
  if (nav) nav.addEventListener('input', () => {
    filterQuery = nav.value;
    document.querySelector('#navigation-results').innerHTML = spacesSection() + agentsSection() + (filterQuery && !spacesList.some(matches) && !agents.some(a => matches(a.name + ' ' + a.provider)) ? '<p class="no-results">No matches</p>' : '');
  });
  fit();
}
function focusSetup() {
  requestAnimationFrame(() => {
    const fields = [...document.querySelectorAll('.setup-dialog input, .setup-dialog select')];
    const target = fields.find(field => field.getClientRects().length) ?? document.querySelector('.setup-dialog button');
    target?.focus({ preventScroll: true });
  });
}
document.addEventListener('input', event => {
  const key = event.target.dataset.field;
  if (!key) return;
  form[key] = event.target.type === 'checkbox' ? event.target.checked : event.target.value;
  if (key === 'repository') document.querySelectorAll('.repository-result').forEach(element => element.innerHTML = repositoryResult());
});
document.addEventListener('change', event => {
  if (event.target.dataset.field === 'checkout') {
    const expanded = document.querySelector('.quick-options')?.open;
    form.checkout = event.target.value;
    render();
    const options = document.querySelector('.quick-options');
    if (options) options.open = Boolean(expanded);
    document.querySelector('[data-field="checkout"]')?.focus({ preventScroll: true });
  }
});
document.addEventListener('click', event => {
  const button = event.target.closest('button');
  if (!button || button.disabled) return;
  if (button.dataset.concept) { concept = button.dataset.concept; fileMode = concept === 'search' ? 'both' : 'markdown'; overlay = null; filterQuery = ''; showOriginal = false; render(); return; }
  if (button.dataset.scene) { scene = button.dataset.scene; setupOpen = scene === 'workspace'; overlay = null; showOriginal = false; render(); if (setupOpen) focusSetup(); return; }
  if (button.id === 'about-toggle') { const notes = document.querySelector('#concept-notes'); notes.hidden = !notes.hidden; button.setAttribute('aria-expanded', String(!notes.hidden)); fit(); return; }
  if (button.id === 'original-toggle') { showOriginal = !showOriginal; render(); return; }
  if (button.dataset.change !== undefined) { pickedFile = Number(button.dataset.change); overlay = null; render(); return; }
  if (button.dataset.view) { fileMode = button.dataset.view; render(); return; }
  if (button.dataset.jump !== undefined) { const target = document.querySelector(`#change-${button.dataset.jump}`); target.open = true; target.scrollIntoView({ block: 'start' }); return; }
  if (button.dataset.step !== undefined) { setupStep = Number(button.dataset.step); render(); focusSetup(); return; }
  switch (button.dataset.action) {
    case 'setup': setupOpen = true; showOriginal = false; overlay = null; render(); focusSetup(); break;
    case 'close-setup': setupOpen = false; render(); document.querySelector('[data-action="setup"]')?.focus(); break;
    case 'setup-next': setupStep = Math.min(3, setupStep + 1); render(); focusSetup(); break;
    case 'setup-back': setupStep = Math.max(0, setupStep - 1); render(); focusSetup(); break;
    case 'review-setup': setupReview = true; render(); focusSetup(); break;
    case 'edit-setup': setupReview = false; if (concept === 'desk') setupStep = 2; render(); focusSetup(); break;
    case 'toggle-project': projectExpanded = !projectExpanded; render(); break;
    case 'spaces': case 'files': case 'changes': case 'warning': overlay = overlay === button.dataset.action ? null : button.dataset.action; render(); break;
    case 'dismiss-overlay': case 'choose-context': overlay = null; render(); break;
  }
});
document.addEventListener('keydown', event => {
  if (event.key === 'Escape') {
    if (setupOpen) { setupOpen = false; render(); document.querySelector('[data-action="setup"]')?.focus(); }
    else if (overlay) { overlay = null; render(); }
  }
  if (event.key === '/' && concept === 'search' && !setupOpen && !['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName)) { event.preventDefault(); document.querySelector('#navigation-filter')?.focus(); }
  if (event.key === 'Tab' && setupOpen && !showOriginal) {
    const stops = [...document.querySelectorAll('.setup-dialog button:not(:disabled), .setup-dialog input, .setup-dialog select, .setup-dialog summary')].filter(element => element.getClientRects().length);
    const first = stops[0], last = stops.at(-1);
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }
});
window.addEventListener('resize', fit);
render();
