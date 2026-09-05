import { describe, expect, it } from "vitest";
import { splitSourceLines } from "./sourceLines";

describe("splitSourceLines", () => {
  it("treats LF and CRLF as line endings while retaining lone carriage returns", () => {
    expect(splitSourceLines("first\rcontent\r\nsecond\nlast")).toEqual([
      { text: "first\rcontent", raw: "first\rcontent\r\n" },
      { text: "second", raw: "second\n" },
      { text: "last", raw: "last" },
    ]);
  });
});
