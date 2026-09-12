// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { routeWorkbenchKeydown } from "./keymap";

afterEach(() => document.body.replaceChildren());

it("leaves prefix commands inside an open comment dialog", () => {
  const dialog = document.createElement("dialog");
  dialog.open = true;
  const button = document.createElement("button");
  button.textContent = "Save comment";
  dialog.append(button);
  document.body.append(dialog);
  button.focus();
  const runCommand = vi.fn();
  const setPrefixActive = vi.fn();
  const setCommandsOpen = vi.fn();
  const routing = { modalOpen: false, prefixActive: false, runCommand, setPrefixActive, setCommandsOpen };
  const route = (event: KeyboardEvent) => routeWorkbenchKeydown(event, routing);
  window.addEventListener("keydown", route, true);
  try {
    button.dispatchEvent(new KeyboardEvent("keydown", { key: "b", ctrlKey: true, bubbles: true }));
    routing.prefixActive = true;
    for (const key of ["c", "v", "x", "?"]) {
      button.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
    }
    expect(document.activeElement).toBe(button);
    expect(runCommand).not.toHaveBeenCalled();
    expect(setPrefixActive).not.toHaveBeenCalled();
    expect(setCommandsOpen).not.toHaveBeenCalled();
  } finally {
    window.removeEventListener("keydown", route, true);
  }
});
