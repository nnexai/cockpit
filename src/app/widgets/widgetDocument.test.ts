// @vitest-environment jsdom
import { expect, it } from "vitest";
import { widgetDocument } from "./widgetDocument";

function page(html: string) {
  const iframe = document.createElement("iframe");
  document.body.append(iframe);
  const child = iframe.contentWindow as Window & typeof globalThis;
  const parsed = new DOMParser().parseFromString(widgetDocument(html, { nonce: "anchors", revision: 1, selection: null }), "text/html");
  child.document.replaceChild(child.document.importNode(parsed.documentElement, true), child.document.documentElement);
  const script = child.document.querySelector("script")!.textContent!;
  new Function("window", "parent", "TextEncoder", "Element", script)(child, { postMessage() {} }, TextEncoder, child.Element);
  const dom = { window: child, close: () => iframe.remove() };
  // jsdom has no navigation engine. Observe the destination at author-handler
  // time, then cancel navigation; actual scrolling is verified in the browser.
  child.document.addEventListener("click", event => event.preventDefault());
  return dom;
}

it.each(["#detail", "#caf%C3%A9", "#legacy", "#", "#missing", "#bad%escape"])(
  "resolves widget-local %s independently of the embedding page", href => {
    const dom = page(`<a href="${href}"><span>Jump</span></a><h2 id="detail">Detail</h2>`);
    try {
      const anchor = dom.window.document.querySelector("a")!;
      let destination = "";
      anchor.addEventListener("click", () => { destination = anchor.href; });
      dom.window.document.querySelector("span")!.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true, cancelable: true }));
      expect(destination).toBe(`about:srcdoc${href}`);
    } finally { dom.close(); }
  },
);

it("handles dynamically inserted self-target anchors and preserves author cancellation", () => {
  const dom = page("<div></div>");
  try {
    const anchor = dom.window.document.createElement("a");
    anchor.href = "#dynamic";
    anchor.target = "_SELF";
    dom.window.document.body.append(anchor);
    let authorRan = false;
    anchor.addEventListener("click", event => { authorRan = true; event.preventDefault(); });
    const event = new dom.window.MouseEvent("click", { bubbles: true, cancelable: true, detail: 0 });
    expect(anchor.dispatchEvent(event)).toBe(false);
    expect(authorRan).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(anchor.href).toBe("about:srcdoc#dynamic");
  } finally { dom.close(); }
});

it.each([
  { href: "https://example.org/page#detail" },
  { href: "/another#detail" },
  { href: "about:srcdoc#detail" },
  { href: "#detail", target: "_blank" },
  { href: "#detail", target: "_top" },
  { href: "#detail", target: "_parent" },
  { href: "#detail", target: "other" },
  { href: "#detail", download: true },
  { href: "#detail", ctrlKey: true },
  { href: "#detail", metaKey: true },
  { href: "#detail", shiftKey: true },
  { href: "#detail", altKey: true },
  { href: "#detail", button: 1 },
])("leaves other navigation intent unchanged: %j", intent => {
  const dom = page("<a>Navigate</a>");
  try {
    const anchor = dom.window.document.querySelector("a")!;
    anchor.setAttribute("href", intent.href);
    if ("target" in intent) anchor.setAttribute("target", intent.target!);
    if ("download" in intent) anchor.setAttribute("download", "");
    anchor.dispatchEvent(new dom.window.MouseEvent("click", { ...intent, bubbles: true, cancelable: true }));
    expect(anchor.getAttribute("href")).toBe(intent.href);
  } finally { dom.close(); }
});
