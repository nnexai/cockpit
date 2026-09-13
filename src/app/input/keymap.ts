export type PrefixCommand =
  | "help" | "new-space" | "rename-space" | "close-space"
  | "new-tab" | "rename-tab" | "previous-tab" | "next-tab" | "close-tab"
  | "rename-pane" | "split-right" | "split-down" | "close-pane" | "zoom-pane" | "resize"
  | "previous-pane" | "next-pane" | "focus-left" | "focus-right" | "focus-up" | "focus-down"
  | "open-file-picker" | "focus-file-tree" | "focus-file-content"
  | `select-tab-${1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9}`;

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
  if (!shiftKey && key === "o") return "next-pane";
  if (shiftKey && key.toLowerCase() === "o") return "previous-pane";
  if (!shiftKey && key === "h") return "focus-left";
  if (!shiftKey && key === "j") return "focus-down";
  if (!shiftKey && key === "k") return "focus-up";
  if (!shiftKey && key === "l") return "focus-right";
  if (!shiftKey && key === "f") return "open-file-picker";
  if (!shiftKey && key === "[") return "focus-file-tree";
  if (!shiftKey && key === "]") return "focus-file-content";
  if (!shiftKey && /^[1-9]$/.test(key)) return `select-tab-${key}` as PrefixCommand;
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

function modifierOnlyKey(key: string): boolean {
  return key === "Shift" || key === "Control" || key === "Alt" || key === "Meta";
}

export function routeWorkbenchKeydown(event: WorkbenchKeyEvent, routing: WorkbenchKeyRouting): void {
  if (event.isComposing) return;
  const target = typeof HTMLElement !== "undefined" && event.target instanceof HTMLElement ? event.target : null;
  const modalOpen = routing.modalOpen || Boolean(target?.closest("dialog[open]"));
  const browserFocused = Boolean(target?.closest(".browser-pane"));
  const remoteBrowserInput = Boolean(target?.closest("[data-browser-input]"));
  const prefixSafe = remoteBrowserInput || (!browserFocused && (!editableTarget(target) || Boolean(target?.closest(".terminal-host"))));
  if (event.key === "Escape" && routing.prefixActive) {
    routing.setPrefixActive(false);
    if (!modalOpen && prefixSafe) {
      event.preventDefault();
      event.stopPropagation();
    }
    return;
  }
  if (modalOpen) return;
  if (!routing.prefixActive) {
    if (event.ctrlKey && !event.shiftKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "b" && prefixSafe) {
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
