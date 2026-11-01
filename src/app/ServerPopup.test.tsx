// @vitest-environment jsdom
import { act } from "react";
import type * as ReactTypes from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../client/CockpitClient";

const observedPopupStyle = vi.hoisted(() => vi.fn());
const popupReady = vi.hoisted(() => ({ current: false }));
vi.mock("./TerminalPane", async () => {
  const React = await vi.importActual<typeof ReactTypes>("react");
  return {
    TerminalPane: (props: { onStateChange?: (state: { ready: boolean; error: string | null }) => void }) => {
      React.useLayoutEffect(() => {
        const popup = document.querySelector<HTMLElement>(".server-popup");
        if (popup) observedPopupStyle({ left: popup.style.left, top: popup.style.top, width: popup.style.width, height: popup.style.height });
        if (popupReady.current) props.onStateChange?.({ ready: true, error: null });
      }, []);
      return React.createElement("textarea", { className: "xterm-helper-textarea" });
    },
  };
});
import { ServerPopup } from "./ServerPopup";

let workbench: HTMLElement | null = null;
let root: Root | null = null;

afterEach(async () => {
  if (root) await act(async () => root?.unmount());
  root = null;
  workbench?.remove();
  workbench = null;
  observedPopupStyle.mockReset();
  popupReady.current = false;
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("ServerPopup geometry", () => {
  it("fits the terminal to the work area on its first render", async () => {
    workbench = document.createElement("div");
    workbench.className = "workbench";
    const area = document.createElement("div");
    area.className = "workarea-content";
    area.getBoundingClientRect = () => new DOMRect(100, 80, 800, 600);
    const host = document.createElement("div");
    workbench.append(area, host);
    document.body.append(workbench);
    root = createRoot(host);

    await act(async () => root?.render(<ServerPopup
      client={{} as CockpitClient}
      sessionId="session"
      popup={{ terminal_id: "popup", title: "Agent Inbox", width: { kind: "percent", value: 80 }, height: { kind: "percent", value: 70 } }}
      live={false}
      error={null}
      focusEpoch={1}
      terminalMouseInput={false}
      onReconnect={() => undefined}
    />));

    expect(observedPopupStyle).toHaveBeenCalledWith({ left: "180px", top: "170px", width: "640px", height: "420px" });
  });
});

describe("ServerPopup touch focus", () => {
  it("keeps the keyboard closed on a coarse-pointer scrim press", async () => {
    vi.stubGlobal("matchMedia", () => ({ matches: true }));
    popupReady.current = true;
    workbench = document.createElement("div");
    document.body.append(workbench);
    root = createRoot(workbench);
    await act(async () => root?.render(<ServerPopup
      client={{} as CockpitClient} sessionId="session"
      popup={{ terminal_id: "popup", title: "Agent Inbox", width: null, height: null }}
      live error={null} focusEpoch={1} terminalMouseInput={false} onReconnect={() => undefined}
    />));
    const textarea = workbench.querySelector<HTMLTextAreaElement>(".xterm-helper-textarea")!;
    textarea.focus();
    workbench.querySelector(".server-popup-scrim")!.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    expect(document.activeElement).toBe(workbench.querySelector(".server-popup"));
  });
});

describe("ServerPopup closing focus", () => {
  it("does not steal focus from the selected terminal when its opener is connected chrome", async () => {
    const frames: FrameRequestCallback[] = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { frames.push(callback); return frames.length; });
    workbench = document.createElement("div");
    workbench.className = "workbench";
    const opener = document.createElement("button");
    const pane = document.createElement("section");
    pane.className = "pane-view is-selected";
    const terminal = document.createElement("textarea");
    terminal.className = "xterm-helper-textarea";
    pane.append(terminal);
    const host = document.createElement("div");
    workbench.append(opener, pane, host);
    document.body.append(workbench);
    opener.focus();
    root = createRoot(host);
    await act(async () => root?.render(<ServerPopup
      client={{} as CockpitClient}
      sessionId="session"
      popup={{ terminal_id: "popup", title: "Agent Inbox", width: null, height: null }}
      live={false}
      error={null}
      focusEpoch={1}
      terminalMouseInput={false}
      onReconnect={() => undefined}
    />));
    expect(document.activeElement).toBe(host.querySelector(".server-popup"));

    await act(async () => root?.render(null));
    // The underlying TerminalPane has regained its confirmed, owned selection.
    terminal.focus();
    frames.splice(0).forEach(callback => callback(0));
    expect(document.activeElement).toBe(terminal);
  });
});
