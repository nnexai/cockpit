import { describe, expect, it } from "vitest";
import type { WorkspaceOperation } from "../../protocol/generated/v1";
import { operationSnapshotIsNewer } from "./SetupDialog";

function operation(generation: number, sequence: number): WorkspaceOperation {
  return { generation, sequence } as WorkspaceOperation;
}

describe("SetupDialog operation snapshot ordering", () => {
  it("accepts a newer generation and sequence, but rejects stale or duplicate snapshots", () => {
    const current = operation(4, 12);
    expect(operationSnapshotIsNewer(current, operation(4, 13))).toBe(true);
    expect(operationSnapshotIsNewer(current, operation(5, 0))).toBe(true);
    expect(operationSnapshotIsNewer(current, operation(4, 12))).toBe(false);
    expect(operationSnapshotIsNewer(current, operation(4, 11))).toBe(false);
    expect(operationSnapshotIsNewer(current, operation(3, 99))).toBe(false);
  });

  it("accepts the first authoritative snapshot without inventing an ordering", () => {
    expect(operationSnapshotIsNewer(null, operation(0, 0))).toBe(true);
  });
});
