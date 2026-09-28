// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { expect, it } from "vitest";
import { providerFacts, remarkProviderDocument, type ProviderDocumentOptions } from "./providerDocument";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const body = [
  "# Rotate signing keys",
  "",
  "**Task** · **To Do**",
  "",
  "## Description",
  "",
  "### Plan",
  "",
  "Rotate them.",
  "",
  "## Comments (20 of 35)",
  "",
  "### Konni Hartmann · 2026-09-28 09:21",
  "[#10067](https://jira.test/browse/OPS-1?focusedCommentId=10067)",
  "",
  "First.",
  "",
  "#### Details",
  "",
  "- a",
  "",
  "### Ann · 2026-09-29 10:02 · edited 2026-09-29 11:00 · review on src/a.rs:12",
  "#note_55",
  "",
  "### Bob · 2026-09-30 08:00",
  "",
  "No permalink here.",
  "",
  "## Links",
  "",
  "### Not a comment",
].join("\n");

async function render(markdown: string, options: ProviderDocumentOptions) {
  const host = document.createElement("div");
  const root = createRoot(host);
  const positions: string[] = [];
  await act(async () => root.render(<ReactMarkdown remarkPlugins={[remarkGfm, [remarkProviderDocument, options]]} components={{
    h3: ({ node, children, className }) => { positions.push(`${node?.position?.start.line}-${node?.position?.end.line}`); return <h3 className={className}>{children}</h3>; },
  }}>{markdown}</ReactMarkdown>));
  return { host, positions, unmount: () => act(async () => root.unmount()) };
}

it("groups each comment heading of a partial Comments section with its body into a card, up to the next section", async () => {
  const { host, positions, unmount } = await render(body, { comments: true, hiddenTitle: null, summaryLead: null });
  try {
    const cards = [...host.querySelectorAll("article.provider-comment-card")];
    expect(cards).toHaveLength(3);
    const header = (card: Element) => card.querySelector(".provider-comment-header")!;
    expect(header(cards[0]).querySelector("strong")?.textContent).toBe("Konni Hartmann");
    expect(header(cards[0]).querySelector(".provider-comment-date")?.textContent).toBe("2026-09-28 09:21");
    expect(header(cards[0]).querySelector(".provider-comment-edited")).toBeNull();
    // The permalink paragraph moves into the header; the body keeps its demoted headings and lists.
    expect(header(cards[0]).querySelector(".provider-comment-permalink a")?.getAttribute("href")).toBe("https://jira.test/browse/OPS-1?focusedCommentId=10067");
    expect([...cards[0].children].map((child) => child.tagName)).toEqual(["H3", "P", "H4", "UL"]);
    expect(header(cards[1]).querySelector(".provider-comment-edited")?.textContent).toBe("edited 2026-09-29 11:00");
    expect(header(cards[1]).querySelector(".provider-comment-tag")?.textContent).toBe("review on src/a.rs:12");
    expect(header(cards[1]).querySelector(".provider-comment-permalink")?.textContent).toBe("#note_55");
    expect(header(cards[2]).querySelector(".provider-comment-permalink")).toBeNull();
    expect(cards[2].querySelector("p")?.textContent).toBe("No permalink here.");
    // Headings outside Comments stay plain, and the header maps to its heading and permalink lines.
    expect(host.querySelector("h3:not(.provider-comment-header)")?.textContent).toBe("Plan");
    expect(host.querySelectorAll("h3:not(.provider-comment-header)")).toHaveLength(2);
    expect(positions).toEqual(["7-7", "13-14", "22-23", "25-25", "31-31"]);
    expect(host.querySelector("h1")?.textContent).toBe("Rotate signing keys");
  } finally { await unmount(); }
});

it("leaves documents that aren't generated provider snapshots, or lack a counted Comments heading, unchanged", async () => {
  const plain = await render(body, { comments: false, hiddenTitle: null, summaryLead: null });
  const uncounted = await render(body.replace("## Comments (20 of 35)", "## Comments"), { comments: true, hiddenTitle: null, summaryLead: null });
  try {
    expect(plain.host.querySelector(".provider-comment-card")).toBeNull();
    expect(uncounted.host.querySelector(".provider-comment-card")).toBeNull();
  } finally { await plain.unmount(); await uncounted.unmount(); }
});

it("hides only a leading title heading that repeats the header title, and the facts summary right after it", async () => {
  const hidden = await render(body, { comments: true, hiddenTitle: "Rotate signing keys", summaryLead: null });
  const withSummary = await render(body, { comments: true, hiddenTitle: "Rotate signing keys", summaryLead: "Task" });
  const otherKind = await render(body, { comments: true, hiddenTitle: "Rotate signing keys", summaryLead: "Bug" });
  const different = await render(body, { comments: true, hiddenTitle: "Another title", summaryLead: "Task" });
  const notLeading = await render(`Intro\n\n${body}`, { comments: true, hiddenTitle: "Rotate signing keys", summaryLead: "Task" });
  try {
    expect(hidden.host.querySelector("h1")).toBeNull();
    expect(hidden.host.firstElementChild?.textContent).toBe("Task · To Do");
    expect(withSummary.host.firstElementChild?.textContent).toBe("Description");
    expect(otherKind.host.firstElementChild?.textContent).toBe("Task · To Do");
    // The title stays, so its summary does too.
    expect(different.host.querySelector("h1")?.textContent).toBe("Rotate signing keys");
    expect(different.host.querySelector("p")?.textContent).toBe("Task · To Do");
    expect(notLeading.host.querySelector("h1")?.textContent).toBe("Rotate signing keys");
  } finally { for (const view of [hidden, withSummary, otherKind, different, notLeading]) await view.unmount(); }
});

it("reads provider facts from generated frontmatter, telling an unassigned item from one without assignees", () => {
  const frontmatter = (lines: string[]) => ["---", "provider: \"jira\"", "generated: true", ...lines, "container:", "  label: \"nested\"", "---", "# T"].join("\n");
  expect(providerFacts(frontmatter(["item_type: \"Task\"", "status: \"To Do\"", "priority: \"Medium\"", "assignee: null", "author: \"Konni\"", "updated: \"2026-09-28T09:21:21+02:00\""]))).toEqual({
    generated: true, itemType: "Task", status: "To Do", priority: "Medium", assignee: null, author: "Konni", created: null, updated: "2026-09-28T09:21:21+02:00",
  });
  expect(providerFacts(frontmatter(["status: \"open\""])).assignee).toBeUndefined();
  expect(providerFacts(frontmatter(["status: \"open\""])).status).toBe("open");
  expect(providerFacts("---\nprovider: \"jira\"\n---\n").generated).toBe(false);
  expect(providerFacts("# No frontmatter\n").generated).toBe(false);
});
