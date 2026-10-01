import { useEffect, useRef, useState } from "react";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { SpaceGitAction, SpaceGitActionOutcome, SpaceGitActionRequest, SpaceGitStatus } from "../../protocol/generated/v1";

export type GitProblem = { kind: "problem"; unknown: boolean; summary: string; target?: string; details: string[]; gitDetail?: string };
export type GitDone = { kind: "done"; noop: boolean; message: string };
export type GitResult = GitDone | GitProblem;
export type GitActionState = { token: number; request: SpaceGitActionRequest; pending: boolean; result?: GitResult };
export const GIT_DONE_HOLD_MS = 1000;
export const GIT_NOOP_HOLD_MS = 700;
export interface SpaceGitActions {
  run(request: SpaceGitActionRequest): void;
  dismiss(root: string): void;
  forStatus(status: SpaceGitStatus | undefined): GitActionState | undefined;
}

export function gitActionReason(status: SpaceGitStatus | undefined, action: SpaceGitAction, entry?: GitActionState, blocked?: string): string | undefined {
  if (blocked) return blocked;
  if (entry?.pending) return `${entry.request.action === "pull" ? "Pull" : "Push"} in progress`;
  if (entry?.result?.kind === "problem" && entry.result.unknown && gitTargetMatches(status, entry.request)) return "Result unknown: check this Space's terminal, then dismiss the notice";
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

export function gitReadProblem(status: SpaceGitStatus | undefined): string | undefined {
  const checkout = status?.checkout;
  if (checkout?.state === "unavailable") return checkout.message;
  if (checkout?.state === "branch" && checkout.upstream.state === "unavailable") return checkout.upstream.message;
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

export function gitFailureProblem(error: unknown, request: SpaceGitActionRequest): GitProblem {
  const code = error instanceof CockpitClientError ? error.operationCode : undefined;
  const notRun = code?.startsWith("invalid_") || ["space_git_target_changed", "space_git_action_ineligible", "space_git_action_in_progress", "space_git_not_run"].includes(code ?? "");
  const action = request.action === "push" ? "Push" : "Pull";
  const target = `${request.expected_branch} ${request.action === "push" ? "→" : "←"} ${request.expected_upstream}`;
  const detail = error instanceof Error ? error.message : String(error);
  return {
    kind: "problem", unknown: !notRun, summary: notRun ? `${action} was not run.` : `${action} result unknown.`, target,
    details: notRun ? [detail] : [
      `Cockpit did not get a result for this ${request.action}.`,
      request.action === "push" ? "It may or may not have reached the remote." : "The branch may or may not have moved.",
      "Check this Space's terminal before trying again.",
      "Cockpit will not retry it.",
    ],
  };
}

export function gitOutcomeResult(outcome: SpaceGitActionOutcome, request: SpaceGitActionRequest): GitResult {
  const target = `${request.expected_branch} ${request.action === "push" ? "→" : "←"} ${request.expected_upstream}`;
  const action = request.action === "push" ? "Push" : "Pull";
  switch (outcome.result) {
    case "updated": return { kind: "done", noop: false, message: `${request.action === "push" ? "Pushed" : "Fast-forwarded"}${outcome.commits === null ? "" : ` ${outcome.commits} commit${outcome.commits === 1 ? "" : "s"}`}: ${target}.` };
    case "up_to_date": return { kind: "done", noop: true, message: `Already up to date: ${target}.` };
    case "refused": {
      const reason = outcome.reason === "local_changes" ? "Local changes would be overwritten by the fast-forward."
        : outcome.reason === "not_fast_forward" ? "The branches cannot be fast-forwarded. Reconcile them in a terminal."
        : "The remote refused the push (for example, a protected branch or hook).";
      const detail = outcome.detail.trim();
      const usefulDetail = detail && !/^(?:aborting[.!]?|not possible to fast-forward, aborting[.!]?|failed to push some refs\b.*)$/i.test(detail);
      return {
        kind: "problem", unknown: false, target,
        summary: outcome.reason === "local_changes" ? `${action} refused: local changes.`
          : outcome.reason === "not_fast_forward" ? `${action} refused: not a fast-forward.` : "Push refused: remote rejected.",
        details: [reason, ...(request.action === "pull" ? ["Your branch and files are unchanged."] : []),
          ...(outcome.reason === "local_changes" ? ["Commit or stash in a terminal, then pull again."] : [])],
        gitDetail: usefulDetail ? detail : undefined,
      };
    }
  }
}

/** Writes outlive navigation; root-keyed reservations and tokens never transfer a result to a retargeted checkout. */
export function useSpaceGitActions(client: CockpitClient, sessionId: string | null, statuses: ReadonlyMap<string, SpaceGitStatus>, refresh: () => void, onProblem: (spaceId: string, problem: GitProblem) => void): SpaceGitActions {
  const [entries, setEntries] = useState<ReadonlyMap<string, GitActionState>>(new Map());
  const active = useRef(new Map<string, GitActionState>());
  const nextToken = useRef(0);
  const latest = useRef({ sessionId, statuses, refresh, onProblem });
  latest.current = { sessionId, statuses, refresh, onProblem };
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
    const existing = active.current.get(key);
    if (existing?.pending || (existing?.result?.kind === "problem" && existing.result.unknown
      && existing.request.expected_branch === request.expected_branch && existing.request.expected_upstream === request.expected_upstream)) return;
    const token = ++nextToken.current;
    active.current.set(key, { token, request, pending: true });
    publish();
    const finish = (result: GitResult) => {
      if (active.current.get(key)?.token !== token) return;
      active.current.set(key, { token, request, pending: false, result });
      publish();
      if (mounted.current && latest.current.sessionId === session) {
        latest.current.refresh();
        if (result.kind === "problem" && gitTargetMatches(latest.current.statuses.get(request.space_id), request)) latest.current.onProblem(request.space_id, result);
      }
      if (result.kind === "done") setTimeout(() => {
        if (active.current.get(key)?.token === token) { active.current.delete(key); publish(); }
      }, result.noop ? GIT_NOOP_HOLD_MS : GIT_DONE_HOLD_MS);
    };
    void client.spaceGitAction(session, request).then(response => finish(gitOutcomeResult(response.outcome, request)), error => finish(gitFailureProblem(error, request)));
  };
  const forStatus = (status: SpaceGitStatus | undefined) => {
    const checkout = status?.checkout;
    if (!sessionId || !checkout || checkout.state === "unavailable") return undefined;
    const entry = entries.get(keyFor(sessionId, checkout.root));
    return entry && gitTargetMatches(status, entry.request) ? entry : entry?.pending ? { ...entry, result: undefined } : undefined;
  };
  return { run, dismiss, forStatus };
}
