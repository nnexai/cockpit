import { expect, it } from "vitest";
import { mermaidFrame, mermaidRefusal } from "./mermaidFrame";
it("rejects directive overrides and bounds Mermaid bytes and physical lines", () => {
  expect(mermaidRefusal('%%{init: {securityLevel: "loose"}}%%\ngraph TD; A-->B')).toContain("directives");
  expect(mermaidRefusal("graph TD; A-->B")).toBeNull();
  expect(mermaidRefusal("α".repeat(6001))).toContain("limit");
  expect(mermaidRefusal("\n".repeat(201))).toContain("limit");
});
it("hostile diagram text remains JSON data inside a frame with no network or host privileges", () => {
  const hostile = '</script><script>parent.document.body.textContent="owned"</script>';
  const frame = mermaidFrame("/* trusted library fixture */", hostile, "fixture");
  expect(frame).not.toContain(hostile);
  expect(frame).toContain('\\u003c/script>');
  expect(frame).toContain("connect-src 'none'");
  expect(frame).toContain("script-src 'nonce-fixture'");
  expect(frame).not.toContain("script-src 'unsafe-inline'");
});
