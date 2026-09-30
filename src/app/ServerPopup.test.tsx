// @vitest-environment jsdom
import { act } from "react";
import type * as ReactTypes from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../client/CockpitClient";

const observedPopupStyle = vi.hoisted(() => vi.fn());
vi.mock("./TerminalPane", async () => {
  const React = await vi.importActual<typeof ReactTypes>("react");
  return {
    TerminalPane: () => {
      React.useLayoutEffect(() => {
        const popup = document.querySelector<HTMLElement>(".server-popup");
        if (popup) observedPopupStyle({ left: popup.style.left, top: popup.style.top, width: popup.style.width, height: popup.style.height });
      }, []);
      return React.createElement("div");
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
