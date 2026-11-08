import type { BackendRestart, BackendSurfaceStatus, RestartTicket } from "./CockpitClient";

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Invalid backend status");
  return value as Record<string, unknown>;
}
function nullableString(value: unknown): value is string | null {
  return value === null || typeof value === "string";
}
export function parseRestartTicket(value: unknown): RestartTicket {
  const record = object(value);
  if (!Number.isSafeInteger(record.pid) || (record.pid as number) <= 0 || typeof record.started_at !== "string" || typeof record.relaunch_app !== "boolean") throw new Error("Invalid restart ticket");
  return { pid: record.pid as number, started_at: record.started_at, relaunch_app: record.relaunch_app };
}
function parseRestart(value: unknown): BackendRestart {
  const record = object(value), ticket = parseRestartTicket(value), result = object(record.result);
  if (typeof record.step !== "string" || !nullableString(record.finished_at)) throw new Error("Invalid restart status");
  if (result.state === "failed") {
    if (typeof result.code !== "string" || typeof result.message !== "string") throw new Error("Invalid restart failure");
    return { ...ticket, step: record.step, finished_at: record.finished_at, result: { state: "failed", code: result.code, message: result.message } };
  }
  if (result.state !== "running" && result.state !== "ok" && result.state !== "interrupted") throw new Error("Invalid restart result");
  return { ...ticket, step: record.step, finished_at: record.finished_at, result: { state: result.state } };
}
function endpoint(value: unknown): BackendSurfaceStatus["backend"]["herdr"] {
  if (value == null) return null;
  const item = object(value);
  if (typeof item.executable !== "string" || !nullableString(item.session) || !nullableString(item.socket)) throw new Error("Invalid Herdr endpoint");
  return { executable: item.executable, session: item.session, socket: item.socket };
}
export function parseBackendSurfaceStatus(value: unknown): BackendSurfaceStatus {
  const root = object(value), backend = object(root.backend), surface = object(root.surface);
  if (typeof backend.state !== "string" || !["running", "starting", "stopped", "legacy_owner", "endpoint_mismatch", "unresponsive"].includes(backend.state)
    || !nullableString(backend.instance_id) || !nullableString(backend.build_commit) || typeof backend.installed !== "boolean" || typeof backend.update_installed !== "boolean"
    || (surface.kind !== "native" && surface.kind !== "web") || typeof surface.installed !== "boolean" || typeof surface.update_installed !== "boolean"
    || !nullableString(surface.build_commit) || !nullableString(surface.frontend_entry) || !nullableString(root.installed_build_commit)) throw new Error("Invalid backend status");
  let last_exit: BackendSurfaceStatus["backend"]["last_exit"] = null;
  if (backend.last_exit != null) {
    const exit = object(backend.last_exit);
    if (typeof exit.code !== "string" || typeof exit.at !== "string" || (exit.message != null && typeof exit.message !== "string")) throw new Error("Invalid backend exit");
    last_exit = { code: exit.code, at: exit.at, message: exit.message as string | null | undefined };
  }
  let ephemeral: BackendSurfaceStatus["backend"]["ephemeral"] = null;
  if (backend.ephemeral != null) {
    const counts = object(backend.ephemeral);
    if (![counts.browser_associations, counts.comment_batches, counts.widgets].every(n => typeof n === "number" && Number.isSafeInteger(n) && n >= 0)) throw new Error("Invalid backend impact");
    ephemeral = { browser_associations: counts.browser_associations as number, comment_batches: counts.comment_batches as number, widgets: counts.widgets as number };
  }
  return {
    backend: { state: backend.state as BackendSurfaceStatus["backend"]["state"], instance_id: backend.instance_id, build_commit: backend.build_commit, installed: backend.installed, update_installed: backend.update_installed, last_exit, ephemeral, herdr: endpoint(backend.herdr), expected_herdr: endpoint(backend.expected_herdr) },
    surface: { kind: surface.kind, installed: surface.installed, update_installed: surface.update_installed, build_commit: surface.build_commit, frontend_entry: surface.frontend_entry },
    installed_build_commit: root.installed_build_commit,
    restart: root.restart == null ? null : parseRestart(root.restart),
  };
}
