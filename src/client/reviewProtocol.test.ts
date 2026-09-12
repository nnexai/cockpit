import { describe, expect, it } from "vitest";
import { parseReviewSnapshot } from "./reviewProtocol";

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
});
