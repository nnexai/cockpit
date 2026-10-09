import { useCallback, useRef } from "react";
import { MAX_INPUT_JOBS, ownsBrowserControl, type InputJob, type WheelIntent } from "./browserPaneModel";
import type { BrowserPaneState } from "./useBrowserPaneState";
import type { BrowserViewCommandApi } from "./useBrowserViewCommand";

export function useBrowserInputQueue(state: BrowserPaneState, view: BrowserViewCommandApi): BrowserInputQueue {
  const { inputJobsRef, inputGenerationRef, inputOverloadedRef, releaseOverloadedInputRef, remotePointerRef, remotePointerIntentRef, remotePointRef, gestureRef, snapshotRef, inputSequence, setStatus, setMessage, setGesture } = state;
  const { command } = view;
  const inputDrainRef = useRef<Promise<void> | null>(null);
  const runInputJobs = useCallback((): Promise<void> => {
    if (inputDrainRef.current) return inputDrainRef.current;
    const drain = Promise.resolve().then(async () => {
      try {
        while (inputJobsRef.current.length > 0) {
          const job = inputJobsRef.current.shift()!;
          if (job.generation !== inputGenerationRef.current && job.kind !== "release") continue;
          try { await job.run(); } catch { /* command reports transport errors */ }
        }
      } finally {
        inputDrainRef.current = null;
      }
    });
    inputDrainRef.current = drain;
    return drain;
  }, []);
  const enqueueInput = useCallback((kind: InputJob["kind"], run: () => Promise<void>, wheel?: WheelIntent): Promise<void> => {
    const jobs = inputJobsRef.current;
    const generation = inputGenerationRef.current;
    const next = { generation, kind, run };
    if (kind === "release") {
      jobs.push(next);
    } else if (kind === "move") {
      const pending = jobs.findIndex((job) => job.kind === "move");
      if (pending >= 0) jobs[pending] = next;
      else if (jobs.length < MAX_INPUT_JOBS) jobs.push(next);
      else {
        setStatus("error");
        setMessage("Browser input queue is full; motion was refused. Release and retry the gesture.");
        return Promise.resolve();
      }
    } else if (kind === "wheel") {
      const last = jobs.at(-1);
      if (wheel && last?.kind === "wheel" && last.generation === generation && last.wheel
        && last.wheel.clientX === wheel.clientX && last.wheel.clientY === wheel.clientY
        && last.wheel.modifiers === wheel.modifiers) {
        last.wheel.deltaX += wheel.deltaX;
        last.wheel.deltaY += wheel.deltaY;
        return runInputJobs();
      }
      if (jobs.length >= MAX_INPUT_JOBS) {
        setStatus("error");
        setMessage("Browser input queue is full; wheel movement was refused. Retry the scroll.");
        return Promise.resolve();
      }
      jobs.push({ ...next, wheel });
    } else if (jobs.length >= MAX_INPUT_JOBS) {
      inputOverloadedRef.current = true;
      inputGenerationRef.current += 1;
      jobs.length = 0;
      remotePointerRef.current = null;
      remotePointerIntentRef.current = null;
      remotePointRef.current = null;
      gestureRef.current = null;
      setGesture(null);
      releaseOverloadedInputRef.current?.();
      setStatus("error");
      setMessage("Browser input queue is full; held input was refused. Control was released; retry the gesture.");
      return Promise.resolve();
    } else {
      jobs.push(next);
    }
    return runInputJobs();
  }, [runInputJobs]);
  const flushInput = runInputJobs;
  const releaseRemotePointer = useCallback((): Promise<void> => {
    const intent = remotePointerIntentRef.current;
    const current = snapshotRef.current;
    remotePointerRef.current = null;
    remotePointerIntentRef.current = null;
    remotePointRef.current = null;
    if (!current || !intent || !ownsBrowserControl(current)) return flushInput();
    const where = { ...intent.location, lease_generation: current.control.lease_generation };
    return enqueueInput("release", async () => {
      const input_sequence = inputSequence.current++;
      await command({ type: "pointer", location: where, input: { kind: "cancel", button: null, x: intent.point.x, y: intent.point.y, buttons: 0, modifiers: 0, click_count: 0, input_sequence } });
    });
  }, [command, enqueueInput, flushInput]);
  const nextInput = (): number => inputSequence.current++;
  return { enqueueInput, flushInput, releaseRemotePointer, nextInput };
}

export interface BrowserInputQueue {
  enqueueInput(kind: InputJob["kind"], run: () => Promise<void>, wheel?: WheelIntent): Promise<void>;
  flushInput(): Promise<void>; releaseRemotePointer(): Promise<void>; nextInput(): number;
}
