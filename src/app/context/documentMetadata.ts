import type { ContextDocument } from "../../protocol/generated/v1";
import { frontmatterScalar, readFrontmatter } from "./providerDocument";
import { splitSourceLines } from "./sourceLines";
export type SourceSpan = { start: number; end: number };

type DerivedMarkdown = {
  text: string;
  sourceLines: number[];
  frontmatter: SourceSpan | null;
};
export function sourceLinesForMarkdown(source: string): DerivedMarkdown {
  const lines = splitSourceLines(source);
  let frontmatter: SourceSpan | null = null;
  let bodyStart = 0;
  if (lines[0]?.text.trim() === "---") {
    for (let index = 1; index < lines.length; index += 1) {
      if (lines[index].text.trim() === "---" || lines[index].text.trim() === "...") {
        frontmatter = { start: 1, end: index + 1 };
        bodyStart = index + 1;
        break;
      }
    }
  }
  const body = lines.slice(bodyStart);
  return {
    text: body.map((line) => line.text).join("\n"),
    sourceLines: body.map((_, index) => bodyStart + index + 1),
    frontmatter,
  };
}
export type SourceMetadata = {
  canonicalId: string | null;
  provider: string | null;
  fetchedAt: string | null;
  /** A Confluence page's last edit (P10 `last_modified`, `last_modified_by`). */
  lastModified: string | null;
  lastModifiedBy: string | null;
};

export function sourceMetadata(source: string): SourceMetadata {
  const fields = readFrontmatter(source);
  // Library documents write these as JSON-quoted scalars.
  return { canonicalId: frontmatterScalar(fields.get("canonical_id")), provider: frontmatterScalar(fields.get("provider")), fetchedAt: frontmatterScalar(fields.get("fetched_at")), lastModified: frontmatterScalar(fields.get("last_modified")), lastModifiedBy: frontmatterScalar(fields.get("last_modified_by")) };
}

/** Preview-limit diagnostics are already explained by the bounded-window notice. */
export function isShownDiagnostic(diagnostic: { code: string }): boolean {
  return diagnostic.code !== "context_preview_lines" && diagnostic.code !== "context_preview_bytes";
}

export function documentName(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  return name || path;
}

export function isMarkdown(document: ContextDocument, path: string): boolean {
  return /(?:^|\.)md(?:own)?$/i.test(path) || document.media_type.toLowerCase().includes("markdown");
}

export function isHtml(document: ContextDocument, path: string): boolean {
  return /(?:^|\.)x?html?$/i.test(path) || document.media_type.toLowerCase().includes("html");
}

