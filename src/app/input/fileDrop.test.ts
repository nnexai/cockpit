// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { installFileDropNavigationGuard, isExternalFileDrag, isTerminalFileDropTarget } from "./fileDrop";

function transfer(types: string[], files: File[] = [], items: Pick<DataTransferItem, "kind">[] = []): DataTransfer {
  return { types, files, items, dropEffect: "move" } as unknown as DataTransfer;
}

function dispatchDrag(target: Element, type: "dragover" | "drop", dataTransfer: DataTransfer | null): DragEvent {
  const event = new Event(type, { bubbles: true, cancelable: true }) as DragEvent;
  Object.defineProperty(event, "dataTransfer", { value: dataTransfer });
  target.dispatchEvent(event);
  return event;
}

function mount(html: string, selector: string): Element {
  document.body.innerHTML = html;
  return document.querySelector(selector)!;
}

let uninstall: () => void;
beforeEach(() => { uninstall = installFileDropNavigationGuard(); });
afterEach(() => {
  uninstall();
  document.body.replaceChildren();
});

describe("external file navigation guard", () => {
  it.each(["dragover", "drop"] as const)("prevents %s file navigation outside terminals without swallowing the event", (type) => {
    const target = mount('<aside><span id="target"></span></aside>', "#target");
    const data = transfer(["Files"]);
    const bubble = vi.fn((event: Event) => expect(event.defaultPrevented).toBe(true));
    target.addEventListener(type, bubble);

    const event = dispatchDrag(target, type, data);

    expect(event.defaultPrevented).toBe(true);
    expect(event.cancelBubble).toBe(false);
    expect(data.dropEffect).toBe("none");
    expect(bubble).toHaveBeenCalledOnce();
  });

  it.each(["dragover", "drop"] as const)("leaves internal text/custom %s payloads untouched", (type) => {
    const target = mount('<div class="terminal-host"><span id="target"></span></div>', "#target");
    for (const types of [["text/plain"], ["application/x-cockpit-tab", "text/plain"], ["application/x-cockpit-space"], []]) {
      const data = transfer(types);
      const bubble = vi.fn();
      target.addEventListener(type, bubble, { once: true });

      const event = dispatchDrag(target, type, data);

      expect(event.defaultPrevented).toBe(false);
      expect(data.dropEffect).toBe("move");
      expect(bubble).toHaveBeenCalledOnce();
    }
  });

  it("ignores drag events without a data transfer", () => {
    const target = mount('<div id="target"></div>', "#target");
    expect(dispatchDrag(target, "drop", null).defaultPrevented).toBe(false);
  });

  it("recognizes protected Files types before file contents become readable", () => {
    expect(isExternalFileDrag({ dataTransfer: transfer(["Files"]) })).toBe(true);
    expect(isExternalFileDrag({ dataTransfer: transfer([], [new File(["a"], "a.txt")]) })).toBe(true);
    expect(isExternalFileDrag({ dataTransfer: transfer([], [], [{ kind: "file" }]) })).toBe(true);
    expect(isExternalFileDrag({ dataTransfer: transfer(["text/plain"], [], [{ kind: "string" }]) })).toBe(false);
  });

  it("advertises copy for a terminal under the pointer and preserves its drop handler", () => {
    const target = mount('<div class="terminal-host"><div class="xterm"><span id="target"></span></div></div>', "#target");
    const data = transfer(["Files", "text/plain"]);
    const terminalDrop = vi.fn();
    target.closest(".terminal-host")!.addEventListener("drop", terminalDrop);

    const event = dispatchDrag(target, "drop", data);

    expect(event.defaultPrevented).toBe(true);
    expect(data.dropEffect).toBe("copy");
    expect(terminalDrop).toHaveBeenCalledOnce();
  });

  it("does not route an outside drop to the selected terminal", () => {
    const target = mount('<div class="terminal-host selected"></div><aside id="target"></aside>', "#target");
    const selectedDrop = vi.fn();
    document.querySelector(".terminal-host")!.addEventListener("drop", selectedDrop);
    const data = transfer(["Files"]);

    const event = dispatchDrag(target, "drop", data);

    expect(event.defaultPrevented).toBe(true);
    expect(data.dropEffect).toBe("none");
    expect(selectedDrop).not.toHaveBeenCalled();
  });

  it.each([
    '<div class="terminal-host" data-closed="true"><span id="target"></span></div>',
    '<section inert><div class="terminal-host"><span id="target"></span></div></section>',
    '<div class="terminal-host" inert><span id="target"></span></div>',
    '<dialog open></dialog><div class="terminal-host"><span id="target"></span></div>',
    '<div role="dialog" aria-modal="true"></div><div class="terminal-host"><span id="target"></span></div>',
  ])("prevents navigation but rejects a closed, inert, or modal-blocked terminal (%s)", (html) => {
    const target = mount(html, "#target");
    const data = transfer(["Files"]);

    expect(isTerminalFileDropTarget(target)).toBe(false);
    expect(dispatchDrag(target, "dragover", data).defaultPrevented).toBe(true);
    expect(data.dropEffect).toBe("none");
  });

  it("allows the terminal inside the active server dialog", () => {
    const target = mount('<div role="dialog" aria-modal="true"><div class="terminal-host"><span id="target"></span></div></div>', "#target");
    const data = transfer(["Files"]);

    expect(isTerminalFileDropTarget(target)).toBe(true);
    expect(dispatchDrag(target, "drop", data).defaultPrevented).toBe(true);
    expect(data.dropEffect).toBe("copy");
  });

  it("does not let a terminal in one dialog accept a drop behind another active modal", () => {
    const target = mount('<dialog open><div class="terminal-host" id="target"></div></dialog><div role="dialog" aria-modal="true"></div>', "#target");
    expect(isTerminalFileDropTarget(target)).toBe(false);
  });

  it("removes both capture listeners on cleanup", () => {
    const target = mount('<div id="target"></div>', "#target");
    uninstall();
    for (const type of ["dragover", "drop"] as const) {
      const data = transfer(["Files"]);
      expect(dispatchDrag(target, type, data).defaultPrevented).toBe(false);
      expect(data.dropEffect).toBe("move");
    }
  });
});
