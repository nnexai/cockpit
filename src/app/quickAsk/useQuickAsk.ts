import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { QuickAskHarness, QuickAskLocation } from "../../protocol/generated/v1";
import { QUICK_ASK_AGENTS, quickAskPromptValidation } from "./quickAsk";

export interface UseQuickAskOptions {
  client: CockpitClient;
  open: boolean;
  setOpen: (open: boolean) => void;
  sessionId: string | null;
  serverInstance: string | null;
  spaceId: string | null;
  initialCwd: string;
  onOpenTab: (location: QuickAskLocation) => void;
}

export interface QuickAskState {
  harness: QuickAskHarness;
  setHarness: (harness: QuickAskHarness) => void;
  /** Installed agents; null until the PATH check answers (OMP is assumed meanwhile). */
  installed: readonly QuickAskHarness[] | null;
  question: string;
  setQuestion: (question: string) => void;
  cwd: string;
  setCwd: (cwd: string) => void;
  busy: boolean;
  error: string | null;
  ask: () => void;
}

/** Composer state lives in the window: closing keeps the draft; a launched question clears it. */
export function useQuickAsk({ client, open, setOpen, sessionId, serverInstance, spaceId, initialCwd, onOpenTab }: UseQuickAskOptions): QuickAskState {
  const [harness, setHarness] = useState<QuickAskHarness>("omp");
  const [installed, setInstalled] = useState<readonly QuickAskHarness[] | null>(null);
  const [question, setQuestion] = useState("");
  const [cwd, setCwd] = useState(initialCwd);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);

  useEffect(() => {
    if (!open) return;
    setCwd(initialCwd);
    setError(null);
    let active = true;
    void client.quickAskAvailability().then(value => {
      if (!active) return;
      const found = QUICK_ASK_AGENTS.map(agent => agent.id).filter(id => value.agents.some(agent => agent.harness === id && agent.found));
      setInstalled(found);
      setHarness(current => found.includes(current) || found.length === 0 ? current : found[0]!);
    }, () => { if (active) setInstalled(null); });
    return () => { active = false; };
    // Re-read the folder only when the composer opens, so an edit survives later snapshots.
  }, [open, client]);

  const ask = useCallback(() => {
    if (inFlight.current || !question.trim()) return;
    // Pasted code often carries tabs and CRLF; agents read them fine as spaces and line feeds.
    const prompt = question.replace(/\r\n?/gu, "\n").replace(/\t/gu, "    ");
    const invalid = quickAskPromptValidation(prompt) ?? (!cwd.startsWith("/") ? "Choose an absolute folder." : null)
      ?? (!sessionId || !serverInstance || !spaceId ? "Select a Space first." : null);
    if (invalid) { setError(invalid); return; }
    inFlight.current = true;
    setBusy(true);
    setError(null);
    void client.quickAsk(sessionId!, { server_instance: serverInstance!, space_id: spaceId!, cwd, harness, prompt })
      .then(location => {
        setQuestion("");
        setOpen(false);
        onOpenTab(location);
      }, (cause: unknown) => setError(cause instanceof Error ? cause.message : "Could not start the agent."))
      .finally(() => { inFlight.current = false; setBusy(false); });
  }, [client, question, cwd, harness, sessionId, serverInstance, spaceId, setOpen, onOpenTab]);

  return { harness, setHarness, installed, question, setQuestion, cwd, setCwd, busy, error, ask };
}
