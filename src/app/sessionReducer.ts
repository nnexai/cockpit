import type {
  FocusRequest,
  SessionSnapshotResponse,
  SessionStreamMessage,
  TerminalCommand,
  TerminalOpenRequest,
  TerminalOwnershipState,
  TerminalStreamMessage,
} from "../protocol/generated/v1";

export type SyncState = "idle" | "loading" | "live" | "stale" | "disconnected";

export type SyncError = { code: string; message: string };

export type PaneAttachment = {
  streamId: string | null;
  mode: "observe" | "control";
  ownership: TerminalOwnershipState | null;
  terminalSequence: bigint | null;
  error: SyncError | null;
  disposed: boolean;
};

export type SessionState = {
  epoch: number;
  sessionId: string | null;
  snapshot: SessionSnapshotResponse | null;
  generation: number | null;
  sequence: number;
  sync: SyncState;
  syncError: SyncError | null;
  focusPending: FocusRequest | null;
  focusToken: number;
  focusError: SyncError | null;
  attachments: Record<string, PaneAttachment>;
};

export const initialSessionState: SessionState = {
  epoch: 0,
  sessionId: null,
  snapshot: null,
  generation: null,
  sequence: 0,
  sync: "idle",
  syncError: null,
  focusPending: null,
  focusToken: 0,
  focusError: null,
  attachments: {},
};

export type SessionAction =
  | { type: "switch"; sessionId: string }
  | { type: "snapshot/request"; epoch: number; sessionId: string }
  | { type: "snapshot/received"; epoch: number; sessionId: string; snapshot: SessionSnapshotResponse; preserveStream?: boolean }
  | { type: "stream/message"; epoch: number; sessionId: string; message: SessionStreamMessage }
  | { type: "stream/error"; epoch: number; sessionId: string; code: string; message: string }
  | { type: "focus/request"; epoch: number; sessionId: string; request: FocusRequest; token?: number }
  | { type: "focus/error"; epoch: number; sessionId: string; code: string; message: string; token?: number }
  | { type: "attachment/opened"; epoch: number; sessionId: string; paneId: string; streamId: string; mode: "observe" | "control" }
  | { type: "attachment/message"; epoch: number; sessionId: string; paneId: string; message: TerminalStreamMessage }
  | { type: "attachment/error"; epoch: number; sessionId: string; paneId: string; code: string; message: string }
  | { type: "attachment/dispose"; epoch: number; sessionId: string; paneId: string }
  | { type: "attachment/sequence-error"; epoch: number; sessionId: string; paneId: string; message: string };

const errorOf = (code: string, message: string): SyncError => ({ code, message });

function current(state: SessionState, epoch: number, sessionId: string): boolean {
  return state.epoch === epoch && state.sessionId === sessionId;
}

function attachment(state: SessionState, paneId: string): PaneAttachment {
  return state.attachments[paneId] ?? {
    streamId: null,
    mode: "observe",
    ownership: null,
    terminalSequence: null,
    error: null,
    disposed: false,
  };
}

function acceptsStream(state: SessionState, message: SessionStreamMessage): boolean {
  if (message.session_id !== state.sessionId) return false;
  if (state.sync === "stale" && (state.generation === null || message.generation <= state.generation)) return false;
  if (state.generation === null) return message.sequence === 1;
  if (message.generation < state.generation) return false;
  if (message.generation > state.generation) return message.sequence === 1;
  return message.sequence === state.sequence + 1;
}

export function focusFulfilled(
  snapshot: SessionSnapshotResponse,
  pending: FocusRequest | null,
): boolean {
  if (!pending) return false;
  switch (pending.kind) {
    case "pane":
      return snapshot.focused_pane_id === pending.target_id;
    case "agent":
      return snapshot.agents.some(
        (agent) => agent.pane_id === pending.target_id && agent.focused,
      );
    case "space":
      return snapshot.focused_space_id === pending.target_id;
    case "tab":
      return snapshot.focused_tab_id === pending.target_id;
  }
}

export function sessionReducer(state: SessionState, action: SessionAction): SessionState {
  switch (action.type) {
    case "switch":
      return {
        ...initialSessionState,
        epoch: state.epoch + 1,
        sessionId: action.sessionId,
        sync: "loading",
      };
    case "snapshot/request":
      return current(state, action.epoch, action.sessionId)
        ? { ...state, sync: "loading", syncError: null }
        : state;
    case "snapshot/received":
      if (!current(state, action.epoch, action.sessionId)) return state;
      const pending = state.focusPending;
      const fulfilled = focusFulfilled(action.snapshot, pending);
      return {
        ...state,
        snapshot: action.snapshot,
        generation: action.preserveStream ? state.generation : null,
        sequence: action.preserveStream ? state.sequence : 0,
        sync:
          action.preserveStream &&
          (state.sync === "stale" ||
            state.sync === "disconnected" ||
            (state.sync === "live" && state.generation !== null))
            ? state.sync
            : "loading",
        focusPending: fulfilled ? null : pending,
      };
    case "stream/message": {
      if (!current(state, action.epoch, action.sessionId)) return state;
      const message = action.message;
      if (message.session_id !== state.sessionId) {
        return { ...state, sync: "stale", syncError: errorOf("stream_session", "Session stream identity mismatch") };
      }
      if (state.generation === null && message.sequence !== 1) {
        return { ...state, sync: "stale", syncError: errorOf("stream_sequence", "Session stream did not start at sequence one") };
      }
      if (message.type === "snapshot" && message.snapshot.session_id !== state.sessionId) {
        return { ...state, sync: "stale", syncError: errorOf("stream_snapshot", "Session stream snapshot identity mismatch") };
      }
      if (
        state.generation !== null &&
        message.generation === state.generation &&
        message.sequence > state.sequence + 1
      ) {
        return {
          ...state,
          sync: "stale",
          syncError: errorOf("stream_gap", "Session stream sequence gap; resync required"),
        };
      }
      if (!acceptsStream(state, message)) return state;
      if (message.type === "snapshot") {
        const pending = state.focusPending;
        const fulfilled = focusFulfilled(message.snapshot, pending);
        return {
          ...state,
          snapshot: message.snapshot,
          generation: message.generation,
          sequence: message.sequence,
          sync: "live",
          syncError: null,
          focusPending: fulfilled ? null : pending,
          focusError: fulfilled ? null : state.focusError,
        };
      }
      const sync = message.type === "stale" ? "stale" : "disconnected";
      return {
        ...state,
        generation: message.generation,
        sequence: message.sequence,
        sync,
        syncError: errorOf(message.code, message.message),
      };
    }
    case "stream/error":
      return current(state, action.epoch, action.sessionId)
        ? { ...state, sync: "disconnected", syncError: errorOf(action.code, action.message) }
        : state;
    case "focus/request":
      if (!current(state, action.epoch, action.sessionId)) return state;
      return { ...state, focusPending: action.request, focusToken: action.token ?? state.focusToken + 1, focusError: null };
    case "focus/error":
      if (!current(state, action.epoch, action.sessionId) || (action.token !== undefined && action.token !== state.focusToken)) return state;
      return { ...state, focusPending: null, focusError: errorOf(action.code, action.message) };
    case "attachment/opened":
      if (!current(state, action.epoch, action.sessionId)) return state;
      return {
        ...state,
        attachments: {
          ...state.attachments,
          [action.paneId]: { ...attachment(state, action.paneId), streamId: action.streamId, mode: action.mode, ownership: null, terminalSequence: null, error: null, disposed: false },
        },
      };
    case "attachment/message": {
      if (!current(state, action.epoch, action.sessionId)) return state;
      const old = attachment(state, action.paneId);
      if (old.disposed) return state;
      const message = action.message;
      if (old.streamId && "stream_id" in message && message.stream_id !== old.streamId) return state;
      if (message.type === "frame") {
        let sequence: bigint;
        try {
          sequence = BigInt(message.seq);
        } catch {
          return {
            ...state,
            attachments: {
              ...state.attachments,
              [action.paneId]: { ...old, error: errorOf("terminal_sequence", "Terminal sent an invalid sequence") },
            },
          };
        }
        if (old.terminalSequence === null && !message.full) {
          return {
            ...state,
            attachments: {
              ...state.attachments,
              [action.paneId]: { ...old, error: errorOf("terminal_sequence", "Terminal stream must begin with a full frame") },
            },
          };
        }
        if (!message.full && old.terminalSequence !== null && sequence !== old.terminalSequence + 1n) {
          return {
            ...state,
            attachments: {
              ...state.attachments,
              [action.paneId]: { ...old, error: errorOf("terminal_sequence", "Terminal output sequence is not consecutive") },
            },
          };
        }
        return {
          ...state,
          attachments: {
            ...state.attachments,
            [action.paneId]: { ...old, terminalSequence: sequence, error: null },
          },
        };
      }
      const nextOwnership = message.type === "ownership" ? message.state : old.ownership;
      const error = message.type === "error" || message.type === "disconnected"
        ? errorOf(message.code, message.message)
        : message.type === "closed" ? errorOf("terminal_closed", message.reason) : null;
      return { ...state, attachments: { ...state.attachments, [action.paneId]: { ...old, ownership: nextOwnership, error } } };
    }
    case "attachment/error":
      if (!current(state, action.epoch, action.sessionId)) return state;
      return { ...state, attachments: { ...state.attachments, [action.paneId]: { ...attachment(state, action.paneId), error: errorOf(action.code, action.message) } } };
    case "attachment/sequence-error":
      if (!current(state, action.epoch, action.sessionId)) return state;
      return { ...state, attachments: { ...state.attachments, [action.paneId]: { ...attachment(state, action.paneId), error: errorOf("terminal_sequence", action.message) } } };
    case "attachment/dispose":
      if (!current(state, action.epoch, action.sessionId)) return state;
      return { ...state, attachments: { ...state.attachments, [action.paneId]: { ...attachment(state, action.paneId), disposed: true } } };
    default:
      return state;
  }
}

export function makeTerminalRequest(sessionId: string, paneId: string, mode: "observe" | "control", takeover: boolean, cols: number, rows: number): TerminalOpenRequest {
  return { session_id: sessionId, pane_id: paneId, mode, takeover, cols, rows };
}

export type { FocusRequest, SessionSnapshotResponse, SessionStreamMessage, TerminalCommand, TerminalOpenRequest, TerminalStreamMessage };
