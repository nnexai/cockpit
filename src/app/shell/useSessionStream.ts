import { useEffect, type MutableRefObject } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { SessionStreamMessage } from "../../protocol/generated/v1";
import { describeError } from "./model";
import type { OrderedSession } from "./useOrderedSession";

export interface SessionStreamHandle { close(): void }

/** Bootstrap and stream share one observation fence and preserve retained recovery intents. */
export function useSessionStream({ client, session, compatible, sessionAvailable, resyncAttempt, recoveryResyncRef, focusTokenRef, mutationTokenRef, retryFocus, sessionObservation, sessionStream }: {
  client: CockpitClient;
  session: OrderedSession;
  compatible: boolean;
  sessionAvailable: boolean;
  resyncAttempt: number;
  recoveryResyncRef: MutableRefObject<boolean>;
  focusTokenRef: MutableRefObject<number>;
  mutationTokenRef: MutableRefObject<number>;
  retryFocus(): void;
  sessionObservation: MutableRefObject<number>;
  sessionStream: MutableRefObject<SessionStreamHandle | null>;
}) {
  const { state, stateRef, dispatchOrdered } = session;
  useEffect(() => {
    const sessionId = state.sessionId;
    if (!compatible || !sessionId || !sessionAvailable) return;
    const epoch = state.epoch;
    const observation = ++sessionObservation.current;
    const recovering = recoveryResyncRef.current;
    const recoveryFocusToken = focusTokenRef.current;
    const recoveryMutationToken = mutationTokenRef.current;
    const controller = new AbortController();
    let active = true;
    sessionStream.current?.close();
    sessionStream.current = null;
    dispatchOrdered({ type: "snapshot/request", epoch, sessionId });
    void (async () => {
      try {
        const snapshot = await client.sessionSnapshot(sessionId, controller.signal);
        if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
        dispatchOrdered({ type: "snapshot/received", epoch, sessionId, snapshot });
        const stream = await client.subscribeSession(sessionId, (message: SessionStreamMessage) => {
          if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
          if (recovering && message.type === "snapshot" && message.sequence === 1 && recoveryFocusToken === focusTokenRef.current && stateRef.current.focusError) {
            // Reissue the coordinator's retained intent. The bootstrap snapshot
            // is a stale observation and must not become a new user request.
            retryFocus();
          }
          dispatchOrdered({ type: "stream/message", epoch, sessionId, message });
          if (message.type !== "snapshot" || message.sequence !== 1) return;
          if (recovering && recoveryMutationToken === mutationTokenRef.current) {
            recoveryResyncRef.current = false;
          }
        }, (error: unknown) => {
          if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
          const described = describeError(error, "Session stream disconnected");
          dispatchOrdered({ type: "stream/error", epoch, sessionId, code: described.code ?? "stream_disconnected", message: described.message });
        }, controller.signal);
        if (active && !controller.signal.aborted && sessionObservation.current === observation) sessionStream.current = stream; else stream.close();
      } catch (error: unknown) {
        if (!active || controller.signal.aborted || sessionObservation.current !== observation) return;
        const described = describeError(error, "Could not read the session snapshot");
        dispatchOrdered({ type: "stream/error", epoch, sessionId, code: described.code ?? "snapshot_error", message: described.message });
      }
    })();
    return () => {
      active = false;
      controller.abort();
      sessionStream.current?.close();
      sessionStream.current = null;
    };
  }, [client, compatible, sessionAvailable, state.sessionId, state.epoch, resyncAttempt]);
}
