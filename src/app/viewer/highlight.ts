import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import css from "highlight.js/lib/languages/css";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import markdown from "highlight.js/lib/languages/markdown";
import python from "highlight.js/lib/languages/python";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

for (const [name, language] of Object.entries({ bash, c, cpp, css, go, ini, java, javascript, json, kotlin, markdown, python, ruby, rust, sql, swift, typescript, xml, yaml })) {
  hljs.registerLanguage(name, language);
}

const byExtension: Record<string, string> = {
  sh: "bash", bash: "bash", zsh: "bash", fish: "bash",
  c: "c", h: "c", cc: "cpp", cpp: "cpp", cxx: "cpp", hpp: "cpp",
  css: "css", scss: "css",
  go: "go",
  toml: "ini", ini: "ini", cfg: "ini", conf: "ini",
  java: "java",
  js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "javascript",
  json: "json", jsonc: "json",
  kt: "kotlin", kts: "kotlin",
  md: "markdown", markdown: "markdown",
  py: "python",
  rb: "ruby",
  rs: "rust",
  sql: "sql",
  swift: "swift",
  ts: "typescript", tsx: "typescript", mts: "typescript", cts: "typescript",
  html: "xml", htm: "xml", xml: "xml", svg: "xml", vue: "xml",
  yml: "yaml", yaml: "yaml",
};

/** Larger sources stay plain; highlighting them would stall the pane. */
const MAX_HIGHLIGHT_BYTES = 512 * 1024;

export function languageFor(path: string | null | undefined): string | null {
  const name = path?.slice(path.lastIndexOf("/") + 1).toLowerCase() ?? "";
  if (name === "dockerfile" || name === "makefile") return null;
  const extension = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "";
  return byExtension[extension] ?? null;
}

/**
 * Highlight a whole source and split the escaped HTML into one string per
 * line, closing and reopening spans that cross a line break.
 */
export function highlightLines(text: string, path: string | null | undefined): string[] | null {
  const language = languageFor(path);
  if (!language || text.length > MAX_HIGHLIGHT_BYTES) return null;
  let html: string;
  try { html = hljs.highlight(text, { language, ignoreIllegals: true }).value; } catch { return null; }
  const lines: string[] = [];
  const open: string[] = [];
  let current = "";
  for (const part of html.split(/(<span[^>]*>|<\/span>|\n)/)) {
    if (part === "\n") {
      lines.push(current + "</span>".repeat(open.length));
      current = open.join("");
    } else if (part.startsWith("<span")) {
      open.push(part);
      current += part;
    } else if (part === "</span>") {
      open.pop();
      current += part;
    } else {
      current += part;
    }
  }
  lines.push(current + "</span>".repeat(open.length));
  if (text.endsWith("\n")) lines.pop();
  return lines;
}

/** Highlight one diff line on its own; multi-line constructs are not tracked. */
export function highlightLine(text: string, path: string | null | undefined): string | null {
  return highlightLines(text, path)?.[0] ?? null;
}
