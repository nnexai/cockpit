import { type PrefixCommand, prefixCommandForKey, unboundPrefixMessage } from "./shortcuts";
import { matchingHerdrCommand, herdrBindingMatches, type HerdrBindingEvent, type HerdrBinding, type HerdrChord } from "./herdrBindings";
import type { HerdrCommand } from "../../protocol/generated/v1";

function editableTarget(target: EventTarget | null): boolean {
  return target !== null && target instanceof HTMLElement && (target.isContentEditable || Boolean(target.closest(".notes-editor")) || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
}

export type WorkbenchKeyEvent = Pick<KeyboardEvent, "key" | "shiftKey" | "ctrlKey" | "altKey" | "metaKey" | "target" | "isComposing" | "preventDefault" | "stopPropagation"> & Partial<Pick<KeyboardEvent, "code" | "repeat">>;
export type WorkbenchKeyRouting = {
  modalOpen: boolean;
  prefixActive: boolean;
  runCommand: (command: PrefixCommand) => void;
  setPrefixActive: (active: boolean) => void;
  setCommandsOpen: (open: boolean) => void;
  /** A key that follows the prefix but has no binding. The key is swallowed; say so. */
  onUnboundPrefixKey?: (message: string) => void;
  herdrBindings?: readonly HerdrBinding[];
  runHerdrCommand?: (command: HerdrCommand) => void;
  serverModalOpen?: boolean;
  popupPending?: boolean;
  herdrPrefixes?: readonly HerdrChord[];
  prefixOrigin?: "cockpit" | "herdr";
  onPrefixArm?: (origin: "cockpit" | "herdr", label: string) => void;
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
  if (routing.serverModalOpen) return;
  if (routing.popupPending) { event.preventDefault(); event.stopPropagation(); return; }
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
  if (prefixSafe && !routing.prefixActive) {
    const custom = matchingHerdrCommand(routing.herdrBindings ?? [], event as HerdrBindingEvent, false);
    if (custom) {
      event.preventDefault();
      event.stopPropagation();
      if (!event.repeat) routing.runHerdrCommand?.(custom);
      return;
    }
  }
  const plainCtrlB = event.ctrlKey && !event.shiftKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "b";
  const serverPrefix = routing.herdrPrefixes?.find(prefix => herdrBindingMatches(prefix, event));
  const sharedPrefix = routing.herdrPrefixes?.some(prefix => herdrBindingMatches(prefix, { key: "b", ctrlKey: true, shiftKey: false, altKey: false, metaKey: false })) ?? false;
  if (!routing.prefixActive) {
    if ((plainCtrlB || serverPrefix) && prefixSafe) {
      event.preventDefault();
      event.stopPropagation();
      routing.setPrefixActive(true);
      routing.onPrefixArm?.(serverPrefix ? "herdr" : "cockpit", serverPrefix?.display ?? "Ctrl+B");
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
  if ((routing.prefixOrigin === "herdr" && serverPrefix) || plainCtrlB) {
    routing.setPrefixActive(false);
    return;
  }
  const custom = routing.prefixOrigin === "herdr" || sharedPrefix ? matchingHerdrCommand(routing.herdrBindings ?? [], event as HerdrBindingEvent, true) : null;
  if (custom) {
    event.preventDefault();
    event.stopPropagation();
    routing.setPrefixActive(false);
    if (!event.repeat) routing.runHerdrCommand?.(custom);
    return;
  }
  if (event.ctrlKey || event.altKey || event.metaKey) {
    routing.setPrefixActive(false);
    return;
  }
  const command = routing.prefixOrigin === "herdr" && !sharedPrefix ? null : prefixCommandForKey(event.key, event.shiftKey);
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
