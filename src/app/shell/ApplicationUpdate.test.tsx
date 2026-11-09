// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { BackendSurfaceStatus, CockpitClient, UpdateStatus } from "../../client/CockpitClient";
import { ApplicationUpdateDialog, ApplicationUpdateSlot, useApplicationUpdate } from "./ApplicationUpdate";
import type { ApplicationUpdateState } from "./ApplicationUpdate";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const status: BackendSurfaceStatus = {
  backend: { state: "running", instance_id: "instance-1", build_commit: "old-build", installed: true, update_installed: true, last_exit: null, ephemeral: null },
  surface: { kind: "native", installed: true, update_installed: true, build_commit: "old-build", frontend_entry: null },
  installed_build_commit: "new-build", restart: null,
};

describe("application update", () => {
  it("offers the combined native action and confirms impact before dispatch", async () => {
    const restart = vi.fn(async () => ({ pid: 42, started_at: "now", relaunch_app: true }));
    const client = { backendStatus: vi.fn(async () => status), updateAndRelaunch: restart, updateStatus: vi.fn(async () => ({ source_path: "", installed_commit: null, commits_behind_main: null })) } as unknown as CockpitClient;
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    function Harness() {
      const update = useApplicationUpdate(client);
      return <><ApplicationUpdateSlot update={update} client={client} onUpdate={() => undefined} />{update.dialog ? <ApplicationUpdateDialog update={update} /> : null}</>;
    }
    try {
      await act(async () => { root.render(<Harness />); });
      const chip = host.querySelector<HTMLButtonElement>(".update-chip")!;
      expect(chip.textContent).toContain("Update installed");
      await act(async () => { chip.click(); });
      expect(host.textContent).toContain("Could not read what is open");
      expect(document.activeElement?.textContent).toBe("Cancel");
      expect(restart).not.toHaveBeenCalled();
      const confirm = [...host.querySelectorAll("button")].find(button => button.textContent === "Restart Cockpit to update")!;
      await act(async () => { confirm.click(); });
      expect(restart).toHaveBeenCalledWith("instance-1");
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });
  it("checks once when visible, stays idle, and refreshes on focus or visibility", async () => {
    vi.useFakeTimers();
    const visibility = Object.getOwnPropertyDescriptor(document, "visibilityState");
    const client = {
      backendStatus: vi.fn(async () => ({ ...status, backend: { ...status.backend, update_installed: false }, surface: { ...status.surface, update_installed: false } })),
      updateStatus: vi.fn(async () => ({ source_path: "/repo", installed_commit: "abc", commits_behind_main: 1 })),
    } as unknown as CockpitClient;
    const host = document.createElement("div"), root = createRoot(host);
    function Harness() {
      const update = useApplicationUpdate(client);
      return <ApplicationUpdateSlot update={update} client={client} onUpdate={() => undefined} />;
    }
    try {
      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
      await act(async () => { root.render(<Harness />); });
      expect(client.backendStatus).toHaveBeenCalledTimes(1);
      expect(client.updateStatus).toHaveBeenCalledTimes(1);
      await act(async () => { await vi.advanceTimersByTimeAsync(1_260_000); });
      expect(client.backendStatus).toHaveBeenCalledTimes(1);
      expect(client.updateStatus).toHaveBeenCalledTimes(1);
      await act(async () => { window.dispatchEvent(new Event("focus")); });
      expect(client.backendStatus).toHaveBeenCalledTimes(2);
      expect(client.updateStatus).toHaveBeenCalledTimes(2);
      await act(async () => { Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true }); document.dispatchEvent(new Event("visibilitychange")); window.dispatchEvent(new Event("focus")); await vi.advanceTimersByTimeAsync(1_260_000); });
      expect(client.backendStatus).toHaveBeenCalledTimes(2);
      expect(client.updateStatus).toHaveBeenCalledTimes(2);
      await act(async () => { Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true }); document.dispatchEvent(new Event("visibilitychange")); });
      expect(client.backendStatus).toHaveBeenCalledTimes(3);
      expect(client.updateStatus).toHaveBeenCalledTimes(3);
    } finally {
      await act(async () => root.unmount());
      if (visibility) Object.defineProperty(document, "visibilityState", visibility);
      else Reflect.deleteProperty(document, "visibilityState");
      vi.useRealTimers();
    }
  });
  it("polls an open dialog and an active restart, stopping when closed, hidden or settled", async () => {
    vi.useFakeTimers();
    const visibility = Object.getOwnPropertyDescriptor(document, "visibilityState");
    let current: BackendSurfaceStatus = { ...status, backend: { ...status.backend, update_installed: false }, surface: { ...status.surface, update_installed: false } };
    const backendStatus = vi.fn(async () => current);
    const client = { backendStatus } as unknown as CockpitClient;
    const host = document.createElement("div"), root = createRoot(host);
    let update: ApplicationUpdateState;
    function Harness() { update = useApplicationUpdate(client); return null; }
    try {
      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
      await act(async () => { root.render(<Harness />); });
      expect(backendStatus).toHaveBeenCalledTimes(1);
      await act(async () => { update.setDialog(true); });
      expect(backendStatus).toHaveBeenCalledTimes(2);
      await act(async () => { await vi.advanceTimersByTimeAsync(4_000); });
      expect(backendStatus).toHaveBeenCalledTimes(4);
      await act(async () => { Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true }); document.dispatchEvent(new Event("visibilitychange")); await vi.advanceTimersByTimeAsync(5_000); });
      expect(backendStatus).toHaveBeenCalledTimes(4);
      await act(async () => { Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true }); document.dispatchEvent(new Event("visibilitychange")); });
      expect(backendStatus).toHaveBeenCalledTimes(5);
      await act(async () => { update.setDialog(false); await vi.advanceTimersByTimeAsync(0); });
      await act(async () => { await vi.advanceTimersByTimeAsync(1_260_000); });
      expect(backendStatus).toHaveBeenCalledTimes(5);

      current = { ...current, restart: { pid: 42, started_at: "now", relaunch_app: false, step: "restart", result: { state: "running" }, finished_at: null } };
      await act(async () => { window.dispatchEvent(new Event("focus")); });
      const runningCount = backendStatus.mock.calls.length;
      await act(async () => { await vi.advanceTimersByTimeAsync(3_000); });
      expect(backendStatus).toHaveBeenCalledTimes(runningCount + 3);
      current = { ...current, restart: { ...current.restart!, result: { state: "ok" }, finished_at: "done" } };
      await act(async () => { await vi.advanceTimersByTimeAsync(1_000); });
      const settledCount = backendStatus.mock.calls.length;
      await act(async () => { await vi.advanceTimersByTimeAsync(1_260_000); });
      expect(backendStatus).toHaveBeenCalledTimes(settledCount);
    } finally {
      await act(async () => root.unmount());
      if (visibility) Object.defineProperty(document, "visibilityState", visibility);
      else Reflect.deleteProperty(document, "visibilityState");
      vi.useRealTimers();
    }
  });
  it("coalesces focus refreshes while checks are in flight", async () => {
    let resolveBackend!: (value: BackendSurfaceStatus) => void;
    let resolveUpdate!: (value: UpdateStatus) => void;
    const backendStatus = vi.fn()
      .mockImplementationOnce(() => new Promise<BackendSurfaceStatus>(resolve => { resolveBackend = resolve; }))
      .mockResolvedValue(status);
    const updateValue = { source_path: "/repo", installed_commit: "abc", commits_behind_main: 1 };
    const updateStatus = vi.fn()
      .mockImplementationOnce(() => new Promise<UpdateStatus>(resolve => { resolveUpdate = resolve; }))
      .mockResolvedValue(updateValue);
    const client = { backendStatus, updateStatus } as unknown as CockpitClient;
    const host = document.createElement("div"), root = createRoot(host);
    function Harness() {
      const update = useApplicationUpdate(client);
      return <ApplicationUpdateSlot update={update} client={client} onUpdate={() => undefined} />;
    }
    try {
      await act(async () => { root.render(<Harness />); });
      await act(async () => { window.dispatchEvent(new Event("focus")); window.dispatchEvent(new Event("focus")); });
      expect(backendStatus).toHaveBeenCalledTimes(1);
      expect(updateStatus).toHaveBeenCalledTimes(1);
      await act(async () => {
        resolveBackend({ ...status, backend: { ...status.backend, update_installed: false }, surface: { ...status.surface, update_installed: false } });
        resolveUpdate(updateValue);
      });
      expect(backendStatus).toHaveBeenCalledTimes(2);
      expect(updateStatus).toHaveBeenCalledTimes(2);
    } finally {
      await act(async () => root.unmount());
    }
  });
});
