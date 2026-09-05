const PREVIEW_CSP = "default-src 'none'; base-uri 'none'; connect-src 'none'; font-src data:; form-action 'none'; frame-src 'none'; img-src data:; media-src 'none'; object-src 'none'; style-src 'unsafe-inline'";

export function sanitizeHtmlPreview(html: string): string {
  const template = document.createElement("template");
  template.innerHTML = html;
  const content = template.content;
  for (const element of content.querySelectorAll("script, noscript, template, iframe, frame, object, embed, applet, base, form, meta, link")) element.remove();
  for (const element of content.querySelectorAll("*")) {
    if (["animate", "set", "animatemotion", "animatetransform", "mpath", "foreignobject"].includes(element.localName.toLowerCase())) element.remove();
  }
  for (const element of content.querySelectorAll("*")) {
    for (const attribute of [...element.attributes]) {
      const name = attribute.name.toLowerCase();
      const isDataImage = element instanceof HTMLImageElement && name === "src" && /^data:image\/(?:avif|gif|jpe?g|png|webp);/i.test(attribute.value);
      if (name.startsWith("on") || ["action", "formaction", "href", "srcdoc", "srcset", "target", "xlink:href"].includes(name) || (name === "src" && !isDataImage)) {
        element.removeAttribute(attribute.name);
      }
    }
  }
  return template.innerHTML;
}

export function htmlPreviewDocument(html: string): string {
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${PREVIEW_CSP}"><style>html { color-scheme: dark; } body { margin: 0; padding: 16px 32px 40px; background: #0C1016; color: #D8DEE8; font: 14px/22px \"IBM Plex Sans\", \"Noto Sans\", sans-serif; overflow-wrap: anywhere; } pre { overflow: auto; } table { border-collapse: collapse; } th, td { padding: 4px 8px; border: 1px solid #2A3340; }</style></head><body>${sanitizeHtmlPreview(html)}</body></html>`;
}

export function HtmlPreview({ html, title }: { html: string; title: string }) {
  return <iframe className="context-html-preview" title={`${title} preview`} sandbox="" referrerPolicy="no-referrer" srcDoc={htmlPreviewDocument(html)} />;
}
