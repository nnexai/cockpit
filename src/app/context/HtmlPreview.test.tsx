// @vitest-environment jsdom
import { expect, it } from "vitest";
import { createRoot } from "react-dom/client";
import { act } from "react";
import { HtmlPreview, htmlPreviewDocument, sanitizeHtmlPreview } from "./HtmlPreview";

it("keeps HTML previews in an opaque, inert iframe", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  try {
    await act(async () => mounted.render(<HtmlPreview title="example.html" html={'<script>window.ran = true</script><form action="https://example.test"><a href="https://example.test">go</a><img src="https://example.test/image.png">'} />));
    const frame = host.querySelector("iframe")!;
    expect(frame.getAttribute("sandbox")).toBe("");
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
    const source = frame.getAttribute("srcdoc")!;
    expect(source).toContain("Content-Security-Policy");
    expect(source).toContain("default-src 'none'");
    expect(source).not.toMatch(/<script|<form|href=|src=/i);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("removes active and network-capable markup before creating srcdoc", () => {
  const source = sanitizeHtmlPreview('<base href="https://example.test"><meta http-equiv="refresh" content="0"><object data="https://example.test"></object><iframe src="https://example.test"></iframe><img src="https://example.test/image.png"><p onclick="alert(1)">safe</p>');
  expect(source).not.toMatch(/<base|<meta http-equiv|<object|<iframe|https:\/\/example\.test|onclick=/i);
  expect(source).toContain("<p>safe</p>");
});

it("makes inline SVG static and strips its navigation hooks", () => {
  const source = sanitizeHtmlPreview('<svg><animate attributeName="x"></animate><set attributeName="fill"></set><a href="https://example.test"><rect></rect></a><foreignObject><p>embedded</p></foreignObject><image href="https://example.test/image.png"></image></svg>');
  expect(source).not.toMatch(/animate|<set|href=|foreignObject|https:\/\/example\.test/i);
  expect(source).toContain("<rect></rect>");
});

it("removes nested markup that could be reparsed from noscript or templates", () => {
  const source = sanitizeHtmlPreview('<noscript><img src="https://example.test/noscript.png"></noscript><template shadowrootmode="open"><script>window.ran = true</script></template><p>safe</p>');
  expect(source).not.toMatch(/noscript|template|script|example\.test/i);
  expect(source).toContain("<p>safe</p>");
});
