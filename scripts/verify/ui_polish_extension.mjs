#!/usr/bin/env node
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { chromium } from '/home/linuxbrew/.linuxbrew/lib/node_modules/@playwright/cli/node_modules/playwright-core/index.mjs';

const repo = resolve(new URL('../..', import.meta.url).pathname);
const fixture = process.env.COCKPIT_EXTENSION_FIXTURE;
if (!fixture) throw new Error('Set COCKPIT_EXTENSION_FIXTURE to the disposable HTTP fixture URL.');
const evidence = process.env.COCKPIT_EXTENSION_EVIDENCE || await mkdtemp(join(tmpdir(), 'cpol-extension-evidence-'));
const extension = join(repo, 'browser-extension');
const extensionId = 'fblkilbfbmpndnfjaacljmcljhepakok';
const receipt = { fixture, extensionId, checks: [], screenshots: [], startedAt: new Date().toISOString() };
let context;
let profile;

function check(name, pass, detail = '') {
  receipt.checks.push({ name, status: pass ? 'pass' : 'fail', detail });
  if (!pass) throw new Error(`${name}: ${detail}`);
}

function findNode(node) {
  if (!node || typeof node !== 'object') return null;
  if (node.nodeName === 'COCKPIT-FEEDBACK-OVERLAY') return node;
  for (const child of [...(node.children || []), ...(node.shadowRoots || [])]) {
    const match = findNode(child);
    if (match) return match;
  }
  return null;
}

async function attachShadow(page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('DOM.enable');
  await cdp.send('Runtime.enable');
  const document = await cdp.send('DOM.getDocument', { depth: -1, pierce: true });
  const host = findNode(document.root);
  if (!host?.shadowRoots?.[0]) throw new Error('closed feedback overlay shadow root was not found through CDP');
  const resolved = await cdp.send('DOM.resolveNode', { nodeId: host.shadowRoots[0].nodeId });
  return { cdp, shadowObjectId: resolved.object.objectId };
}

async function shadowEval(cdp, objectId, functionDeclaration) {
  const result = await cdp.send('Runtime.callFunctionOn', {
    objectId,
    functionDeclaration,
    returnByValue: true,
    awaitPromise: true,
  });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.text || 'closed-shadow evaluation failed');
  return result.result.value;
}

async function controls(cdp, shadowObjectId) {
  return shadowEval(cdp, shadowObjectId, `function () {
    const rect = selector => { const node = this.querySelector(selector); if (!node) return null; const r = node.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height, hidden: node.hidden, opacity: getComputedStyle(node).opacity, text: node.textContent }; };
    return {
      select: rect('[data-tool="select"]'), freehand: rect('[data-tool="freehand"]'), region: rect('[data-tool="region"]'),
      notes: rect('.notes-toggle'), capture: rect('.capture-action'), editor: rect('.editor'),
      sidebar: rect('.notes-sidebar'), toolbar: rect('.controls'),
      notesItems: [...this.querySelectorAll('.notes-list button')].map(node => { const r = node.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height, text: node.textContent }; }),
      notesClear: rect('.notes-clear'),
      marks: [...this.querySelectorAll('.mark')].map(mark => ({ tag: mark.tagName, className: mark.getAttribute('class'), points: mark.getAttribute('points'), strokeWidth: getComputedStyle(mark).strokeWidth })),
      annotationCount: this.querySelectorAll('.mark:not(.preview)').length,
      textarea: this.querySelector('textarea')?.value || '',
      hint: this.querySelector('.hint')?.textContent || '',
      notesExpanded: this.querySelector('.notes-toggle')?.getAttribute('aria-expanded'),
      sidebarHidden: this.querySelector('.notes-sidebar')?.hidden,
    };
  }`);
}

const center = rect => ({ x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 });

async function click(page, rect) {
  const point = center(rect);
  await page.mouse.click(point.x, point.y);
}

async function screenshot(page, name) {
  const path = join(evidence, `extension-${name}.png`);
  await page.screenshot({ path, fullPage: false });
  receipt.screenshots.push(path);
}

try {
  await mkdir(evidence, { recursive: true });
  profile = await mkdtemp(join(tmpdir(), 'cpol-i11uqx_w-extension-profile-'));
  context = await chromium.launchPersistentContext(profile, {
    headless: true,
    channel: 'chromium',
    viewport: { width: 1000, height: 700 },
    args: [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`],
  });
  const page = await context.newPage();
  await page.goto(fixture, { waitUntil: 'networkidle' });
  let worker = context.serviceWorkers().find(item => item.url().includes(extensionId));
  if (!worker) worker = await context.waitForEvent('serviceworker', { timeout: 10_000 });
  check('extension-worker', worker.url() === `chrome-extension://${extensionId}/background.js`, worker.url());
  await worker.evaluate(async fixtureUrl => openToolbar((await chrome.tabs.query({})).find(tab => tab.url?.startsWith(fixtureUrl))), fixture);
  await page.waitForTimeout(200);

  const { cdp, shadowObjectId } = await attachShadow(page);
  let state = await controls(cdp, shadowObjectId);
  check('toolbar-opened', Boolean(state.toolbar && !state.toolbar.hidden), JSON.stringify(state.toolbar));
  check('toolbar-idle-opacity', state.toolbar.opacity === '0.45', state.toolbar.opacity);
  check('notes-default-closed-outside-toolbar', state.notesExpanded === 'false' && state.sidebarHidden && state.notes.y >= state.toolbar.y + state.toolbar.height, JSON.stringify({ notes: state.notes, toolbar: state.toolbar, expanded: state.notesExpanded, hidden: state.sidebarHidden }));
  await click(page, state.freehand);
  state = await controls(cdp, shadowObjectId);
  await page.mouse.move(110, 205);
  await page.mouse.down();
  for (const [x, y] of [[120, 205], [130, 205], [140, 205], [150, 205], [160, 205], [170, 205], [180, 215], [190, 225], [200, 235], [210, 245]]) await page.mouse.move(x, y);
  const live = await controls(cdp, shadowObjectId);
  check('freehand-live-points', Boolean(live.marks[0]?.points?.split(' ').length >= 10), JSON.stringify(live.marks));
  await page.mouse.up();
  state = await controls(cdp, shadowObjectId);
  const freehand = state.marks.find(mark => mark.tag === 'polyline');
  check('freehand-release-simplifies', Boolean(freehand?.points && freehand.points.split(' ').length >= 2 && freehand.points.split(' ').length < 11), JSON.stringify(freehand));
  check('freehand-3px', freehand?.strokeWidth === '3px', freehand?.strokeWidth || 'missing');
  await screenshot(page, 'freehand');

  await click(page, state.region);
  await page.mouse.move(300, 205);
  await page.mouse.down();
  await page.mouse.move(430, 280);
  await page.mouse.up();
  state = await controls(cdp, shadowObjectId);
  const region = state.marks.find(mark => mark.tag === 'rect');
  check('region-created', Boolean(region), JSON.stringify(state.marks));
  check('region-2px', region?.strokeWidth === '2px', region?.strokeWidth || 'missing');

  await click(page, state.notes);
  state = await controls(cdp, shadowObjectId);
  check('notes-sidebar-opens', state.notesExpanded === 'true' && !state.sidebarHidden, JSON.stringify({ expanded: state.notesExpanded, hidden: state.sidebarHidden }));
  await screenshot(page, 'notes-open');
  await click(page, state.notes);
  state = await controls(cdp, shadowObjectId);
  check('notes-sidebar-closes', state.notesExpanded === 'false' && state.sidebarHidden, JSON.stringify({ expanded: state.notesExpanded, hidden: state.sidebarHidden }));

  // Selecting the freehand mark opens its real comment editor; type through the browser keyboard.
  await click(page, state.select);
  await page.mouse.click(110, 205);
  await page.keyboard.press('Enter');
  await page.keyboard.type('offline draft note');
  state = await controls(cdp, shadowObjectId);
  check('inline-comment', state.textarea === 'offline draft note', state.textarea);
  await page.keyboard.press('Enter');
  await page.mouse.move(center(state.freehand).x, center(state.freehand).y);
  await page.waitForTimeout(160);
  state = await controls(cdp, shadowObjectId);
  check('toolbar-hover-opacity', state.toolbar.opacity === '1', state.toolbar.opacity);
  await page.mouse.move(700, 500);
  for (let index = 0; index < 12; index += 1) {
    await page.keyboard.press('Tab');
    await page.waitForTimeout(30);
    state = await controls(cdp, shadowObjectId);
    if (state.toolbar.opacity === '1') break;
  }
  check('toolbar-focus-opacity', state.toolbar.opacity === '1', state.toolbar.opacity);

  // Pointer cancellation should discard only the in-progress drawing.
  await click(page, state.freehand);
  await page.mouse.move(500, 200);
  await page.mouse.down();
  await page.mouse.move(530, 230);
  const beforeCancel = (await controls(cdp, shadowObjectId)).annotationCount;
  await shadowEval(cdp, shadowObjectId, `function () { this.querySelector('div[style*="touch-action"]')?.dispatchEvent(new PointerEvent('pointercancel', { pointerId: 1, bubbles: true, composed: true })); }`);
  await page.mouse.up();
  state = await controls(cdp, shadowObjectId);
  check('pointercancel-discards-drawing', state.annotationCount === beforeCancel - 1, `${beforeCancel} -> ${state.annotationCount}`);

  // Notes can revisit a mark after the page has moved, using its stored document-space geometry.
  await page.evaluate(() => scrollTo(0, 850));
  await page.waitForTimeout(100);
  state = await controls(cdp, shadowObjectId);
  await click(page, state.notes);
  state = await controls(cdp, shadowObjectId);
  check('notes-revisit-item', state.notesItems.length === 2, JSON.stringify(state.notesItems));
  await click(page, state.notesItems[0]);
  await page.waitForTimeout(100);
  check('notes-revisit-scrolls-to-mark', await page.evaluate(() => scrollY < 100), String(await page.evaluate(() => scrollY)));

  // Moving the viewport and changing visual scale retains the marks, then capture failure retains the draft.
  await page.evaluate(() => scrollTo(0, 0));
  await cdp.send('Emulation.setPageScaleFactor', { pageScaleFactor: 1.25 });
  await page.waitForTimeout(100);
  check('page-zoom-changes-visual-scale', await page.evaluate(() => visualViewport.scale > 1), String(await page.evaluate(() => visualViewport.scale)));
  await cdp.send('Emulation.setPageScaleFactor', { pageScaleFactor: 1 });
  await page.setViewportSize({ width: 900, height: 650 });
  await page.setViewportSize({ width: 1000, height: 700 });
  state = await controls(cdp, shadowObjectId);
  check('scroll-zoom-retains-marks', state.annotationCount === 2, `${state.annotationCount} marks`);
  await click(page, state.capture);
  await page.waitForTimeout(700);
  state = await controls(cdp, shadowObjectId);
  const stored = await worker.evaluate(() => chrome.storage.local.get('cockpit.feedback.drafts'));
  const drafts = stored['cockpit.feedback.drafts'] || [];
  check('offline-capture-retains-draft', drafts.some(draft => Array.isArray(draft.annotations) && draft.annotations.length === 2), JSON.stringify({ hint: state.hint, drafts }));
  await screenshot(page, 'offline-draft');
  if (state.sidebarHidden) await click(page, state.notes);
  state = await controls(cdp, shadowObjectId);
  await click(page, state.notesClear);
  state = await controls(cdp, shadowObjectId);
  check('notes-clear', state.annotationCount === 0 && state.notesItems.length === 0, JSON.stringify({ annotations: state.annotationCount, notes: state.notesItems }));
  await screenshot(page, 'notes-cleared');
  receipt.status = 'pass';
} catch (error) {
  receipt.status = 'fail';
  receipt.error = error instanceof Error ? error.message : String(error);
} finally {
  receipt.finishedAt = new Date().toISOString();
  await writeFile(join(evidence, 'extension-smoke.json'), `${JSON.stringify(receipt, null, 2)}\n`);
  if (context) await context.close();
  if (profile) await rm(profile, { recursive: true, force: true });
}

console.log(JSON.stringify(receipt, null, 2));
if (receipt.status !== 'pass') process.exitCode = 1;
