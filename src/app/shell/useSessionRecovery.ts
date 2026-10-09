import { useCallback, useEffect, useRef, useState, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import type { OrderedSession } from "./useOrderedSession";

export interface ResyncControl {
  resyncAttempt: number;
  setResyncAttempt: Dispatch<SetStateAction<number>>;
  recoveryResyncRef: MutableRefObject<boolean>;
  autoResyncTimer: MutableRefObject<number | null>;
  autoResyncAttempts: MutableRefObject<number>;
  healthyLiveTimer: MutableRefObject<number | null>;
  clearRecoveryTimers(): void;
  requestResync(): void;
}

/** Timer controls have no effects; their lifecycle remains ordered by the App. */
export function useResyncControl(): ResyncControl {
  const [resyncAttempt, setResyncAttempt] = useState(0);
  const recoveryResyncRef = useRef(false);
  const autoResyncTimer = useRef<number | null>(null);
  const autoResyncAttempts = useRef(0);
  const healthyLiveTimer = useRef<number | null>(null);
  const clearRecoveryTimers = useCallback(() => {
    if (autoResyncTimer.current !== null) window.clearTimeout(autoResyncTimer.current);
    if (healthyLiveTimer.current !== null) window.clearTimeout(healthyLiveTimer.current);
    autoResyncTimer.current = null;
    healthyLiveTimer.current = null;
  }, []);
  const requestResync = useCallback(() => {
    recoveryResyncRef.current = true;
    setResyncAttempt((value) => value + 1);
  }, []);
  return { resyncAttempt, setResyncAttempt, recoveryResyncRef, autoResyncTimer, autoResyncAttempts, healthyLiveTimer, clearRecoveryTimers, requestResync };
}

export function useAutoResync(control: ResyncControl, session: OrderedSession, mountedRef: MutableRefObject<boolean>) {
  const { autoResyncTimer, autoResyncAttempts, healthyLiveTimer, setResyncAttempt } = control;
  const { state, stateRef } = session;
  useEffect(() => {
    if (state.sync !== "live") {
      if (healthyLiveTimer.current !== null) {
        window.clearTimeout(healthyLiveTimer.current);
        healthyLiveTimer.current = null;
      }
      return;
    }
    if (healthyLiveTimer.current !== null) return;
    const epoch = state.epoch;
    const generation = state.generation;
    // A stream snapshot is only a bootstrap boundary; require one second of
    // ordered live traffic so an immediate snapshot/disconnect cannot renew
    // the outage budget indefinitely.
    healthyLiveTimer.current = window.setTimeout(() => {
      healthyLiveTimer.current = null;
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.generation !== generation || stateRef.current.sync !== "live") return;
      autoResyncAttempts.current = 0;
    }, 1000);
    return () => {
      if (healthyLiveTimer.current !== null) {
        window.clearTimeout(healthyLiveTimer.current);
        healthyLiveTimer.current = null;
      }
    };
  }, [state.sync, state.epoch, state.generation]);
  useEffect(() => {
    if (state.sync !== "stale" && state.sync !== "disconnected") return;
    if (autoResyncAttempts.current >= 3 || autoResyncTimer.current !== null) return;
    const epoch = state.epoch;
    const sessionId = state.sessionId;
    const attempt = autoResyncAttempts.current;
    autoResyncTimer.current = window.setTimeout(() => {
      autoResyncTimer.current = null;
      if (!mountedRef.current || stateRef.current.epoch !== epoch || stateRef.current.sessionId !== sessionId
        || (stateRef.current.sync !== "stale" && stateRef.current.sync !== "disconnected")) return;
      autoResyncAttempts.current += 1;
      setResyncAttempt((value) => value + 1);
    }, [250, 500, 1000][attempt] ?? 1000);
    return () => {
      if (autoResyncTimer.current !== null) {
        window.clearTimeout(autoResyncTimer.current);
        autoResyncTimer.current = null;
      }
    };
  }, [state.sync, state.epoch]);
}
