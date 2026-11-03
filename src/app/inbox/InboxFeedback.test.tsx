// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { InboxFeedback } from "./InboxFeedback";

it("renders rich report structure without activating untrusted content", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const report = [
    "## Findings", "", "A **strong** result and `code`.", "", "- first", "- second", "",
    "| Check | Result |", "| --- | --- |", "| safety | passed |", "",
    "```sh", "echo hello", "```", "", "[external](https://example.test) [http](http://example.test) [unsafe](javascript:alert(1)) [relative](/private) [mail](mailto:x@example.test)",
    "![private image](https://example.test/tracker.png)", "", "<img src=x onerror=alert(1)>", "<script>alert(1)</script>",
  ].join("\n");
  try {
    await act(async () => root.render(<InboxFeedback text={report} />));
    expect(host.querySelector("h2")?.textContent).toBe("Findings");
    expect(host.querySelector("strong")?.textContent).toBe("strong");
    expect(host.querySelectorAll("li")).toHaveLength(2);
    expect(host.querySelector("table td")?.textContent).toBe("safety");
    expect(host.querySelector("pre code")?.textContent).toContain("echo hello");
    expect(host.textContent).toContain("external");
    expect(host.textContent).toContain("unsafe");
    expect(host.textContent).toContain("private image");
    const links = [...host.querySelectorAll("a")];
    expect(links.map(link => link.href)).toEqual(["https://example.test/", "http://example.test/"]);
    for (const link of links) {
      expect(link.target).toBe("_blank");
      expect(link.rel).toBe("noopener noreferrer");
    }
    expect(host.querySelector("img, script, iframe")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("keeps plain-text reports and their line breaks", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  try {
    await act(async () => root.render(<InboxFeedback text={"First line\nSecond line"} />));
    expect(host.querySelector("p")?.textContent).toBe("First line\nSecond line");
  } finally {
    await act(async () => root.unmount());
  }
});
