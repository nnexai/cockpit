import { useEffect, useRef, type KeyboardEvent } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextRoot, ContextSnapshotResponse } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { SnapshotImport } from "./SnapshotImport";
import { SourceImport } from "./SourceImport";
export function ContextResources({
  client,
  sessionId,
  paneId,
  bindingId,
  root,
  onChanged,
  onImported,
  onClose,
}: {
  client: CockpitClient;
  sessionId: string;
  paneId: string;
  bindingId: string;
  root: ContextRoot;
  onChanged: (paths: string[]) => void;
  onImported: (result: ContextSnapshotResponse) => void;
  onClose: () => void;
}) {
  const dialogRef = useRef<HTMLElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  useEffect(() => {
    restoreFocusRef.current = globalThis.document.activeElement instanceof HTMLElement ? globalThis.document.activeElement : null;
    const firstFocusable = [...(dialogRef.current?.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([disabled]), select:not([disabled]), summary") ?? [])].find(element => {
      const details = element.closest("details");
      return element.tagName === "SUMMARY" || !details || details.open;
    });
    firstFocusable?.focus();
    return () => restoreFocusRef.current?.focus({ preventScroll: true });
  }, [root.kind]);
  if (root.kind !== "companion") return null;
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = [...event.currentTarget.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([disabled]), select:not([disabled]), summary")].filter(element => {
      const details = element.closest("details");
      return element.tagName === "SUMMARY" || !details || details.open;
    });
    if (focusable.length === 0) return;
    const current = focusable.indexOf(globalThis.document.activeElement as HTMLElement);
    if (event.shiftKey && current <= 0) {
      event.preventDefault();
      focusable[focusable.length - 1]?.focus();
    } else if (!event.shiftKey && current === focusable.length - 1) {
      event.preventDefault();
      focusable[0]?.focus();
    }
  };
  return <section className="context-resources" role="dialog" aria-modal="true" aria-label="Context resources" ref={dialogRef} onKeyDown={onKeyDown}>
    <header><strong>Context resources</strong><button type="button" className="context-resources-close" aria-label="Close Context resources" title="Close Context resources" onClick={onClose}><UiIcon name="close" /></button></header>
    <div className="context-resources-body">
      <SourceImport client={client} sessionId={sessionId} paneId={paneId} bindingId={bindingId} rootId={root.root_id} onChanged={onChanged} />
      <SnapshotImport client={client} sessionId={sessionId} paneId={paneId} bindingId={bindingId} rootId={root.root_id} onImported={onImported} />
    </div>
  </section>;
}
