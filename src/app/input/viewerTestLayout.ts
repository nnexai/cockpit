import { beforeEach, vi } from "vitest";

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    unobserve() {}
    disconnect() {}
  });
  Object.defineProperties(HTMLDialogElement.prototype, {
    show: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
    showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
    close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
  });
});
