type Node = { type?: string; lang?: string; children?: Node[]; data?: { hProperties?: Record<string, unknown> } };

/** Count parsed code blocks, independently of CommonMark fence spelling,
 * indentation, list nesting, or whitespace around the language name. */
export function remarkBoundDiagrams() {
  return (tree: unknown) => {
    let diagrams = 0;
    const visit = (node: Node) => {
      if (node.type === "code" && node.lang === "mermaid") {
        node.data ??= {}; node.data.hProperties ??= {};
        node.data.hProperties.dataMermaidPreview = ++diagrams <= 4;
      }
      for (const child of node.children ?? []) visit(child);
    };
    visit(tree as Node);
  };
}
