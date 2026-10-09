import { describe, expect, it } from "vitest";
import { parseLibraryOperation, parseSpaceAddRequest, parseSpaceRemoveRequest, parseSpaceRepositoriesRequest, matchSpaceOperation } from "./libraryProtocol";

describe("parseLibraryOperation", () => {
  it("accepts all selected item IDs from one save-and-select operation", () => {
    const item_ids = Array.from({ length: 5_001 }, (_, index) => `item-${index}`);
    const operation = parseLibraryOperation({
      operation_id: "operation-1",
      kind: "space_add",
      phases: [],
      item_ids: [],
      report: null,
      space: {
        space_id: "space-1",
        item_ids,
      },
      target: null,
      cancel_requested: false,
      finished: true,
      created_at: "2026-09-27T00:00:00Z",
      updated_at: "2026-09-27T00:00:01Z",
    });

    expect(operation.space?.item_ids).toEqual(item_ids);
  });
});

describe("Library-direct Space requests", () => {
  const target = { session_id: "session", space_id: "space" };
  it("rejects unknown request fields", () => {
    expect(() => parseSpaceAddRequest({ target, item_ids: [], unexpected: true })).toThrow();
    expect(() => parseSpaceRemoveRequest({ target, item_ids: [], unexpected: true })).toThrow();
    expect(() => parseSpaceRepositoriesRequest({ target, repository_paths: [], unexpected: true })).toThrow();
  });
  it("checks the completed selection phase against the requested Space identity", () => {
    const operation = parseLibraryOperation({
      operation_id: "op", kind: "space_add", phases: [], item_ids: [], report: null,
      space: { space_id: "other", item_ids: [] }, target, cancel_requested: false, finished: true,
      created_at: "now", updated_at: "now",
    });
    expect(() => matchSpaceOperation(operation, target)).toThrow();
  });
});
