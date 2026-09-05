export type PrefixCommand =
  | "help" | "new-space" | "rename-space" | "close-space"
  | "new-tab" | "rename-tab" | "previous-tab" | "next-tab" | "close-tab"
  | "rename-pane" | "split-right" | "split-down" | "close-pane" | "zoom-pane" | "resize";

export function prefixCommandForKey(key: string, shiftKey: boolean): PrefixCommand | null {
  if (key === "?") return "help";
  if (shiftKey && key.toLowerCase() === "n") return "new-space";
  if (shiftKey && key.toLowerCase() === "w") return "rename-space";
  if (shiftKey && key.toLowerCase() === "d") return "close-space";
  if (!shiftKey && key === "c") return "new-tab";
  if (shiftKey && key.toLowerCase() === "t") return "rename-tab";
  if (!shiftKey && key === "p") return "previous-tab";
  if (!shiftKey && key === "n") return "next-tab";
  if (shiftKey && key.toLowerCase() === "x") return "close-tab";
  if (shiftKey && key.toLowerCase() === "p") return "rename-pane";
  if (!shiftKey && key === "v") return "split-right";
  if (!shiftKey && key === "-") return "split-down";
  if (!shiftKey && key === "x") return "close-pane";
  if (!shiftKey && key === "z") return "zoom-pane";
  if (!shiftKey && key === "r") return "resize";
  return null;
}

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
};

export function routeWorkbenchKeydown(event: WorkbenchKeyEvent, routing: WorkbenchKeyRouting): void {
  if (event.isComposing) return;
  const target = typeof HTMLElement !== "undefined" && event.target instanceof HTMLElement ? event.target : null;
  const prefixSafe = !editableTarget(target) || Boolean(target?.closest(".terminal-host"));
  if (event.key === "Escape" && routing.prefixActive) {
    routing.setPrefixActive(false);
    if (!routing.modalOpen && prefixSafe) {
      event.preventDefault();
      event.stopPropagation();
    }
    return;
  }
  if (routing.modalOpen) return;
  if (!routing.prefixActive) {
    if (event.ctrlKey && !event.shiftKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "b" && prefixSafe) {
      event.preventDefault();
      event.stopPropagation();
      routing.setPrefixActive(true);
      return;
    }
    if (event.key === "?" && !event.ctrlKey && !event.altKey && !event.metaKey && !editableTarget(target)) {
      event.preventDefault();
      routing.setCommandsOpen(true);
    }
    return;
  }
  if (!prefixSafe) return;
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
}
