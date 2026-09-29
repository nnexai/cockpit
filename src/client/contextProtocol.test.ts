import { describe, expect, it } from "vitest";
import { parseContextDirectory, parseContextDocument } from "./contextProtocol";

describe("context paging protocol", () => {

  it("normalizes nullable paging fields from complete responses", () => {
    const directory = parseContextDirectory({
      binding_id: "binding", root_id: "folder", path: "", entries: [], truncated: false,
      revision: null, next_offset: null, total_entries: null, diagnostics: [],
    });
    expect(directory.revision).toBeUndefined();
    expect(directory.next_offset).toBeUndefined();
    const document = parseContextDocument({
      binding_id: "binding", root_id: "folder", path: "notes.md", revision: "r1", content_hash: null,
      bytes: 0, media_type: "text/plain", text: "", truncated: false,
      offset: null, next_offset: null, total_bytes: null, line_offset: null, diagnostics: [],
    });
    expect(document.next_offset).toBeUndefined();
    expect(document.line_offset).toBeUndefined();
  });
});
