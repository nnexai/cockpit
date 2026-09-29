import { type PrefixCommand, prefixCommandForKey, unboundPrefixMessage } from "./shortcuts";

function editableTarget(target: EventTarget | null): boolean {
  return target !== null && target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
}

export type WorkbenchKeyEvent = Pick<KeyboardEvent, "key" | "shiftKey" | "ctrlKey" | "altKey" | "metaKey" | "target" | "isComposing" | "preventDefault" | "stopPropagation">;
export type WorkbenchKeyRouting = {
  modalOpen: boolean;
  prefixActive: boolean;
  runCommand: (command: PrefixCommand) => void;
  setPrefixActive: (active: boolean) => void;
  setCommandsOpen: (open: boolean) => void;
  /** A key that follows the prefix but has no binding. The key is swallowed; say so. */
  onUnboundPrefixKey?: (message: string) => void;
};

function modifierOnlyKey(key: string): boolean {
  return key === "Shift" || key === "Control" || key === "Alt" || key === "Meta";
}

/**
 * Modal dialogs the router cannot see in React state: native `<dialog>` and
 * Library-internal `role="dialog" aria-modal` sections. The narrow sidebar
 * drawer is such a dialog too, but the prefix must keep working inside it.
 */
const MODAL_DIALOG = 'dialog[open], [role="dialog"][aria-modal="true"]:not(#cockpit-sidebar)';

/**
 * The window-level (capture) router for the `Ctrl+B` prefix. Order, per the
 * keyboard design: composing → (reserved: Herdr magic escape) → armed `Esc` →
 * modal → arm / bare `?` → armed key lookup. With a terminal or the inline
 * browser surface focused it consumes `Ctrl+B` and the one key after it, nothing
 * else; plain `Tab` and `Shift+Tab` only mean anything as that next key.
 */
export function routeWorkbenchKeydown(event: WorkbenchKeyEvent, routing: WorkbenchKeyRouting): void {
  if (event.isComposing) return;
  const target = typeof HTMLElement !== "undefined" && event.target instanceof HTMLElement ? event.target : null;
  const modalOpen = routing.modalOpen || Boolean(target?.closest(MODAL_DIALOG));
  const browserFocused = Boolean(target?.closest(".browser-pane"));
  // The remote page's surface arms the prefix like a terminal does; the URL field, note editor and other browser chrome do not.
  const prefixSafe = Boolean(target?.closest(".terminal-host, .browser-surface")) || (!browserFocused && !editableTarget(target));
  if (event.key === "Escape" && routing.prefixActive) {
    routing.setPrefixActive(false);
    if (!modalOpen && prefixSafe) {
      event.preventDefault();
      event.stopPropagation();
    }
    return;
  }
  if (modalOpen) return;
  const plainCtrlB = event.ctrlKey && !event.shiftKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "b";
  if (!routing.prefixActive) {
    if (plainCtrlB && prefixSafe) {
      event.preventDefault();
      event.stopPropagation();
      routing.setPrefixActive(true);
      return;
    }
    if (event.key === "?" && !event.ctrlKey && !event.altKey && !event.metaKey && !editableTarget(target) && !browserFocused) {
      event.preventDefault();
      routing.setCommandsOpen(true);
    }
    return;
  }
  if (!prefixSafe) return;
  if (modifierOnlyKey(event.key)) return;
  // `Ctrl+B Ctrl+B`: disarm and let this key through, so the terminal or page receives a literal Ctrl+B.
  if (plainCtrlB) {
    routing.setPrefixActive(false);
    return;
  }
  if (event.ctrlKey || event.altKey || event.metaKey) {
    routing.setPrefixActive(false);
    return;
  }
  const command = prefixCommandForKey(event.key, event.shiftKey);
  if (command) {
    event.preventDefault();
    event.stopPropagation();
    routing.setPrefixActive(false);
    routing.runCommand(command);
    return;
  }
  event.preventDefault();
  routing.setPrefixActive(false);
  routing.onUnboundPrefixKey?.(unboundPrefixMessage(event.key, event.shiftKey));
}
