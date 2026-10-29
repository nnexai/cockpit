import { describe, expect, it } from "vitest";
import { plainError } from "./plainErrors";

describe("plainError", () => {
  it("keeps a recognized refusal's original code and message in detail", () => {
    const result = plainError("request_outcome_unknown", "write timed out");
    expect(result.text).not.toContain("request_outcome_unknown");
    expect(result.text).toMatch(/resync/i);
    expect(result.detail).toBe("request_outcome_unknown: write timed out");
  });

  it("leaves unknown and absent codes' host messages unchanged", () => {
    expect(plainError("future_code", "original message")).toEqual({ text: "original message", detail: "future_code: original message" });
    expect(plainError(undefined, "original message")).toEqual({ text: "original message" });
    expect(plainError("toString", "original message").text).toBe("original message");
  });
});
