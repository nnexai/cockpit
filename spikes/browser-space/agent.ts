import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

type Descriptor = { version: number; endpoint: string; token: string; session: string; spaceId: string; spaceLabel: string };

function usage(): never {
  throw new Error('usage: bun agent.ts [--connection PATH] status|annotations|inspect TAB|click TAB SELECTOR EXPECTED_URL|fill TAB SELECTOR VALUE EXPECTED_URL');
}
function option(args: string[], name: string): string | undefined {
  const index = args.indexOf(name);
  if (index < 0) return undefined;
  const value = args[index + 1];
  if (!value || value.startsWith('--')) throw new Error(`${name} requires a path`);
  return value;
}
async function descriptorPath(args: string[]): Promise<string> {
  const explicit = option(args, '--connection');
  const fromEnvironment = process.env.COCKPIT_BROWSER_CONNECTION;
  const path = explicit ?? fromEnvironment;
  if (!path) throw new Error('connection descriptor required (--connection or COCKPIT_BROWSER_CONNECTION)');
  return resolve(path);
}
async function loadDescriptor(path: string): Promise<Descriptor> {
  let parsed: unknown;
  try { parsed = JSON.parse(await readFile(path, 'utf8')); } catch { throw new Error(`cannot read connection descriptor: ${path}`); }
  if (!parsed || typeof parsed !== 'object') throw new Error('invalid connection descriptor');
  const descriptor = parsed as Partial<Descriptor>;
  if (descriptor.version !== 1 || typeof descriptor.endpoint !== 'string' || !/^https?:\/\/(127\.0\.0\.1|localhost)(?::\d+)?$/.test(descriptor.endpoint) || typeof descriptor.token !== 'string' || descriptor.token.length < 20 || typeof descriptor.session !== 'string' || typeof descriptor.spaceId !== 'string' || typeof descriptor.spaceLabel !== 'string') throw new Error('invalid connection descriptor');
  const workspace = process.env.HERDR_WORKSPACE_ID ?? process.env.COCKPIT_WORKSPACE_ID;
  if (workspace && workspace !== descriptor.spaceId) throw new Error('connection descriptor Space does not match workspace environment');
  const session = process.env.COCKPIT_HERDR_SESSION;
  if (session && session !== descriptor.session) throw new Error('connection descriptor session does not match COCKPIT_HERDR_SESSION');
  return descriptor as Descriptor;
}
async function request(descriptor: Descriptor, route: string, body?: Record<string, unknown>): Promise<unknown> {
  const response = await fetch(`${descriptor.endpoint.replace(/\/$/, '')}${route}`, {
    method: body ? 'POST' : 'GET',
    headers: { authorization: `Bearer ${descriptor.token}`, ...(body ? { 'content-type': 'application/json' } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await response.text();
  let result: unknown;
  try { result = text ? JSON.parse(text) : {}; } catch { result = { error: text || `HTTP ${response.status}` }; }
  if (!response.ok) throw new Error(typeof result === 'object' && result && 'error' in result ? String((result as { error: unknown }).error) : `HTTP ${response.status}`);
  return result;
}
function required(args: string[], index: number, name: string): string {
  const value = args[index];
  if (!value) throw new Error(`${name} is required`);
  return value;
}
async function run() {
  const args = Bun.argv.slice(2);
  const path = await descriptorPath(args);
  const commandArgs: string[] = [];
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === '--connection') { index += 1; continue; }
    commandArgs.push(args[index]);
  }
  const command = commandArgs[0];
  if (!command || command === '--help') usage();
  const descriptor = await loadDescriptor(path);
  let result: unknown;
  if (command === 'status' && commandArgs.length === 1) result = await request(descriptor, '/api/state');
  else if (command === 'annotations' && commandArgs.length === 1) result = await request(descriptor, '/api/annotations');
  else if (command === 'inspect' && commandArgs.length === 2) result = await request(descriptor, '/api/inspect', { tabId: required(commandArgs, 1, 'TAB') });
  else if (command === 'click' && commandArgs.length === 4) result = await request(descriptor, '/api/act', { tabId: commandArgs[1], action: 'click', selector: commandArgs[2], url: commandArgs[3] });
  else if (command === 'fill' && commandArgs.length === 5) result = await request(descriptor, '/api/act', { tabId: commandArgs[1], action: 'fill', selector: commandArgs[2], value: commandArgs[3], url: commandArgs[4] });
  else usage();
  console.log(JSON.stringify(result, null, 2));
}
run().catch((error) => { console.error(error instanceof Error ? error.message : error); process.exitCode = 1; });
