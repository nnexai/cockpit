import { describe, expect, it } from "vitest";
import { parseReviewFile, parseReviewSnapshot } from "./reviewProtocol";

describe("review snapshot protocol", () => {
  it("accepts a complete changed-file inventory beyond the former 256-file cap", () => {
    const files = Array.from({ length: 257 }, (_, index) => ({
      file_id: `file-${index}`,
      comparison: "untracked",
      status: "untracked",
      old_path: null,
      new_path: `generated/${index}.txt`,
      binary: false,
      additions: null,
      deletions: null,
      summary: "untracked",
      old_revision: null,
      new_revision: "worktree",
    }));
    const snapshot = parseReviewSnapshot({
      binding_id: "binding",
      session_id: "session",
      pane_id: "pane",
      review_id: "review",
      generation: 1,
      repository_id: "repository",
      checkout_path: "/tmp/checkout",
      source_id: "source",
      comparison: "untracked",
      base_revision: null,
      head_revision: null,
      index_revision: "index",
      worktree_revision: "worktree",
      files,
      truncated: false,
      diagnostics: [],
    });
    expect(snapshot.files).toHaveLength(257);
  });

  it("normalizes nullable source paging fields on complete files", () => {
    const file = parseReviewFile({
      binding_id: "binding", session_id: "session", pane_id: "pane", review_id: "review", generation: 1,
      file: { file_id: "file", comparison: "untracked", status: "untracked", old_path: null, new_path: "file.txt", binary: false, additions: null, deletions: null, summary: "file", old_revision: null, new_revision: "r1" },
      hunks: [], old_source: null, new_source: null, old_source_hash: null, new_source_hash: null,
      old_source_offset: null, new_source_offset: null, old_source_total_bytes: null, new_source_total_bytes: null,
      old_total_lines: null, new_total_lines: null, old_source_truncated: false, new_source_truncated: false,
      truncated: false, diagnostics: [],
    });
    expect(file.old_source_offset).toBe(0);
    expect(file.new_source_offset).toBe(0);
  });
});
