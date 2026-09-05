import type { SessionStreamMessage } from "../protocol/generated/v1";

export const STREAM_SEQUENCE_MAX = 0xffffffff;

export type StreamOrderCursor = {
  sessionId: string;
  generation: number;
  sequence: number;
};

export type StreamOrderCode = "stream_identity" | "stream_sequence" | "stream_generation" | "stream_overflow";

export type StreamOrderResult =
  | {
      kind: "accept";
      classification: "initial" | "same_generation" | "generation_transition";
      cursor: StreamOrderCursor;
    }
  | {
      kind: "ignore";
      classification: "duplicate" | "stale";
      code: "stream_sequence" | "stream_generation";
      cursor: StreamOrderCursor;
    }
  | {
      kind: "error";
      classification: "identity" | "missing_first" | "sequence_gap" | "generation_gap" | "overflow";
      code: StreamOrderCode;
      message: string;
    };

function validCounter(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0 && value <= STREAM_SEQUENCE_MAX;
}

/**
 * Applies the transport-neutral ordering contract to one session stream frame.
 * A null cursor means this is the first frame of a fresh subscription attempt.
 */
export function transitionSessionStream(
  sessionId: string,
  previous: StreamOrderCursor | null,
  message: SessionStreamMessage,
): StreamOrderResult {
  if (message.session_id !== sessionId || (message.type === "snapshot" && message.snapshot.session_id !== sessionId)) {
    return {
      kind: "error",
      classification: "identity",
      code: "stream_identity",
      message: "Session stream message belongs to another session",
    };
  }
  if (previous !== null && previous.sessionId !== sessionId) {
    return {
      kind: "error",
      classification: "identity",
      code: "stream_identity",
      message: "Session stream cursor belongs to another session",
    };
  }
  if (
    (previous !== null && (!validCounter(previous.generation) || !validCounter(previous.sequence))) ||
    !validCounter(message.generation) ||
    !validCounter(message.sequence)
  ) {
    return {
      kind: "error",
      classification: "overflow",
      code: "stream_overflow",
      message: "Session stream generation or sequence exceeds the supported range",
    };
  }
  if (previous === null) {
    if (message.sequence !== 1) {
      return {
        kind: "error",
        classification: "missing_first",
        code: "stream_sequence",
        message: "Session stream must begin at sequence 1",
      };
    }
    return {
      kind: "accept",
      classification: "initial",
      cursor: { sessionId, generation: message.generation, sequence: message.sequence },
    };
  }
  if (message.generation < previous.generation) {
    return {
      kind: "ignore",
      classification: "stale",
      code: "stream_generation",
      cursor: previous,
    };
  }
  if (message.generation > previous.generation + 1) {
    return {
      kind: "error",
      classification: "generation_gap",
      code: "stream_generation",
      message: "Session stream generation gap detected",
    };
  }
  if (message.generation > previous.generation) {
    if (message.sequence !== 1) {
      return {
        kind: "error",
        classification: "missing_first",
        code: "stream_sequence",
        message: "Session stream generation transition must begin at sequence 1",
      };
    }
    return {
      kind: "accept",
      classification: "generation_transition",
      cursor: { sessionId, generation: message.generation, sequence: message.sequence },
    };
  }
  if (message.sequence < previous.sequence) {
    return {
      kind: "ignore",
      classification: "stale",
      code: "stream_sequence",
      cursor: previous,
    };
  }
  if (message.sequence === previous.sequence) {
    return {
      kind: "ignore",
      classification: "duplicate",
      code: "stream_sequence",
      cursor: previous,
    };
  }
  if (previous.sequence === STREAM_SEQUENCE_MAX) {
    return {
      kind: "error",
      classification: "overflow",
      code: "stream_overflow",
      message: "Session stream sequence overflow",
    };
  }
  if (message.sequence !== previous.sequence + 1) {
    return {
      kind: "error",
      classification: "sequence_gap",
      code: "stream_sequence",
      message: "Session stream sequence gap detected",
    };
  }
  return {
    kind: "accept",
    classification: "same_generation",
    cursor: { sessionId, generation: message.generation, sequence: message.sequence },
  };
}
