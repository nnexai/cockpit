export type SourceLine = { text: string; raw: string };

/**
 * Split canonical Context source into LF-delimited physical lines without
 * changing the snapshot bytes retained for comment capture. A CR immediately
 * before LF belongs to that delimiter; a lone CR remains source content.
 */
export function splitSourceLines(source: string): SourceLine[] {
  const lines: SourceLine[] = [];
  let start = 0;
  for (let end = source.indexOf("\n", start); end !== -1; end = source.indexOf("\n", start)) {
    const raw = source.slice(start, end + 1);
    lines.push({ text: raw.endsWith("\r\n") ? raw.slice(0, -2) : raw.slice(0, -1), raw });
    start = end + 1;
  }
  if (start < source.length || lines.length === 0) lines.push({ text: source.slice(start), raw: source.slice(start) });
  return lines;
}
