// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { libraryViewSlots } from "./LibraryViewerView";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

it("restores focus only when a focused watch action disappears", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const refresh = vi.fn();
  const retry = vi.fn();
  const render = async (unavailable: boolean, selected = true) => {
    const args = {
      source: { library: { watchUnavailable: unavailable, retryWatch: retry } },
      controller: {}, toolbar: { refresh }, document: {}, narrow: false,
    } as unknown as Parameters<typeof libraryViewSlots>[0];
    const notice = libraryViewSlots(args).watchNotice;
    await act(async () => root.render(<section className="context-viewer">
      {notice}<aside className="context-tree">{selected ? <button aria-current="true">Selected</button> : null}</aside>
      <main className="context-document" tabIndex={-1}>Document</main><button>Outside notice</button>
    </section>));
  };
  try {
    await render(true);
    const retryButton = [...host.querySelectorAll("button")].find((button) => button.textContent === "Retry automatic updates")!;
    retryButton.focus();
    await act(async () => retryButton.click());
    expect(retry).toHaveBeenCalledOnce();
    await render(false);
    expect(document.activeElement?.textContent).toBe("Selected");

    await render(true, false);
    host.querySelector<HTMLButtonElement>(".context-notice button")!.focus();
    await render(false, false);
    expect(document.activeElement).toBe(host.querySelector(".context-document"));

    await render(true);
    const outside = [...host.querySelectorAll("button")].find((button) => button.textContent === "Outside notice")!;
    outside.focus();
    await render(false);
    expect(document.activeElement).toBe(outside);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
