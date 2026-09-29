import { useEffect, useRef, type KeyboardEvent } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextRoot } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import type { LibrarySpace } from "../library/libraryState";
import { SpaceContextList } from "../library/SpaceContextList";
import type { SpaceListingState } from "../library/useLibraryOperation";

/**
 * The companion root's `Context resources` overlay (design §4.8): the Library
 * context held by this viewer's Space. Add uses the Library-first workflow.
 */
export function ContextResources({
  client,
  root,
  space,
  spaceListing,
  onAdd,
  onClose,
}: {
  client: CockpitClient;
  root: ContextRoot;
  /** The viewer's own Space; its listing is read only while Herdr is live. */
  space: LibrarySpace | null;
  spaceListing: SpaceListingState;
  onAdd: () => void;
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
      {space?.live ? <SpaceContextList client={client} space={space} state={spaceListing} onAdd={onAdd} /> : null}
      {space && !space.live ? <p className="context-resource-empty space-context-offline">Herdr isn't live, so {space.label}'s Library context can't be checked.</p> : null}
    </div>
  </section>;
}
