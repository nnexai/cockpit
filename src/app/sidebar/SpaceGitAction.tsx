import { useLayoutEffect, useRef } from "react";
import type { SpaceGitAction as GitAction, SpaceGitStatus } from "../../protocol/generated/v1";
import { aheadBehindLabel } from "../session/spaceGitStatus";
import { gitActionReason, type GitActionState } from "../session/spaceGitActions";

export function SpaceGitAction({ status, child, entry, blocked, onAction, onExplain }: {
  status: SpaceGitStatus | undefined; child: boolean; entry?: GitActionState; blocked?: string;
  onAction(action: GitAction): void; onExplain(message: string): void;
}) {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const present = Boolean(status);
  useLayoutEffect(() => {
    const button = buttonRef.current;
    const select = button?.closest(".space-tree-row")?.querySelector<HTMLButtonElement>(".resource-select");
    return () => {
      if (button && document.activeElement === button) queueMicrotask(() => select?.isConnected && select.focus({ preventScroll: true }));
    };
  }, [present]);
  if (!status) return null;
  const checkout = status.checkout;
  const upstream = checkout.state === "branch" ? checkout.upstream : undefined;
  const tracked = upstream?.state === "tracked" ? upstream : undefined;
  const action: GitAction = tracked && tracked.ahead > 0 && tracked.behind === 0 ? "push" : "pull";
  const stateReason = gitActionReason(status, action);
  const pending = entry?.pending;
  const reason = gitActionReason(status, action, pending ? entry.request.action : undefined, blocked);
  const quiet = tracked?.ahead === 0 && tracked.behind === 0;
  const diverged = Boolean(tracked && tracked.ahead > 0 && tracked.behind > 0);
  const position = aheadBehindLabel(status);
  const target = checkout.state === "branch" && tracked ? `${checkout.branch} ${action === "push" ? "→" : "←"} ${tracked.name}` : "";
  const label = stateReason ?? (action === "push" ? `Push ${tracked?.ahead} commit${tracked?.ahead === 1 ? "" : "s"}: ${target}` : tracked?.behind ? `Pull ${tracked.behind} commit${tracked.behind === 1 ? "" : "s"}: ${target} (fast-forward only)` : `Pull: check ${tracked?.name} and fast-forward ${checkout.state === "branch" ? checkout.branch : ""} if it moved`);
  const symbol = checkout.state === "detached" ? (child ? "⦿" : "detached HEAD") : checkout.state === "unavailable" ? (child ? "ⓘ" : "ⓘ status unavailable") : upstream?.state === "none" ? "⊘" : tracked ? position || "↓" : "ⓘ";
  return <button ref={buttonRef} type="button" tabIndex={-1} className={`space-git-action${quiet ? " is-quiet" : ""}${diverged || checkout.state === "detached" ? " is-warning" : ""}${stateReason ? " is-explanation" : ""}${pending ? " is-git-pending" : ""}`} data-git-action={stateReason ? undefined : action}
    aria-label={label} aria-disabled={Boolean(reason) || undefined} aria-busy={pending || undefined}
    title={`${reason ?? label}\n${status.source === "pane_folder" && checkout.state !== "unavailable" ? `${checkout.root}\n` : ""}Counts since last fetch; ${action === "push" ? "normal push, never forced" : "fast-forward only"}`}
    onClick={event => { event.stopPropagation(); if (blocked || pending) return; if (stateReason) onExplain(stateReason); else onAction(action); }}>
    <span className="space-ahead-behind" aria-hidden="true">{symbol}</span>{pending ? <span className="space-git-spinner" aria-hidden="true" /> : null}
  </button>;
}
