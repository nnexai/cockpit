import { useEffect, useRef, useState } from "react";
import { nativePowerStatus, nativeScreenOff, setNativeStayAwake, type NativePowerStatus } from "../../client/power";

const KEY = "cockpit.stay-awake-while-agents-work";

export function useAgentPower(working: boolean, report: (message: string) => void) {
  const [enabled, setEnabled] = useState(() => {
    try { return localStorage.getItem(KEY) !== "false"; } catch { return true; }
  });
  const [status, setStatus] = useState<NativePowerStatus | null>(null);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const reportRef = useRef(report);
  reportRef.current = report;
  useEffect(() => {
    let active = true;
    void nativePowerStatus().then(value => { if (active) setStatus(value); }).catch(() => { if (active) setStatus(null); });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    if (!status?.stayAwakeAvailable) return;
    const desired = enabled && working;
    queue.current = queue.current.catch(() => undefined).then(() => setNativeStayAwake(desired)).catch(error => {
      reportRef.current(`Could not change stay-awake state: ${String(error)}`);
    });
  }, [enabled, working, status?.stayAwakeAvailable]);
  useEffect(() => () => {
    queue.current = queue.current.catch(() => undefined).then(() => setNativeStayAwake(false)).catch(() => undefined);
  }, []);
  return {
    available: Boolean(status?.stayAwakeAvailable), screenOffAvailable: Boolean(status?.screenOffAvailable), enabled,
    toggle: () => setEnabled(value => {
      const next = !value;
      try { localStorage.setItem(KEY, String(next)); } catch { /* preference remains active for this session */ }
      return next;
    }),
    screenOff: () => { void nativeScreenOff().catch(error => reportRef.current(`Could not turn screen off: ${String(error)}`)); },
  };
}
