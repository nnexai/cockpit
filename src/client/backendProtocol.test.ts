import { describe, expect, it } from "vitest";
import { parseBackendSurfaceStatus } from "./backendProtocol";

const status = {
  backend: { state: "running", instance_id: "instance-1", build_commit: "old", installed: true, update_installed: true, last_exit: null, ephemeral: { browser_associations: 1, comment_batches: 2, widgets: 3 }, herdr: null, expected_herdr: null },
  surface: { kind: "web", installed: true, update_installed: false, build_commit: "old", frontend_entry: "/assets/index-new.js" },
  installed_build_commit: "new", restart: null,
};
const ticket = { pid: 123, started_at: "2026-01-01T00:00:00Z", relaunch_app: false };

describe("background service transport", () => {
  it("rejects an unknown restart state", () => {
    expect(() => parseBackendSurfaceStatus({ ...status, restart: { ...ticket, step: "stop", finished_at: null, result: { state: "unknown" } } })).toThrow();
  });
});
