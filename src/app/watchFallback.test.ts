// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { watchFallback } from "./watchFallback";

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

it("runs only during a visible outage, serializes reads, and stops on recovery or close", async () => {
  vi.useFakeTimers();
  let visible = true;
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visible ? "visible" : "hidden");
  // The frontend's ES2022 target does not expose Promise.withResolvers.
  let finish!: () => void;
  const pending = new Promise<void>(resolve => { finish = resolve; });
  const refresh = vi.fn().mockReturnValueOnce(pending).mockResolvedValue(undefined);
  const fallback = watchFallback(refresh);
  await vi.advanceTimersByTimeAsync(120_000);
  expect(refresh).not.toHaveBeenCalled();
  fallback.start();
  await vi.advanceTimersByTimeAsync(30_000);
  expect(refresh).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(120_000);
  expect(refresh).toHaveBeenCalledTimes(1);
  visible = false; document.dispatchEvent(new Event("visibilitychange"));
  finish(); await Promise.resolve(); await Promise.resolve();
  await vi.advanceTimersByTimeAsync(120_000);
  expect(refresh).toHaveBeenCalledTimes(1);
  visible = true; document.dispatchEvent(new Event("visibilitychange"));
  await vi.advanceTimersByTimeAsync(30_000);
  expect(refresh).toHaveBeenCalledTimes(2);
  fallback.stop();
  await vi.advanceTimersByTimeAsync(120_000);
  expect(refresh).toHaveBeenCalledTimes(2);
  fallback.start(); fallback.close();
  await vi.advanceTimersByTimeAsync(120_000);
  expect(refresh).toHaveBeenCalledTimes(2);
});
