import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { LibraryFollowSummary } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { ErrorSlot } from "../ErrorSlot";
import { errorText, followCountText, followTitle } from "./libraryState";
import "../projects/setup.css";
import "../projects/taskSetup.css";
import "./library.css";

const FOCUSABLE = "button:not(:disabled), input:not(:disabled), select:not(:disabled), a[href], summary, [tabindex]:not([tabindex='-1'])";

/**
 * Escape closes, Tab stays inside, and focus returns to the opener on close.
 * A dialog portalled out of another trap (a confirmation over `Context
 * resources`) still bubbles through it in React, so neither key goes further.
 */
export function trapDialogKeys(event: KeyboardEvent<HTMLElement>, onClose: () => void): void {
  if (event.nativeEvent.isComposing) return;
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    onClose();
    return;
  }
  if (event.key !== "Tab") return;
  event.stopPropagation();
  const focusable = [...event.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE)];
  if (focusable.length === 0) return;
  const current = focusable.indexOf(document.activeElement as HTMLElement);
  if (event.shiftKey && current <= 0) { event.preventDefault(); focusable.at(-1)?.focus(); }
  else if (!event.shiftKey && current === focusable.length - 1) { event.preventDefault(); focusable[0]?.focus(); }
}

/**
 * Remembers the opener when mounted and focuses it again when unmounted. When the opener is gone by then (the
 * `Provider token…` button disappears once a token is stored), focus goes to the nearest of its former ancestors
 * that is still mounted, so the next Tab and Escape stay in the same part of the page instead of the page body.
 */
export function useRestoreFocus(): void {
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null;
    const ancestors: HTMLElement[] = [];
    for (let element = opener?.parentElement; element && element !== document.body; element = element.parentElement) ancestors.push(element);
    return () => {
      if (opener?.isConnected) { opener.focus({ preventScroll: true }); return; }
      const container = ancestors.find((element) => element.isConnected);
      if (!container) return;
      // A container takes focus only while it holds it, and without a ring: it is a landing place, not a control.
      if (!container.hasAttribute("tabindex")) {
        const outline = container.style.outline;
        container.tabIndex = -1;
        container.style.outline = "none";
        container.addEventListener("blur", () => { container.removeAttribute("tabindex"); container.style.outline = outline; }, { once: true });
      }
      container.focus({ preventScroll: true });
    };
  }, []);
}

/**
 * Compact confirmation (design §4.10): focus starts on the safe button, the
 * destructive button names its effect, and a failure stays inline. An
 * `alternative` is a lesser action between them (`Stop following only`).
 */
export function LibraryConfirmDialog({ title, body, safeLabel, confirmLabel, destructive = false, alternative, onConfirm, onClose }: {
  title: string;
  body: ReactNode;
  safeLabel: string;
  confirmLabel: string;
  destructive?: boolean;
  alternative?: { label: string; onConfirm: () => Promise<void> };
  /** Resolves when the action was accepted; a rejection keeps the dialog open with the reason. */
  onConfirm: () => Promise<void>;
  onClose: () => void;
}) {
  const titleId = useId();
  const bodyId = useId();
  const safeRef = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useRestoreFocus();
  useEffect(() => { safeRef.current?.focus(); }, []);
  const confirm = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(errorText(cause, "The action could not be completed."));
      setBusy(false);
    }
  };
  // On the body: a Context viewer is a size container and would clip a fixed overlay to its pane.
  return createPortal(<div className="setup-overlay library-dialog-overlay" role="presentation">
    <section className="setup-dialog task-setup library-confirm" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={bodyId} onKeyDown={(event) => trapDialogKeys(event, onClose)}>
      <header className="task-setup-header"><h2 id={titleId}>{title}</h2><button type="button" className="task-setup-close" onClick={onClose} aria-label="Close" disabled={busy}><UiIcon name="close" /></button></header>
      <div className="task-setup-body">
        <div id={bodyId} className="library-confirm-body">{body}</div>
      </div>
      <footer className="task-setup-footer">
        <ErrorSlot placement="dialog" message={error ?? (busy ? "Working…" : null)} error={Boolean(error)} />
        <button ref={safeRef} type="button" onClick={onClose} disabled={busy}>{safeLabel}</button>
        {alternative ? <button type="button" onClick={() => void confirm(alternative.onConfirm)} disabled={busy}>{alternative.label}</button> : null}
        <button type="button" className={destructive ? "library-destructive" : "setup-primary"} onClick={() => void confirm(onConfirm)} disabled={busy}>{busy ? "Working…" : confirmLabel}</button>
      </footer>
    </section>
  </div>, document.body);
}

/** `stop_following` keeps the items; `follow` removes the ones no other reference holds with the follow (design §4.10). */
export type FollowRemoveMode = "stop_following" | "follow";

/**
 * Remove a followed space or query (design §4.10): `Cancel` is focused, `Stop
 * following only` keeps every item, and `Remove` deletes the items only this
 * follow holds from the Library. Items kept manually or held by another follow
 * stay, as do items selected by a Space.
 */
export function FollowRemoveDialog({ follow, remove, onClose }: {
  follow: LibraryFollowSummary;
  /** Resolves once the Library accepted the removal; a rejection stays inline. */
  remove: (mode: FollowRemoveMode) => Promise<void>;
  onClose: () => void;
}) {
  const query = follow.source.kind === "jira_query";
  const noun = query ? "query" : "space";
  return <LibraryConfirmDialog title={`Remove ${followTitle(follow)} from the Library?`} safeLabel="Cancel" confirmLabel={`Remove ${noun}`} destructive
    body={<p>{`Deletes items only this ${noun} holds (up to ${followCountText(follow)}) from the Library and stops following it. Items selected by a Space, kept in the Library, or held by another follow stay. ${query ? "Jira" : "Confluence"} isn't changed.`}</p>}
    alternative={{ label: "Stop following only", onConfirm: () => remove("stop_following") }}
    onConfirm={() => remove("follow")} onClose={onClose} />;
}

