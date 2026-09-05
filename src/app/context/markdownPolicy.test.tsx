// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import ReactMarkdown from "react-markdown";
import { expect, it } from "vitest";
import { remarkBoundDiagrams } from "./markdownPolicy";
it("caps actual parsed Mermaid blocks across whitespace and list nesting", async () => {
  const source = ['``` mermaid\ngraph TD; A-->B\n```', '~~~  mermaid\ngraph TD; A-->B\n~~~', ...Array(4).fill('- nested\n\n  ``` mermaid\n  graph TD; A-->B\n  ```')].join('\n\n');
  const host = document.createElement('div'); const root = createRoot(host);
  try {
    await act(async () => root.render(<ReactMarkdown remarkPlugins={[remarkBoundDiagrams]} components={{ code: ({node}) => <span data-allowed={String(node?.properties.dataMermaidPreview)} /> }}>{source}</ReactMarkdown>));
    expect(host.querySelectorAll('[data-allowed=true]')).toHaveLength(4);
    expect(host.querySelectorAll('[data-allowed=false]')).toHaveLength(2);
  } finally { await act(async () => root.unmount()); }
});

it("resolves local image references without permitting root escape or URL fetches", async () => {
  const { localImagePath } = await import("./ContextViewer");
  expect(localImagePath("docs/readme.md", "../images/a%20b.png")).toBe("images/a b.png");
  expect(localImagePath("readme.md", "../outside.png")).toBeNull();
  expect(localImagePath("readme.md", "https://remote.example/x.png")).toBeNull();
  expect(localImagePath("readme.md", "%2Fetc/passwd")).toBeNull();
  expect(localImagePath("docs/readme.md", "..%5Coutside.png")).toBeNull();
});
