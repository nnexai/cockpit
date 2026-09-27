import { describe, expect, it } from "vitest";
import { parseLibraryOperation } from "./libraryProtocol";

describe("parseLibraryOperation", () => {
  it("accepts more than 5,000 paths written by one Space operation", () => {
    const written = Array.from({ length: 5_001 }, (_, index) => `folders/project/file-${index}.md`);
    const operation = parseLibraryOperation({
      operation_id: "operation-1",
      kind: "space_update",
      phases: [],
      item_ids: [],
      report: null,
      space: {
        space_id: "space-1",
        copy_mode: "copy",
        written,
        skipped_edited: [],
        companion_root_id: "companion-1",
      },
      target: null,
      cancel_requested: false,
      finished: true,
      created_at: "2026-09-27T00:00:00Z",
      updated_at: "2026-09-27T00:00:01Z",
    });

    expect(operation.space?.written).toHaveLength(5_001);
  });
});
