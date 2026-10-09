import { chromium, type BrowserContext, type Page } from 'playwright';
import { mkdir, chmod, copyFile, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { randomBytes, randomUUID } from 'node:crypto';

const ROOT = resolve(import.meta.dir);
const UI_DIR = join(ROOT, 'ui');
const EXTENSION_DIR = join(ROOT, 'extension');
const FIXTURE_PATH = join(ROOT, 'fixture.html');
const EXTENSION_ASSETS = ['manifest.json', 'background.js', 'content.js', 'popup.css', 'popup.js', 'popup.html', 'editor.js', 'editor.html', 'editor.css'];
const MAX_BODY = 12 * 1024 * 1024;
const MAX_TEXT = 8000;
const MAX_ANNOTATIONS = 500;

type Control = 'human' | 'agent';
type Role = 'ui' | 'extension' | 'agent';
type Annotation = Record<string, unknown> & { id: string; createdAt: string; session: string; spaceId: string };

type Config = { session: string; spaceId: string; label: string; stateDir: string; port: number; executable?: string };

function argValue(args: string[], name: string, required = true): string | undefined {
  const i = args.indexOf(name);
  if (i < 0) { if (required) throw new Error(`missing ${name}`); return undefined; }
  const value = args[i + 1];
  if (!value || value.startsWith('--')) throw new Error(`${name} requires a value`);
  return value;
}
function parseConfig(): Config {
  const args = Bun.argv.slice(2);
  const session = argValue(args, '--session');
  const spaceId = argValue(args, '--space');
  const label = argValue(args, '--label');
  const stateDir = argValue(args, '--state-dir');
  const portRaw = argValue(args, '--port', false) ?? '0';
  const port = Number(portRaw);
  if (!session || !spaceId || !label || !stateDir || !Number.isInteger(port) || port < 0 || port > 65535) throw new Error('invalid server arguments');
  return { session, spaceId, label, stateDir: resolve(stateDir), port, executable: argValue(args, '--executable', false) };
}
function token(): string { return randomBytes(32).toString('base64url'); }
function json(value: unknown, status = 200): Response { return Response.json(value, { status, headers: { 'cache-control': 'no-store' } }); }
function error(message: string, status = 400): Response { return json({ error: message }, status); }
function bearer(request: Request): string | undefined {
  const value = request.headers.get('authorization');
  return value?.match(/^Bearer ([A-Za-z0-9_-]{20,})$/)?.[1];
}
class ApiError extends Error {
  constructor(message: string, readonly status: number) { super(message); }
}
async function readJson(request: Request): Promise<Record<string, unknown>> {
  const length = Number(request.headers.get('content-length') ?? 0);
  if (length > MAX_BODY) throw new Error('request body too large');
  const text = await request.text();
  if (text.length > MAX_BODY) throw new Error('request body too large');
  const value = JSON.parse(text);
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('JSON object required');
  return value as Record<string, unknown>;
}
function isHttpUrl(value: unknown): value is string { return typeof value === 'string' && /^https?:\/\//i.test(value) && value.length <= 4096; }
function boundedString(value: unknown, name: string, max: number): string {
  if (typeof value !== 'string' || value.length > max) throw new Error(`${name} must be a string of at most ${max} characters`);
  return value;
}
function validateAnnotationDetails(body: Record<string, unknown>) {
  if (!body.viewport || typeof body.viewport !== 'object' || Array.isArray(body.viewport)) throw new Error('viewport is required');
  const viewport = body.viewport as Record<string, unknown>;
  for (const name of ['width', 'height', 'scrollX', 'scrollY', 'devicePixelRatio']) {
    if (typeof viewport[name] !== 'number' || !Number.isFinite(viewport[name])) throw new Error(`viewport.${name} must be finite`);
  }
  const element = body.element;
  if (element !== undefined) {
    if (!element || typeof element !== 'object' || Array.isArray(element)) throw new Error('element must be an object');
    const value = element as Record<string, unknown>;
    boundedString(value.tag, 'element.tag', 100);
    boundedString(value.text, 'element.text', 1000);
    boundedString(value.selector, 'element.selector', 1000);
    if (!value.rect || typeof value.rect !== 'object' || Array.isArray(value.rect)) throw new Error('element.rect is required');
    for (const name of ['x', 'y', 'width', 'height']) {
      const coordinate = (value.rect as Record<string, unknown>)[name];
      if (typeof coordinate !== 'number' || !Number.isFinite(coordinate)) throw new Error(`element.rect.${name} must be finite`);
    }
  }
  if (body.image !== undefined && (typeof body.image !== 'string' || !/^data:image\/(?:png|jpeg);base64,[A-Za-z0-9+/=]+$/.test(body.image) || body.image.length > 10_000_000)) throw new Error('image must be a bounded PNG or JPEG data URL');
  if (body.strokes !== undefined) {
    if (!Array.isArray(body.strokes) || body.strokes.length > 500) throw new Error('strokes are bounded');
    for (const stroke of body.strokes) {
      if (!stroke || typeof stroke !== 'object' || !Array.isArray((stroke as Record<string, unknown>).points) || ((stroke as Record<string, unknown>).points as unknown[]).length > 2000) throw new Error('stroke points are bounded');
    }
  }
}
function sameOrigin(origin: string | null, expected: string): boolean { return origin === expected; }

async function main() {
  const config = parseConfig();
  await mkdir(config.stateDir, { recursive: true, mode: 0o700 });
  await chmod(config.stateDir, 0o700);
  const stateFile = join(config.stateDir, 'annotations.json');
  const connectionPath = join(config.stateDir, 'connection.json');
  const profileDir = join(config.stateDir, 'browser-profile');
  const extensionRuntimeDir = join(config.stateDir, `extension-runtime-${randomUUID()}`);
  let annotations: Annotation[] = [];
  try {
    const prior = JSON.parse(await readFile(stateFile, 'utf8'));
    if (!Array.isArray(prior) || prior.length > MAX_ANNOTATIONS) throw new Error('annotation state is invalid');
    for (const item of prior) {
      if (!item || typeof item !== 'object' || item.session !== config.session || item.spaceId !== config.spaceId) throw new Error('annotation state belongs to another Space');
    }
    annotations = prior as Annotation[];
  } catch (caught) {
    if ((caught as NodeJS.ErrnoException).code !== 'ENOENT') throw caught;
  }
  const instanceId = randomUUID();
  const uiToken = token();
  const extensionToken = token();
  const agentToken = token();
  let control: Control = 'human';
  let controlGeneration = 0;
  let context: BrowserContext | undefined;
  let mainServer: ReturnType<typeof Bun.serve> | undefined;
  let fixtureServer: ReturnType<typeof Bun.serve> | undefined;
  let closed = false;
  let fixtureUrl = '';
  let endpoint = '';

  async function persistAnnotations(next: Annotation[]) {
    const temporary = `${stateFile}.${randomUUID()}.tmp`;
    await writeFile(temporary, JSON.stringify(next, null, 2), { mode: 0o600 });
    await rename(temporary, stateFile);
    await chmod(stateFile, 0o600);
  }
  function inspectableUrl(value: string): boolean {
    if (!isHttpUrl(value)) return false;
    try {
      const candidate = new URL(value);
      const controller = endpoint ? new URL(endpoint) : undefined;
      const isController = Boolean(controller && candidate.protocol === controller.protocol && candidate.port === controller.port && (candidate.hostname === '127.0.0.1' || candidate.hostname === 'localhost') && (controller.hostname === '127.0.0.1' || controller.hostname === 'localhost'));
      return !isController;
    } catch { return false; }
  }
  async function pageTarget(page: Page): Promise<{ id: string; url: string; title: string }> {
    const session = await context!.newCDPSession(page);
    try {
      const info = await session.send('Target.getTargetInfo');
      return { id: String(info.targetInfo.targetId), url: page.url(), title: await page.title().catch(() => '') };
    } finally { await session.detach().catch(() => {}); }
  }
  let annotationTail: Promise<void> = Promise.resolve();
  async function appendAnnotation(build: (current: Annotation[]) => Annotation): Promise<Annotation> {
    let created: Annotation | undefined;
    const operation = annotationTail.then(async () => {
      if (annotations.length >= MAX_ANNOTATIONS) throw new Error('annotation limit reached');
      created = build(annotations);
      const next = [...annotations, created];
      await persistAnnotations(next);
      annotations = next;
    });
    annotationTail = operation.catch(() => {});
    await operation;
    if (!created) throw new Error('annotation was not created');
    return created;
  }
  let actionTail: Promise<void> = Promise.resolve();
  async function enqueueAction(action: () => Promise<void>) {
    const operation = actionTail.then(action);
    actionTail = operation.catch(() => {});
    return operation;
  }
  async function tabs(): Promise<{ id: string; url: string; title: string }[]> {
    const out: { id: string; url: string; title: string }[] = [];
    for (const page of context?.pages() ?? []) {
      try {
        if (inspectableUrl(page.url())) out.push(await pageTarget(page));
      } catch { /* closed target */ }
    }
    return out;
  }
  async function findPage(targetId: string): Promise<Page | undefined> {
    for (const page of context?.pages() ?? []) {
      try {
        if (inspectableUrl(page.url()) && (await pageTarget(page)).id === targetId) return page;
      } catch { /* closed target */ }
    }
    return undefined;
  }
  function state() {
    return { instanceId, session: config.session, spaceId: config.spaceId, spaceLabel: config.label, control, tabs: [] as { id: string; url: string; title: string }[], annotations, connectionPath, fixtureUrl };
  }
  async function stateResponse() { const value = state(); value.tabs = await tabs(); return value; }

  async function shutdown() {
    if (closed) return; closed = true;
    await context?.close().catch(() => {});
    mainServer?.stop(); fixtureServer?.stop();
    try {
      const descriptor = JSON.parse(await readFile(connectionPath, 'utf8'));
      if (descriptor.token === agentToken) await rm(connectionPath, { force: true });
    } catch { /* absent or no longer owned */ }
    await rm(extensionRuntimeDir, { recursive: true, force: true }).catch(() => {});
  }
  try {
  const fixtureFile = await readFile(FIXTURE_PATH);
  fixtureServer = Bun.serve({
    hostname: '127.0.0.1', port: 0,
    async fetch(request) {
      const hostHeader = request.headers.get('host');
      const expectedPort = String(fixtureServer?.port ?? '');
      if (hostHeader !== `127.0.0.1:${expectedPort}` && hostHeader !== `localhost:${expectedPort}`) return error('invalid host', 400);
      const url = new URL(request.url);
      if (request.method !== 'GET' || url.pathname !== '/fixture') return error('not found', 404);
      return new Response(fixtureFile, { headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' } });
    },
  });
  fixtureUrl = `http://127.0.0.1:${fixtureServer.port}/fixture`;

  await mkdir(extensionRuntimeDir, { recursive: true, mode: 0o700 });
  await chmod(extensionRuntimeDir, 0o700);
  for (const file of EXTENSION_ASSETS) {
    try { await copyFile(join(EXTENSION_DIR, file), join(extensionRuntimeDir, file)); } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
    }
  }
  await writeFile(join(extensionRuntimeDir, 'config.js'), `globalThis.COCKPIT_SPIKE=${JSON.stringify({ endpoint: '', token: extensionToken, session: config.session, spaceId: config.spaceId, spaceLabel: config.label })};\n`, { mode: 0o600 });

  mainServer = Bun.serve({
    hostname: '127.0.0.1', port: config.port, maxRequestBodySize: MAX_BODY,
    async fetch(request) {
      const url = new URL(request.url);
      const hostHeader = request.headers.get('host');
      const expectedPort = String(mainServer?.port ?? '');
      if (hostHeader !== `127.0.0.1:${expectedPort}` && hostHeader !== `localhost:${expectedPort}`) return error('invalid host', 400);
      const auth = bearer(request);
      const role: Role | undefined = auth === uiToken ? 'ui' : auth === extensionToken ? 'extension' : auth === agentToken ? 'agent' : undefined;
      const origin = request.headers.get('origin');
      const originExpected = `${url.protocol}//${request.headers.get('host')}`;
      if (role === 'ui' && origin && !sameOrigin(origin, originExpected)) return error('invalid origin', 403);
      if (role === 'extension' && origin && !/^chrome-extension:\/\/[a-p]{32}$/.test(origin)) return error('invalid origin', 403);
      if (role === 'agent' && origin) return error('origin not allowed', 403);
      if (url.pathname.startsWith('/api/')) {
        if (!role) return error('authorization required', 401);
        try {
          if (request.method === 'GET' && url.pathname === '/api/state') return json(await stateResponse());
          if (request.method === 'GET' && url.pathname === '/api/annotations') return json({ annotations });
          if (request.method !== 'POST') return error('method not allowed', 405);
          const body = await readJson(request);
          if (url.pathname === '/api/open') {
            if (role === 'agent' || role === 'extension') return error('open is not allowed for this role', 403);
            const requested = body.url;
            if (requested !== undefined && (!isHttpUrl(requested) || !inspectableUrl(requested))) return error('url must be an external http or https page', 400);
            let page = requested ? await context!.newPage() : context?.pages().find((candidate) => inspectableUrl(candidate.url()));
            if (!page) page = await context!.newPage();
            if (requested) await page.goto(requested, { waitUntil: 'domcontentloaded', timeout: 15_000 });
            else if (!inspectableUrl(page.url())) await page.goto(fixtureUrl, { waitUntil: 'domcontentloaded', timeout: 15_000 });
            await page.bringToFront();
            return json({ tab: await pageTarget(page) });
          }
          if (url.pathname === '/api/control') {
            if (role === 'agent') return error('agent cannot grant control', 403);
            const next = body.control;
            if (next !== 'human' && next !== 'agent') return error('control must be human or agent', 400);
            control = next;
            controlGeneration += 1;
            if (next === 'human') await actionTail;
            return json({ control });
          }
          if (url.pathname === '/api/inspect') {
            const tabId = boundedString(body.tabId, 'tabId', 200);
            const page = await findPage(tabId);
            if (!page) return error('tab not found', 404);
            const result = await page.evaluate((maxText) => {
              const visible = (element: Element) => { const s = getComputedStyle(element); return s.display !== 'none' && s.visibility !== 'hidden'; };
              const nodes = Array.from(document.querySelectorAll('a,button,input,textarea,select,[role]')).filter(visible).slice(0, 100);
              return { text: (document.body?.innerText ?? '').slice(0, maxText), elements: nodes.map((element) => ({ selector: element.id ? `#${CSS.escape(element.id)}` : element.tagName.toLowerCase(), tag: element.tagName.toLowerCase(), text: (element.textContent ?? '').trim().slice(0, 300), role: element.getAttribute('role') ?? element.getAttribute('type') ?? '' })) };
            }, MAX_TEXT);
            return json({ tabId, url: page.url(), title: await page.title().catch(() => ''), ...result });
          }
          if (url.pathname === '/api/act') {
            if (role !== 'agent') return error('agent token required', 403);
            const tabId = boundedString(body.tabId, 'tabId', 200);
            const expectedUrl = boundedString(body.url, 'url', 4096);
            const action = body.action;
            const selector = boundedString(body.selector, 'selector', 1000);
            const value = action === 'fill' ? boundedString(body.value, 'value', 20000) : undefined;
            let result: { tab: { id: string; url: string; title: string } } | undefined;
            const generation = controlGeneration;
            await enqueueAction(async () => {
              if (control !== 'agent' || controlGeneration !== generation) throw new ApiError('agent control has not been granted or was revoked', 409);
              const page = await findPage(tabId);
              if (!page) throw new ApiError('tab not found', 404);
              if (page.url() !== expectedUrl || !inspectableUrl(expectedUrl)) throw new ApiError('tab URL changed', 409);
              const documentGeneration = await page.evaluate(() => performance.timeOrigin);
              const locator = page.locator(selector);
              if (await locator.count() !== 1) throw new ApiError('selector must match exactly one element', 409);
              if (control !== 'agent' || controlGeneration !== generation || page.url() !== expectedUrl || await page.evaluate(() => performance.timeOrigin) !== documentGeneration) throw new ApiError('agent control or tab document changed', 409);
              if (action === 'click') await locator.click({ timeout: 1_000, noWaitAfter: true });
              else if (action === 'fill') await locator.fill(value ?? '', { timeout: 1_000 });
              else throw new ApiError('action must be click or fill', 400);
              result = { tab: await pageTarget(page) };
            });
            return json(result);
          }
          if (url.pathname === '/api/annotations') {
            if (role !== 'extension') return error('extension token required', 403);
            const tabId = boundedString(body.tabId, 'tabId', 200);
            const targetUrl = boundedString(body.url, 'url', 4096);
            const page = await findPage(tabId);
            if (!page) return error('tab not found', 404);
            if (page.url() !== targetUrl || !inspectableUrl(targetUrl)) return error('tab URL changed', 409);
            const kind = body.kind;
            if (kind !== 'element' && kind !== 'drawing') return error('invalid annotation kind', 400);
            const comment = boundedString(body.comment ?? '', 'comment', 4000);
            validateAnnotationDetails(body);
            const candidate: Annotation = { id: randomUUID(), createdAt: new Date().toISOString(), session: config.session, spaceId: config.spaceId, tabId, url: targetUrl, title: boundedString(body.title ?? '', 'title', 500), comment, kind, element: body.element, viewport: body.viewport, image: body.image, strokes: body.strokes };
            const annotation = await appendAnnotation(() => candidate);
            return json({ annotation }, 201);
          }
          return error('not found', 404);
        } catch (caught) {
          const message = caught instanceof SyntaxError ? 'invalid JSON' : caught instanceof Error ? caught.message : 'request failed';
          return error(message, caught instanceof ApiError ? caught.status : 400);
        }
      }
      if (request.method !== 'GET') return error('not found', 404);
      const files: Record<string, string> = { '/': 'index.html', '/ui/index.html': 'index.html', '/app.js': 'app.js', '/ui/app.js': 'app.js', '/style.css': 'style.css', '/ui/style.css': 'style.css' };
      const file = files[url.pathname];
      if (!file) return error('not found', 404);
      const type = file.endsWith('.html') ? 'text/html; charset=utf-8' : file.endsWith('.js') ? 'text/javascript; charset=utf-8' : 'text/css; charset=utf-8';
      const response = new Response(Bun.file(join(UI_DIR, file)), { headers: { 'content-type': type, 'cache-control': 'no-store' } });
      return response;
    },
  });
  endpoint = `http://127.0.0.1:${mainServer.port}`;
  await writeFile(join(extensionRuntimeDir, 'config.js'), `globalThis.COCKPIT_SPIKE=${JSON.stringify({ endpoint, token: extensionToken, session: config.session, spaceId: config.spaceId, spaceLabel: config.label })};\n`, { mode: 0o600 });
  await chmod(join(extensionRuntimeDir, 'config.js'), 0o600);

  context = await chromium.launchPersistentContext(profileDir, {
    headless: false,
    chromiumSandbox: true,
    executablePath: config.executable,
    args: [`--disable-extensions-except=${extensionRuntimeDir}`, `--load-extension=${extensionRuntimeDir}`, '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0'],
  });
  let page = context.pages()[0];
  if (!page) page = await context.newPage();
  await page.goto(fixtureUrl, { waitUntil: 'domcontentloaded' });
  const descriptor = { version: 1, endpoint, token: agentToken, session: config.session, spaceId: config.spaceId, spaceLabel: config.label };
  await writeFile(connectionPath, JSON.stringify(descriptor, null, 2), { mode: 0o600 });
  await chmod(connectionPath, 0o600);
  console.log(`Cockpit browser Space ${config.label} (${config.spaceId})`);
  console.log(`UI: ${endpoint}/#${uiToken}`);
  console.log(`Fixture: ${fixtureUrl}`);
  console.log(`Connection: ${connectionPath}`);

  process.once('SIGINT', () => void shutdown().finally(() => process.exit(0)));
  process.once('SIGTERM', () => void shutdown().finally(() => process.exit(0)));
  await new Promise<void>((resolveExit) => { process.once('exit', () => { void resolveExit(); }); });
  } catch (cause) {
    await shutdown();
    throw cause;
  }
}

main().catch((error) => { console.error(error instanceof Error ? error.message : error); process.exitCode = 1; });
