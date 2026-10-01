import { useEffect, useRef, useState } from "react";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { SpaceGitAction, SpaceGitActionOutcome, SpaceGitActionRequest, SpaceGitStatus } from "../../protocol/generated/v1";

export type GitNote = { tone: "success" | "error" | "explanation"; message: string };
export type GitActionState = { token: number; request: SpaceGitActionRequest; pending: boolean; note?: GitNote };
export interface SpaceGitActions {
  run(request: SpaceGitActionRequest): void;
  dismiss(root: string): void;
  forStatus(status: SpaceGitStatus | undefined): GitActionState | undefined;
}

export function gitActionReason(status: SpaceGitStatus | undefined, action: SpaceGitAction, pending?: SpaceGitAction, blocked?: string): string | undefined {
  if (blocked) return blocked;
  if (pending) return `${pending === "pull" ? "Pull" : "Push"} in progress`;
  if (!status) return "Not a Git checkout";
  const checkout = status.checkout;
  if (checkout.state === "detached") return "Detached HEAD: check out a branch in a terminal";
  if (checkout.state === "unavailable") return `Git status unavailable: ${checkout.message}`;
  const upstream = checkout.upstream;
  if (upstream.state === "none") return "No upstream branch: set one in a terminal";
  if (upstream.state === "gone") return `Upstream ${upstream.name} is gone: repair it in a terminal`;
  if (upstream.state === "local") return `Upstream ${upstream.name} is a local branch, not a remote`;
  if (upstream.state === "unavailable") return `Git status unavailable: ${upstream.message}`;
  if (upstream.ahead > 0 && upstream.behind > 0) return `Diverged: ${upstream.ahead} local commits, ${upstream.behind} upstream commits. Merge or rebase in a terminal`;
  if (action === "push" && upstream.ahead === 0) return `Nothing to push to ${upstream.name}`;
  return undefined;
}

export function gitActionRequest(status: SpaceGitStatus | undefined, action: SpaceGitAction): SpaceGitActionRequest | null {
  if (!status || gitActionReason(status, action)) return null;
  const checkout = status.checkout;
  if (checkout.state !== "branch" || checkout.upstream.state !== "tracked") return null;
  return { space_id: status.space_id, action, expected_root: checkout.root, expected_branch: checkout.branch, expected_upstream: checkout.upstream.name };
}

export function gitTargetMatches(status: SpaceGitStatus | undefined, request: SpaceGitActionRequest): boolean {
  const checkout = status?.checkout;
  return checkout?.state === "branch" && checkout.root === request.expected_root && checkout.branch === request.expected_branch
    && checkout.upstream.state === "tracked" && checkout.upstream.name === request.expected_upstream;
}

export function gitTargetDetail(label: string, status: SpaceGitStatus | undefined, action: SpaceGitAction): string {
  const checkout = status?.checkout;
  if (checkout?.state !== "branch" || checkout.upstream.state !== "tracked") return label;
  return `${label} · ${checkout.branch} ${action === "push" ? "→" : "←"} ${checkout.upstream.name} · ${action === "push" ? `↑${checkout.upstream.ahead}, never forced` : "fast-forward only; counts since last fetch"}${status?.source === "pane_folder" ? ` · ${checkout.root}` : ""}`;
}

export function gitFailureNote(error: unknown, request: SpaceGitActionRequest): GitNote {
  const code = error instanceof CockpitClientError ? error.operationCode : undefined;
  const notRun = code?.startsWith("invalid_") || ["space_git_target_changed", "space_git_action_ineligible", "space_git_action_in_progress", "space_git_not_run"].includes(code ?? "");
  const action = request.action === "push" ? "Push" : "Pull";
  const detail = error instanceof Error ? error.message : String(error);
  return { tone: "error", message: notRun
    ? `${detail.startsWith(`${action} was not run`) ? detail : `${action} was not run: ${detail}`} (${request.expected_branch} ${request.action === "push" ? "→" : "←"} ${request.expected_upstream})`
    : `Cockpit did not get a result for the ${request.action} on ${request.expected_branch} ${request.action === "push" ? "→" : "←"} ${request.expected_upstream}. ${request.action === "push" ? "It may or may not have reached the remote." : "The branch may or may not have moved."} Check this Space's terminal before trying again.` };
}

export function gitOutcomeNote(outcome: SpaceGitActionOutcome, request: SpaceGitActionRequest): GitNote {
  const target = `${request.expected_branch} ${request.action === "push" ? "→" : "←"} ${request.expected_upstream}`;
  const action = request.action === "push" ? "Push" : "Pull";
  switch (outcome.result) {
    case "updated": return { tone: "success", message: `${request.action === "push" ? "Pushed" : "Fast-forwarded"}${outcome.commits === null ? "" : ` ${outcome.commits} commit${outcome.commits === 1 ? "" : "s"}`}: ${target}.` };
    case "up_to_date": return { tone: "success", message: `Already up to date: ${target}.` };
    case "refused": {
      const reason = outcome.reason === "local_changes" ? "Local changes would be overwritten by the fast-forward."
        : outcome.reason === "not_fast_forward" ? "The branches cannot be fast-forwarded. Reconcile them in a terminal."
        : "The remote refused the push (for example, a protected branch or hook).";
      const detail = outcome.detail.trim();
      const usefulDetail = detail && !/^(?:aborting[.!]?|not possible to fast-forward, aborting[.!]?|failed to push some refs\b.*)$/i.test(detail);
      return { tone: "error", message: `${action} was refused: ${target}. ${reason}${usefulDetail ? ` ${detail}` : ""}${request.action === "pull" ? " Your branch and files are unchanged." : ""}` };
    }
  }
}

/** Writes outlive navigation; root-keyed reservations and tokens never transfer a result to a retargeted checkout. */
export function useSpaceGitActions(client: CockpitClient, sessionId: string | null, statuses: ReadonlyMap<string, SpaceGitStatus>, refresh: () => void, onResult: (spaceId: string, note: GitNote) => void): SpaceGitActions {
  const [entries, setEntries] = useState<ReadonlyMap<string, GitActionState>>(new Map());
  const active = useRef(new Map<string, GitActionState>());
  const nextToken = useRef(0);
  const latest = useRef({ sessionId, statuses, refresh, onResult });
  latest.current = { sessionId, statuses, refresh, onResult };
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const keyFor = (session: string, root: string) => `${session}\u0000${root}`;
  const publish = () => { if (mounted.current) setEntries(new Map(active.current)); };
  const dismiss = (root: string) => {
    if (!sessionId) return;
    const key = keyFor(sessionId, root);
    const entry = active.current.get(key);
    if (entry && !entry.pending) { active.current.delete(key); publish(); }
  };
  const run = (request: SpaceGitActionRequest) => {
    const session = latest.current.sessionId;
    if (!session || !gitTargetMatches(latest.current.statuses.get(request.space_id), request)) return;
    const key = keyFor(session, request.expected_root);
    if (active.current.get(key)?.pending) return;
    const token = ++nextToken.current;
    active.current.set(key, { token, request, pending: true });
    publish();
    const finish = (note: GitNote) => {
      if (active.current.get(key)?.token !== token) return;
      active.current.set(key, { token, request, pending: false, note });
      publish();
      if (mounted.current && latest.current.sessionId === session) {
        latest.current.refresh();
        if (gitTargetMatches(latest.current.statuses.get(request.space_id), request)) latest.current.onResult(request.space_id, note);
      }
      if (note.tone === "success") setTimeout(() => {
        if (active.current.get(key)?.token === token) { active.current.delete(key); publish(); }
      }, 6000);
    };
    void client.spaceGitAction(session, request).then(response => finish(gitOutcomeNote(response.outcome, request)), error => finish(gitFailureNote(error, request)));
  };
  const forStatus = (status: SpaceGitStatus | undefined) => {
    const checkout = status?.checkout;
    if (!sessionId || !checkout || checkout.state === "unavailable") return undefined;
    const entry = entries.get(keyFor(sessionId, checkout.root));
    return entry && gitTargetMatches(status, entry.request) ? entry : entry?.pending ? { ...entry, note: undefined } : undefined;
  };
  return { run, dismiss, forStatus };
}
