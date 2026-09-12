import { expect, it } from "vitest";
import { reviewCommentBatchIdentity } from "./ReviewViewer";

const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding" } as const;

it("uses the CommentDrafts source identity and comparison for review count status", () => {
  const allLocal = reviewCommentBatchIdentity(presentation, "checkout", "all_local");

  expect(allLocal).toBe("session\u0000pane\u0000binding\u0000checkout\u0000checkout\u0000all_local");
  expect(reviewCommentBatchIdentity(presentation, "checkout", "all_local")).toBe(allLocal);
  expect(reviewCommentBatchIdentity(presentation, "checkout", "staged")).not.toBe(allLocal);
  expect(reviewCommentBatchIdentity(presentation, "other-checkout", "all_local")).not.toBe(allLocal);
});
