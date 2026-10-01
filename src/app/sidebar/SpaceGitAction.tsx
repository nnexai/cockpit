import { useLayoutEffect, useRef } from "react";
import type { SpaceGitAction as GitAction, SpaceGitStatus } from "../../protocol/generated/v1";
import { aheadBehindLabel } from "../session/spaceGitStatus";
import { gitActionReason, gitReadProblem, type GitActionState, type GitProblem } from "../session/spaceGitActions";
import { UiIcon } from "../UiIcon";

export function SpaceGitAction({ status, child, entry, blocked, statusError, onAction, onReadProblem }: {
  status: SpaceGitStatus | undefined; child: boolean; entry?: GitActionState; blocked?: string; statusError?: string;
  onAction(action: GitAction): void; onReadProblem?(): void;
}) {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const present = Boolean(status || statusError);
  useLayoutEffect(() => {
    const button = buttonRef.current;
    const select = button?.closest(".space-tree-row")?.querySelector<HTMLButtonElement>(".resource-select");
    return () => {
      if (button && document.activeElement === button) queueMicrotask(() => select?.isConnected && select.focus({ preventScroll: true }));
    };
  }, [present]);
  if (!status && !statusError) return null;
  const checkout = status?.checkout ?? { state: "unavailable" as const, root: null, message: statusError! };
  const upstream = checkout.state === "branch" ? checkout.upstream : undefined;
  const tracked = upstream?.state === "tracked" ? upstream : undefined;
  const action: GitAction = tracked && tracked.ahead > 0 && tracked.behind === 0 ? "push" : "pull";
  const stateReason = gitActionReason(status, action);
  const pending = entry?.pending;
  const result = entry?.result;
  const phase = pending ? "pending" : result?.kind === "done" ? "done" : "idle";
  const unknown = result?.kind === "problem" && result.unknown;
  const readProblem = Boolean(onReadProblem && gitReadProblem(status));
  const reason = gitActionReason(status, action, entry, blocked ?? (statusError ? `Git status unavailable: ${statusError}` : undefined));
  const quiet = !statusError && tracked?.ahead === 0 && tracked.behind === 0;
  const diverged = Boolean(tracked && tracked.ahead > 0 && tracked.behind > 0);
  const position = aheadBehindLabel(status);
  const target = checkout.state === "branch" && tracked ? `${checkout.branch} ${action === "push" ? "→" : "←"} ${tracked.name}` : "";
  const label = reason ?? (action === "push" ? `Push ${tracked?.ahead} commit${tracked?.ahead === 1 ? "" : "s"}: ${target}` : tracked?.behind ? `Pull ${tracked.behind} commit${tracked.behind === 1 ? "" : "s"}: ${target} (fast-forward only)` : `Pull: check ${tracked?.name} and fast-forward ${checkout.state === "branch" ? checkout.branch : ""} if it moved`);
  const symbol = statusError ? "ⓘ" : unknown ? "?" : checkout.state === "detached" ? (child ? "⦿" : "detached HEAD") : checkout.state === "unavailable" ? "ⓘ" : upstream?.state === "none" ? "⊘" : tracked ? position || "↓" : "ⓘ";
  return <button ref={buttonRef} type="button" tabIndex={-1} className={`space-git-action${quiet ? " is-quiet" : ""}${diverged || checkout.state === "detached" ? " is-warning" : ""}${stateReason || statusError ? " is-explanation" : ""}${unknown ? " is-unknown" : ""}${readProblem ? " is-problem" : ""}${phase === "done" && result?.kind === "done" && result.noop ? " is-noop" : ""}`} data-phase={phase} data-git-action={stateReason || statusError || unknown ? undefined : action}
    aria-label={label} aria-disabled={Boolean(reason) || undefined} aria-busy={pending || undefined}
    title={`${reason ?? label}\n${status?.source === "pane_folder" && checkout.state !== "unavailable" ? `${checkout.root}\n` : ""}Counts since last fetch; ${action === "push" ? "normal push, never forced" : "fast-forward only"}`}
    onClick={event => { event.stopPropagation(); if (blocked || statusError || pending || unknown) return; if (readProblem) onReadProblem!(); else if (!reason) onAction(action); }}>
    <span className="space-ahead-behind" aria-hidden="true">{symbol}</span>
    {pending ? <span className="space-git-spinner" aria-hidden="true" /> : phase === "done" ? <span className="space-git-check" aria-hidden="true"><UiIcon name="check" /></span> : null}
  </button>;
}

export function GitProblemNotice({ problem, child, section, onDismiss, onRetry }: {
  problem: GitProblem; child?: boolean; section?: boolean; onDismiss?(): void; onRetry?(): void;
}) {
  return <div className={`row-note git-problem${problem.unknown ? " is-unknown" : ""}${child ? " is-child" : ""}${section ? " is-section" : ""}`} role="alert" onKeyDown={event => {
    if (event.key === "Escape" && onDismiss) { event.preventDefault(); event.stopPropagation(); onDismiss(); }
  }}>
    <details open={problem.unknown}>
      <summary><span className="git-problem-icon" aria-hidden="true">{problem.unknown ? "?" : "!"}</span><span>{problem.summary}</span><UiIcon name="down" /></summary>
      <div className="git-problem-details">
        {problem.target ? <p className="git-problem-target">{problem.target}</p> : null}
        {problem.details.map(line => <p key={line}>{line}</p>)}
        {problem.gitDetail ? <pre>{problem.gitDetail}</pre> : null}
      </div>
    </details>
    <span className="git-note-actions">
      {onRetry ? <button type="button" onClick={onRetry}>Retry</button> : null}
      {onDismiss ? <button type="button" aria-label="Dismiss" title="Dismiss (Esc)" onClick={onDismiss}><UiIcon name="close" /></button> : null}
    </span>
  </div>;
}
