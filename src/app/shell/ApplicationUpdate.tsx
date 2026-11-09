import { useCallback, useEffect, useRef, useState } from "react";
import type { BackendSurfaceStatus, CockpitClient } from "../../client/CockpitClient";
import { trapModalTab, useModalFocus } from "../input/modal";
import { UpdateChip } from "./UpdateChip";
import "../projects/taskSetup.css";

function message(error: unknown): string {
  return error instanceof Error ? error.message : typeof error === "object" && error !== null && "message" in error && typeof error.message === "string" ? error.message : "Could not reach the background service.";
}
function short(id: string | null | undefined): string { return id ? id.slice(0, 8) : "unknown"; }

export type ApplicationUpdateState = {
  status: BackendSurfaceStatus | null; dialog: boolean; setDialog(open: boolean): void;
  error: string | null; active: boolean; different: boolean; reload: boolean; stopped: boolean;
  combined: boolean; available: boolean; label: string; open(): void; run(): Promise<void>;
};
export function useApplicationUpdate(client: CockpitClient): ApplicationUpdateState {
  const [status, setStatus] = useState<BackendSurfaceStatus | null>(null);
  const [dialog, setDialog] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [ticket, setTicket] = useState<string | null>(null);
  const mounted = useRef(true);
  const hasStatus = useRef(false);
  const pollingMode = useRef({ active: false, dialog: false });
  const updatePollingMode = useRef<(() => void) | null>(null);
  const inFlight = useRef<{ client: CockpitClient; promise: Promise<BackendSurfaceStatus> } | null>(null);
  const currentClient = useRef(client);
  currentClient.current = client;
  const refresh = useCallback(() => {
    if (inFlight.current?.client === client) return inFlight.current.promise;
    const promise: Promise<BackendSurfaceStatus> = Promise.resolve().then(() => client.backendStatus()).then(next => {
      if (mounted.current && currentClient.current === client) { hasStatus.current = true; setStatus(next); setError(current => current?.startsWith("Could not refresh service status:") ? null : current); }
      return next;
    }).finally(() => {
      if (inFlight.current?.promise === promise) inFlight.current = null;
    });
    inFlight.current = { client, promise };
    return promise;
  }, [client]);
  const restart = status?.restart;
  const active = pending || Boolean(ticket && restart?.started_at !== ticket) || restart?.result.state === "running";
  pollingMode.current = { active, dialog };
  useEffect(() => {
    mounted.current = true;
    let timer: number | undefined;
    let stopped = false;
    let pendingPoll = false, refreshAgain = false;
    const isPageHidden = () => document.visibilityState === "hidden";
    const poll = async () => {
      if (stopped || isPageHidden()) return;
      if (pendingPoll) { refreshAgain = true; return; }
      pendingPoll = true;
      try { await refresh(); }
      catch (cause) { if (hasStatus.current && !pollingMode.current.active && !stopped) setError(`Could not refresh service status: ${message(cause)}`); }
      pendingPoll = false;
      if (stopped || isPageHidden()) return;
      if (refreshAgain) { refreshAgain = false; void poll(); }
      else if (pollingMode.current.active || pollingMode.current.dialog) timer = window.setTimeout(poll, pollingMode.current.active ? 1000 : 2000);
    };
    const focus = () => { clearTimeout(timer); void poll(); };
    const visibility = () => { clearTimeout(timer); if (isPageHidden()) refreshAgain = false; else void poll(); };
    updatePollingMode.current = () => {
      clearTimeout(timer);
      if ((pollingMode.current.active || pollingMode.current.dialog) && !pendingPoll) void poll();
    };
    void poll();
    window.addEventListener("focus", focus);
    document.addEventListener("visibilitychange", visibility);
    return () => { stopped = true; mounted.current = false; updatePollingMode.current = null; clearTimeout(timer); window.removeEventListener("focus", focus); document.removeEventListener("visibilitychange", visibility); };
  }, [refresh]);
  useEffect(() => { updatePollingMode.current?.(); }, [active, dialog]);
  useEffect(() => {
    if (ticket === null) return;
    if (restart?.started_at === ticket && restart.result.state !== "running") { setTicket(null); return; }
    const timeout = window.setTimeout(() => { setError("The service restart has not completed. Check its status before trying again."); setTicket(null); }, 150000);
    return () => clearTimeout(timeout);
  }, [ticket, restart?.started_at, restart?.result.state]);
  const backend = status?.backend, surface = status?.surface;
  const combined = Boolean(surface?.kind === "native" && surface.installed && surface.update_installed);
  const different = combined || Boolean(backend?.installed && backend.update_installed);
  const entry = surface?.kind === "web" ? surface.frontend_entry : null;
  const pageEntry = typeof document === "undefined" ? null : document.querySelector<HTMLScriptElement>('script[type="module"][src]')?.getAttribute("src");
  const reload = Boolean(entry && pageEntry && new URL(entry, location.href).pathname !== new URL(pageEntry, location.href).pathname);
  const stopped = backend?.state === "stopped" || backend?.state === "legacy_owner";
  const failure = error || (restart?.result.state === "failed" ? `${restart.result.code}: ${restart.result.message}` : restart?.result.state === "interrupted" ? "The service restart was interrupted. Check the background service before trying again." : null);
  const webOld = Boolean(surface?.kind === "web" && surface.installed && surface.update_installed);
  const problem = backend?.state === "endpoint_mismatch" || backend?.state === "unresponsive";
  const available = Boolean(different || reload || stopped || active || failure || webOld || problem);
  const label = active ? (restart?.relaunch_app || combined ? "Updating…" : "Restarting service…") : failure ? "Restart failed · details" : stopped ? "Background service stopped" : problem ? "Service unavailable · details" : different ? (combined ? "Update installed · restart" : "Update installed · restart service") : reload ? "New version · reload" : "Web server update installed";
  const run = async () => {
    if (!status || pending) return;
    setError(null);
    setPending(true);
    try {
      if (stopped) { await client.backendStart(); await refresh(); }
      else if (different && backend?.instance_id) {
        if (combined && !client.updateAndRelaunch) throw new Error("This window cannot relaunch Cockpit.");
        const result = combined ? await client.updateAndRelaunch!(backend.instance_id) : await client.backendRestart(backend.instance_id);
        setTicket(result.started_at);
        await refresh();
      } else if (reload) {
        await refresh();
        window.location.reload();
      }
    } catch (cause) { setError(message(cause)); }
    finally { setPending(false); }
  };
  return { status, dialog, setDialog, error: failure, active, different, reload, stopped, combined, available, label, open: () => setDialog(true), run };
}

export function ApplicationUpdateSlot({ update, client, onUpdate }: { update: ApplicationUpdateState; client: CockpitClient; onUpdate(sourcePath: string): void }) {
  if (!update.available) return <UpdateChip client={client} onUpdate={onUpdate} />;
  return <button type="button" className={`tab-strip-action update-chip${update.error ? " is-error" : ""}`} aria-haspopup="dialog" aria-busy={update.active} title={`Running ${short(update.status?.backend.build_commit)} · installed ${short(update.status?.installed_build_commit)}`} onClick={update.open}>{update.active ? "" : update.error ? "! " : "↑ "}{update.label}</button>;
}

export function ApplicationUpdateDialog({ update }: { update: ApplicationUpdateState }) {
  const close = () => update.setDialog(false);
  const ref = useModalFocus<HTMLElement>(close);
  const status = update.status;
  if (!status) return null;
  const counts = status.backend.ephemeral;
  const action = update.stopped ? "Start background service" : update.reload && !update.different ? "Reload Cockpit" : update.combined ? "Restart Cockpit to update" : "Restart background service";
  const actionable = update.stopped || update.reload || (update.different && status.backend.state === "running" && Boolean(status.backend.instance_id));
  return <div className="overlay-scrim" role="presentation" onPointerDown={event => { if (event.target === event.currentTarget) close(); }}><section ref={ref} className="setup-dialog task-setup application-update-dialog" role="dialog" aria-modal="true" aria-labelledby="application-update-title" aria-describedby="application-update-description" onKeyDown={event => trapModalTab(event, ref.current)}>
    <header className="task-setup-header"><h2 id="application-update-title">{actionable ? action : "Cockpit service status"}</h2></header>
    <div className="task-setup-body task-setup-summary" id="application-update-description">
      {update.active ? <p role="status">Updating Cockpit… The service restart continues if you close this dialog.</p> : status.backend.state === "legacy_owner" ? <p>Quit the older Cockpit app or web server to finish the upgrade, then start the background service.</p> : status.backend.state === "endpoint_mismatch" ? <p>The running service uses a different Herdr endpoint. Running: {JSON.stringify(status.backend.herdr)}. Expected: {JSON.stringify(status.backend.expected_herdr)}.</p> : update.stopped ? <p>The background service is not running. Start it to resume service operations. {status.backend.last_exit?.message ?? status.backend.last_exit?.code}</p> : update.reload && !update.different ? <p>Reloading this window may discard unsent text. Saved Notes drafts remain in this browser.</p> : update.different ? <><p>Restarting the background service ends Browser sessions, comments and widgets. Save any unsent work first. Terminals and agents keep running; open web pages reconnect.</p>{counts ? <p>Open: {counts.browser_associations} Browser sessions, {counts.comment_batches} comment batches, {counts.widgets} widgets.</p> : <p>Could not read what is open; Browser sessions, comments and widgets may end.</p>}{update.combined ? <p>Cockpit will close and reopen after the service starts.</p> : null}</> : <p>Background service: {status.backend.state}.</p>}
      <p>Service build: {short(status.backend.build_commit)} · {status.surface.kind === "native" ? "App" : "Web server"} build: {short(status.surface.build_commit)} · Installed: {short(status.installed_build_commit)}</p>
      {!status.surface.installed ? <p>This is a development or unmanaged build.</p> : status.surface.kind === "web" && status.surface.update_installed ? <p>The web server is a different build from the installed version. Run <code>cockpit-cli web restart</code> if it is managed.</p> : null}
      <div className="task-setup-status">{update.error ? <p className="is-error" role="alert">{update.error}</p> : null}</div>
    </div>
    <footer className="task-setup-footer"><button type="button" onClick={close}>{update.active ? "Close" : "Cancel"}</button>{!update.active && actionable ? <button type="button" className="setup-primary" onClick={() => void update.run()}>{action}</button> : null}</footer>
  </section></div>;
}
