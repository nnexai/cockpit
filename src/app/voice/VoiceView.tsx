import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { selectClient } from "../../client/select";
import type { VoiceTransportApi } from "../../client/voiceTransport";
import { UiIcon } from "../UiIcon";
import { VoiceContainer } from "./VoiceContainer";
import type { VoiceChoice } from "./VoiceScreen";

type VoiceViewDestination = "workbench" | "inbox";
type VoiceContext = { client: CockpitClient; sessionId: string; roots: VoiceChoice[]; projects: VoiceChoice[]; workspaces: VoiceChoice[] };

export function VoiceView() {
  const [context, setContext] = useState<VoiceContext | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [destination, setDestination] = useState<VoiceViewDestination | null>(null);
  const [navigationError, setNavigationError] = useState<string | null>(null);
  const navigatingRef = useRef(false);
  const stopRef = useRef<(() => Promise<void>) | null>(null);
  const onStopReady = useCallback((stop: (() => Promise<void>) | null) => { stopRef.current = stop; }, []);

  useEffect(() => {
    let current = true;
    void (async () => {
      const [{ session_id: sessionId }, client] = await Promise.all([
        invoke<{ session_id: string | null }>("cockpit_voice_view_context"), selectClient(),
      ]);
      if (!sessionId) throw new Error("Select a session before starting voice.");
      const [lookup, repositories, snapshot] = await Promise.all([
        client.orchestrationLiaisonLookup({ session_id: sessionId, selected_root_id: null }),
        client.repositories(),
        client.sessionSnapshot(sessionId),
      ]);
      if (lookup.status !== "resolved") throw new Error(lookup.message);
      if (current) setContext({
        client, sessionId,
        roots: lookup.roots.map(root => ({ id: root.root_id, label: root.root_id })),
        projects: repositories.repositories.map(repo => ({ id: repo.repository_id, label: repo.name })),
        workspaces: snapshot.spaces.map(space => ({ id: space.id, label: space.label })),
      });
    })().catch(error => { if (current) setLoadError(error instanceof Error ? error.message : String(error)); });
    return () => { current = false; };
  }, []);

  const api = useMemo<VoiceTransportApi | null>(() => context ? ({
    connect: request => context.client.voiceConnect(request),
    events: (request, signal) => context.client.voiceEvents(request, signal),
    control: request => context.client.voiceControl(request),
  }) : null, [context]);

  const navigate = async (view: VoiceViewDestination) => {
    if (navigatingRef.current) return;
    navigatingRef.current = true;
    setNavigationError(null);
    setDestination(view);
    try {
      await stopRef.current?.();
      await invoke("cockpit_voice_view_close", { view });
    } catch (error) {
      setNavigationError(error instanceof Error ? error.message : String(error));
      setDestination(null);
      navigatingRef.current = false;
    }
  };

  return <div className="app-shell">
    <nav className="view-bar" aria-label="Application views">
      <button type="button" disabled={destination !== null} onClick={() => void navigate("workbench")}><UiIcon name="grid" /> <span className="view-bar-label">Workbench</span></button>
      <button type="button" disabled={destination !== null} onClick={() => void navigate("inbox")}><UiIcon name="comment" /> <span className="view-bar-label">Inbox</span></button>
      <button type="button" aria-current="page">Voice</button>
    </nav>
    {navigationError && <p role="alert">Could not return to the selected view: {navigationError}</p>}
    <div inert={destination !== null} style={{ display: "contents" }}>
      {loadError ? <section role="alert">Could not load voice choices: {loadError}</section>
        : context && api ? <VoiceContainer sessionId={context.sessionId} roots={context.roots} projects={context.projects} workspaces={context.workspaces} api={api} onStopReady={onStopReady} />
        : <section aria-label="Voice">Loading voice choices…</section>}
    </div>
  </div>;
}
