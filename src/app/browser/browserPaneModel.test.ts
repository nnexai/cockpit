import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DelayedNotice } from "./browserPaneModel";

describe("DelayedNotice", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  const notice = () => {
    const shown: string[] = []; const cleared: string[] = [];
    return { shown, cleared, delayed: new DelayedNotice((message) => shown.push(message), (message) => cleared.push(message), 2_000) };
  };

  it("stays silent when the problem resolves within the delay", () => {
    const { shown, cleared, delayed } = notice();
    delayed.report("behind");
    vi.advanceTimersByTime(1_500);
    delayed.resolve();
    vi.advanceTimersByTime(5_000);
    expect(shown).toEqual([]);
    expect(cleared).toEqual([]);
  });

  it("shows once the problem outlasts the delay, measured from the first report", () => {
    const { shown, delayed } = notice();
    delayed.report("behind");
    vi.advanceTimersByTime(1_000);
    delayed.report("still behind");
    vi.advanceTimersByTime(1_000);
    expect(shown).toEqual(["still behind"]);
  });

  it("clears a shown notice when the problem resolves", () => {
    const { shown, cleared, delayed } = notice();
    delayed.report("behind");
    vi.advanceTimersByTime(2_000);
    delayed.resolve();
    expect(shown).toEqual(["behind"]);
    expect(cleared).toEqual(["behind"]);
  });

  it("never shows after being disposed", () => {
    const { shown, delayed } = notice();
    delayed.report("behind");
    delayed.dispose();
    vi.advanceTimersByTime(5_000);
    expect(shown).toEqual([]);
  });
});
