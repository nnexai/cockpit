import type { ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import type { InboxMessage } from "./wake";
import type { ControlReceipt } from "./cliCall";
import type * as FileSystem from "node:fs";

export interface OutboxEntry {
  operation_id: string;
  request_digest: string;
  args: string[];
  payload?: { flag: string; value: unknown };
  provenance: { run_id: string; attachment_id: string; epoch: number; subject: string; task_revision?: string; claim_id?: string };
  state: "pending" | "delivered" | "conflict";
  result?: unknown;
  error?: { code: string; message: string };
}
export interface CapturedMessage { message: InboxMessage; state: "received" | "presented" | "acked"; native_delivery_id?: string }
export interface DeliveryState {
  run_id: string;
  outbox: OutboxEntry[];
  delivered_through_seq: number;
  acked_through_seq: number;
  entries: CapturedMessage[];
  controls: Array<{ seq: number; body: InboxMessage; receipt: ControlReceipt }>;
}

// Type-only import is erased: installed OMP's loader cannot resolve node:fs.
function nativeFiles(): typeof FileSystem {
  const fs = process.getBuiltinModule?.("fs") as typeof FileSystem | undefined;
  if (!fs) throw new Error("Durable Cockpit delivery requires the runtime process.getBuiltinModule('fs') API; local tools remain available.");
  return fs;
}
const stores = new Map<string, DeliveryState>();
const uncommitted = new WeakSet<DeliveryState>();


export function deliveryStore(runId: string, subject = "main") {
  let state: DeliveryState | undefined;
  let directory: string | undefined;
  const locate = () => {
    if (directory) return directory;
    const credential = process.env.COCKPIT_JOB_CREDENTIAL_FILE;
    const attachment = process.env.COCKPIT_ATTACHMENT_ID;
    const epoch = Number(process.env.COCKPIT_ATTACHMENT_EPOCH);
    if (!credential?.startsWith("/") || !attachment || !Number.isSafeInteger(epoch) || epoch < 1) {
      throw new Error("Private job/epoch delivery storage is unavailable; no semantic operation was submitted.");
    }
    const io = nativeFiles();
    const parent = credential.slice(0, credential.lastIndexOf("/"));
    const privateDirectory = (path: string) => {
      const stat = io.lstatSync(path);
      if (!stat.isDirectory() || stat.isSymbolicLink() || (stat.mode & 0o077) !== 0 || stat.uid !== process.getuid?.()) {
        throw new Error("Cockpit delivery storage is not a private owned directory.");
      }
    };
    privateDirectory(parent);
    const path = `${parent}/delivery-${encodeURIComponent(runId)}-${encodeURIComponent(attachment)}-${epoch}`;
    try {
      io.mkdirSync(path, { mode: 0o700 });
      const parentFd = io.openSync(parent, io.constants.O_RDONLY | io.constants.O_DIRECTORY | io.constants.O_NOFOLLOW);
      try { io.fsyncSync(parentFd); } finally { io.closeSync(parentFd); }
    } catch (error) { if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error; }
    privateDirectory(path);
    directory = path;
    return path;
  };
  const atomic = (path: string, value: unknown) => {
    const io = nativeFiles();
    const temp = `${path}.${crypto.randomUUID()}.tmp`;
    const fd = io.openSync(temp, io.constants.O_CREAT | io.constants.O_EXCL | io.constants.O_WRONLY | io.constants.O_NOFOLLOW, 0o600);
    try {
      try { io.writeFileSync(fd, JSON.stringify(value)); io.fsyncSync(fd); }
      finally { io.closeSync(fd); }
      io.renameSync(temp, path);
      const dir = io.openSync(locate(), io.constants.O_RDONLY | io.constants.O_DIRECTORY | io.constants.O_NOFOLLOW);
      try { io.fsyncSync(dir); } finally { io.closeSync(dir); }
    } finally { try { io.unlinkSync(temp); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; } }
  };
  return {
    load(): DeliveryState {
      const path = `${locate()}/${encodeURIComponent(subject)}.json`;
      state ??= stores.get(path);
      if (state) {
        // A failed fsync/rename must not make cached mutations eligible for
        // transport. Every consumer re-enters load before ACK/submission.
        if (uncommitted.has(state)) { atomic(path, state); uncommitted.delete(state); }
        return state;
      }
      const io = nativeFiles();
      let saved: DeliveryState;
      try {
        const fd = io.openSync(path, io.constants.O_RDONLY | io.constants.O_NOFOLLOW);
        try {
          const stat = io.fstatSync(fd);
          if (!stat.isFile() || (stat.mode & 0o077) !== 0 || stat.uid !== process.getuid?.()) throw new Error("Cockpit delivery record is not private.");
          saved = JSON.parse(io.readFileSync(fd, "utf8"));
        } finally { io.closeSync(fd); }
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
        saved = { run_id: runId, outbox: [], delivered_through_seq: 0, acked_through_seq: 0, entries: [], controls: [] };
      }
      if (saved.run_id !== runId || !Array.isArray(saved.outbox) || !Array.isArray(saved.entries) || !Array.isArray(saved.controls) || !Number.isSafeInteger(saved.delivered_through_seq) || !Number.isSafeInteger(saved.acked_through_seq) || saved.acked_through_seq < 0 || saved.acked_through_seq > saved.delivered_through_seq) {
        throw new Error("Malformed private Cockpit delivery record; preserving it without delivery.");
      }
      state = saved;
      stores.set(path, state);
      return state;
    },
    save() {
      if (!state) throw new Error("Cockpit delivery record is not loaded.");
      uncommitted.add(state);
      atomic(`${locate()}/${encodeURIComponent(subject)}.json`, state);
      uncommitted.delete(state);
    },
    payload(operationId: string, value: unknown): string {
      const path = `${locate()}/${encodeURIComponent(subject)}-${encodeURIComponent(operationId)}-payload.json`;
      atomic(path, value);
      return path;
    },
  };
}

export function subjectFor(ctx: ExtensionContext): string { return ctx.agent.kind === "main" ? "main" : `child-${ctx.agent.id}`; }
export const operationDigest = (args: string[], payload?: { flag: string; value: unknown }): string =>
  new Bun.CryptoHasher("sha256").update(JSON.stringify({ args, payload: payload ?? null })).digest("hex");
