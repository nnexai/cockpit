// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { SHORTCUTS, SHORTCUT_DOCS_BEGIN, SHORTCUT_DOCS_END, formatShortcut, prefixCommandForKey, renderShortcutDocs, returnFocusFromSidebar, focusSidebarList, registerSidebarFocus, viewerShortcutAction, type PrefixCommand } from "./shortcuts";

function chord(overrides: Partial<{ key: string; code: string; shiftKey: boolean; ctrlKey: boolean; altKey: boolean; metaKey: boolean; target: EventTarget | null }>) {
  return { key: "", code: "", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, target: null, ...overrides };
}

describe("shortcut registry", () => {
  it("resolves every prefix entry from its own key and gives no two entries the same key", () => {
    const seen = new Set<string>();
    for (const entry of SHORTCUTS.filter((candidate) => candidate.prefix)) {
      const { key, shift } = entry.prefix!;
      const shiftKey = shift === true;
      expect(prefixCommandForKey(shiftKey && key.length === 1 ? key.toUpperCase() : key, shiftKey), entry.id).toBe(entry.id);
      const identity = `${key}:${shift}`;
      expect(seen.has(identity), `duplicate ${identity}`).toBe(false);
      seen.add(identity);
    }
  });

  it("keeps Herdr's meaning for every key Herdr binds by default and leaves its other keys unbound", () => {
    const herdr: Array<[string, boolean, PrefixCommand]> = [
      ["?", false, "help"], ["n", true, "new-space"], ["w", true, "rename-space"], ["d", true, "close-space"], ["c", false, "new-tab"], ["t", true, "rename-tab"],
      ["p", false, "previous-tab"], ["n", false, "next-tab"], ["x", true, "close-tab"], ["3", false, "select-tab-3"], ["p", true, "rename-pane"],
      ["v", false, "split-right"], ["-", false, "split-down"], ["x", false, "close-pane"], ["z", false, "zoom-pane"], ["r", false, "resize"],
      ["h", false, "focus-left"], ["j", false, "focus-down"], ["k", false, "focus-up"], ["l", false, "focus-right"],
      ["h", true, "swap-left"], ["j", true, "swap-down"], ["k", true, "swap-up"], ["l", true, "swap-right"],
      ["Tab", false, "next-pane"], ["Tab", true, "previous-pane"], ["b", false, "toggle-sidebar"], ["w", false, "focus-spaces"], ["g", false, "switch-session"],
    ];
    for (const [key, shift, command] of herdr) expect(prefixCommandForKey(shift && key.length === 1 ? key.toUpperCase() : key, shift), `${shift ? "Shift+" : ""}${key}`).toBe(command);
    // Herdr-only actions Cockpit deliberately does not implement, and the pane-cycling key it moved to Tab.
    for (const [key, shift] of [["s", false], ["q", false], ["e", false], ["[", false], ["]", false], ["o", false], ["O", true], ["G", true], ["R", true]] as const) {
      expect(prefixCommandForKey(key, shift), `${shift ? "Shift+" : ""}${key}`).toBeNull();
    }
  });

  it("formats keys for each platform and only lists a sequence's prefix as Ctrl+B", () => {
    expect(formatShortcut("new-space", "other")).toBe("Ctrl+B Shift+N");
    expect(formatShortcut("previous-pane", "mac")).toBe("Ctrl+B Shift+Tab");
    expect(formatShortcut("open-file-picker", "other")).toBe("Ctrl+B f or Ctrl+P or /");
    expect(formatShortcut("open-file-picker", "mac", "chord")).toBe("Cmd+P or /");
    expect(formatShortcut("toggle-preview", "mac")).toBe("Option+M");
    expect(formatShortcut("terminal-copy", "mac")).toBe("Cmd+Shift+C or Cmd+C");
    expect(formatShortcut("terminal-copy", "other")).toBe("Ctrl+Shift+C");
  });

  it("keeps docs/keyboard-shortcuts.md in step with the registry", () => {
    const docs = readFileSync("docs/keyboard-shortcuts.md", "utf8");
    const start = docs.indexOf(SHORTCUT_DOCS_BEGIN);
    const end = docs.indexOf(SHORTCUT_DOCS_END);
    expect(start).toBeGreaterThanOrEqual(0);
    expect(docs.slice(start + SHORTCUT_DOCS_BEGIN.length, end).trim()).toBe(renderShortcutDocs().trim());
  });
});

describe("viewer shortcuts", () => {
  it("matches Alt chords by physical key so macOS Option works", () => {
    expect(viewerShortcutAction(chord({ altKey: true, key: "¡", code: "Digit1" }))).toBe("focus-file-tree");
    expect(viewerShortcutAction(chord({ altKey: true, key: "™", code: "Digit2" }))).toBe("focus-file-content");
    expect(viewerShortcutAction(chord({ altKey: true, key: "µ", code: "KeyM" }))).toBe("toggle-preview");
    expect(viewerShortcutAction(chord({ altKey: true, key: "Ω", code: "KeyZ" }))).toBe("toggle-wrap");
  });

  it("opens the picker with Mod+P or / and reloads with Mod+R but not Ctrl+Shift+R", () => {
    expect(viewerShortcutAction(chord({ ctrlKey: true, key: "p", code: "KeyP" }))).toBe("open-file-picker");
    expect(viewerShortcutAction(chord({ metaKey: true, key: "p", code: "KeyP" }))).toBe("open-file-picker");
    expect(viewerShortcutAction(chord({ key: "/", code: "Slash" }))).toBe("open-file-picker");
    expect(viewerShortcutAction(chord({ ctrlKey: true, key: "r", code: "KeyR" }))).toBe("reload-listing");
    expect(viewerShortcutAction(chord({ ctrlKey: true, shiftKey: true, key: "R", code: "KeyR" }))).toBeNull();
    expect(viewerShortcutAction(chord({ key: "m", code: "KeyM" }))).toBeNull();
    expect(viewerShortcutAction(chord({ altKey: true, ctrlKey: true, key: "1", code: "Digit1" }))).toBeNull();
  });

  it("leaves keys inside a modal dialog to the dialog", () => {
    document.body.innerHTML = '<section role="dialog" aria-modal="true"><button>ok</button></section>';
    expect(viewerShortcutAction(chord({ altKey: true, code: "Digit1", target: document.querySelector("button") }))).toBeNull();
    document.body.replaceChildren();
  });
});

describe("sidebar focus registration", () => {
  it("focuses through the registered handler and returns to where focus was", () => {
    document.body.innerHTML = '<button id="origin">pane</button><aside id="cockpit-sidebar"><button id="row">row</button></aside><button class="tab-button" aria-selected="true">tab</button>';
    const origin = document.getElementById("origin")!;
    origin.focus();
    const unregister = registerSidebarFocus({ spaces: () => document.getElementById("row")!.focus(), agents: () => undefined });
    expect(focusSidebarList("spaces")).toBe(true);
    expect(document.activeElement?.id).toBe("row");
    expect(returnFocusFromSidebar()).toBe(true);
    expect(document.activeElement).toBe(origin);
    unregister();
    expect(focusSidebarList("spaces")).toBe(false);
    document.body.replaceChildren();
  });
});
