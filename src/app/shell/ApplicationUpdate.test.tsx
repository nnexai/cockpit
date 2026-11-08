// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { BackendSurfaceStatus, CockpitClient } from "../../client/CockpitClient";
import { ApplicationUpdateDialog, ApplicationUpdateSlot, useApplicationUpdate } from "./ApplicationUpdate";

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
});
