// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { useRestoreFocus } from "./LibraryConfirmDialog";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

function Dialog() {
  useRestoreFocus();
  return <div role="dialog"><button type="button">Inside</button></div>;
}

/** A surface with an opener that can disappear while its dialog is open, like `Provider token…` once a token is stored. */
function Surface() {
  const [open, setOpen] = useState(false);
  const [opener, setOpener] = useState(true);
  return <section aria-label="pane">
    <div className="head" aria-label="head">
      {opener ? <button type="button" onClick={() => setOpen(true)}>Opener</button> : <span>Stored</span>}
    </div>
    <button type="button" onClick={() => setOpener(false)}>Drop opener</button>
    <button type="button" onClick={() => setOpen(false)}>Close</button>
    {open ? <Dialog /> : null}
  </section>;
}

async function run(steps: (host: HTMLElement) => Promise<void>) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<Surface />));
    await steps(host);
  } finally { await act(async () => root.unmount()); host.remove(); }
}
const press = (host: HTMLElement, name: string) => act(async () => { [...host.querySelectorAll("button")].find((node) => node.textContent === name)!.click(); });

it("returns focus to the opener when it still exists", async () => {
  await run(async (host) => {
    const opener = [...host.querySelectorAll("button")].find((node) => node.textContent === "Opener")!;
    opener.focus();
    await press(host, "Opener");
    await act(async () => { (host.querySelector("[role='dialog'] button") as HTMLElement).focus(); });
    await press(host, "Close");
    expect(document.activeElement).toBe(opener);
  });
});

it("falls back to the opener's nearest surviving container, without leaving it focusable or ringed afterwards", async () => {
  await run(async (host) => {
    const opener = [...host.querySelectorAll("button")].find((node) => node.textContent === "Opener")!;
    const head = host.querySelector<HTMLElement>(".head")!;
    opener.focus();
    await press(host, "Opener");
    await press(host, "Drop opener");
    expect(opener.isConnected).toBe(false);
    await press(host, "Close");
    expect(document.activeElement).toBe(head);
    expect(head.style.outline).toBe("none");
    // The next Tab leaves it: it was a landing place, not a control.
    await act(async () => { host.querySelector<HTMLElement>("section button:last-of-type")!.focus(); });
    expect(head.hasAttribute("tabindex")).toBe(false);
    expect(head.style.outline).toBe("");
  });
});
