import type { HandoffBinding } from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

export interface VoiceConnectRequest { session_id: string; sdp_offer: string; selected_root_id?: string | null; project_scope?: HandoffBinding | null }
export interface VoiceConnectResponse { call_id: string; generation: number; sdp_answer: string; voice: string; liaison_root_id: string }
export interface VoiceEventsRequest { call_id: string; generation: number; after_seq: number; wait_ms: number }
export interface VoiceEventsResponse { events: VoiceEvent[]; next_seq: number; ended: boolean }
export type VoiceControl = { type: "mute"; muted: boolean } | { type: "stop" };
export interface VoiceControlRequest { call_id: string; generation: number; control: VoiceControl }
export type VoiceEvent = { call_id: string; generation: number; seq: number } & (
  | { kind: "phase"; phase: string }
  | { kind: "transcript"; role: "user" | "assistant"; turn: number; text: string; final: boolean }
  | { kind: "handoff"; receipt: unknown }
  | { kind: "ended"; reason: string | null; error: string | null }
);

function invalid(): never { throw new CockpitClientError("malformed_response", "Invalid voice data"); }
function object(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalid();
  return value as Record<string, unknown>;
}
function exact(value: Record<string, unknown>, keys: readonly string[]): void {
  if (Object.keys(value).some(key => !keys.includes(key))) invalid();
}
function string(value: unknown): string { return typeof value === "string" ? value : invalid(); }
function boolean(value: unknown): boolean { return typeof value === "boolean" ? value : invalid(); }
function counter(value: unknown): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : invalid();
}

export function parseVoiceConnectRequest(value: unknown): VoiceConnectRequest {
  const v = object(value); exact(v, ["session_id", "sdp_offer", "selected_root_id", "project_scope"]);
  let projectScope: HandoffBinding | null | undefined;
  if (v.project_scope === null || v.project_scope === undefined) projectScope = v.project_scope;
  else {
    const scope = object(v.project_scope);
    exact(scope, ["repository_id", "workspace_id", "branch", "base_ref"]);
    projectScope = {
      repository_id: scope.repository_id == null ? null : string(scope.repository_id),
      workspace_id: scope.workspace_id == null ? null : string(scope.workspace_id),
      branch: scope.branch == null ? null : string(scope.branch),
      base_ref: scope.base_ref == null ? null : string(scope.base_ref),
    };
  }
  return {
    session_id: string(v.session_id), sdp_offer: string(v.sdp_offer),
    ...(v.selected_root_id === undefined ? {} : { selected_root_id: v.selected_root_id === null ? null : string(v.selected_root_id) }),
    ...(projectScope === undefined ? {} : { project_scope: projectScope }),
  };
}
export function parseVoiceConnectResponse(value: unknown): VoiceConnectResponse {
  const v = object(value);
  return { call_id: string(v.call_id), generation: counter(v.generation), sdp_answer: string(v.sdp_answer), voice: string(v.voice), liaison_root_id: string(v.liaison_root_id) };
}
export function parseVoiceEventsRequest(value: unknown): VoiceEventsRequest {
  const v = object(value); exact(v, ["call_id", "generation", "after_seq", "wait_ms"]);
  const waitMs = counter(v.wait_ms);
  if (waitMs > 0xffffffff) return invalid();
  return { call_id: string(v.call_id), generation: counter(v.generation), after_seq: counter(v.after_seq), wait_ms: waitMs };
}
export function parseVoiceControlRequest(value: unknown): VoiceControlRequest {
  const v = object(value); exact(v, ["call_id", "generation", "control"]);
  const c = object(v.control);
  let control: VoiceControl;
  if (c.type === "mute") { exact(c, ["type", "muted"]); control = { type: "mute", muted: boolean(c.muted) }; }
  else if (c.type === "stop") { exact(c, ["type"]); control = { type: "stop" }; }
  else return invalid();
  return { call_id: string(v.call_id), generation: counter(v.generation), control };
}
export function parseVoiceEvent(value: unknown): VoiceEvent {
  const v = object(value);
  const identity = { call_id: string(v.call_id), generation: counter(v.generation), seq: counter(v.seq) };
  switch (v.kind) {
    case "phase": return { ...identity, kind: "phase", phase: string(v.phase) };
    case "transcript": {
      if (v.role !== "user" && v.role !== "assistant") return invalid();
      return { ...identity, kind: "transcript", role: v.role, turn: counter(v.turn), text: string(v.text), final: boolean(v.final) };
    }
    case "handoff": {
      if (!Object.hasOwn(v, "receipt")) return invalid();
      return { ...identity, kind: "handoff", receipt: v.receipt };
    }
    case "ended": return { ...identity, kind: "ended", reason: v.reason === null ? null : string(v.reason), error: v.error === null ? null : string(v.error) };
    default: return invalid();
  }
}
export function parseVoiceEventsResponse(value: unknown): VoiceEventsResponse {
  const v = object(value);
  if (!Array.isArray(v.events)) return invalid();
  return { events: v.events.map(parseVoiceEvent), next_seq: counter(v.next_seq), ended: boolean(v.ended) };
}
