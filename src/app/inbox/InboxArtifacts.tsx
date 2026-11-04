import { useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { InboxArtifact as Artifact } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { InboxFeedback } from "./InboxFeedback";
import "./InboxImages.css";

type Props = { client: CockpitClient; sessionId: string; runId: string; artifact: Artifact };
type Content = { kind: "image"; url: string } | { kind: "html" | "markdown"; text: string };
type LoadState = { key: string; status: "loading" } | { key: string; status: "ready"; content: Content } | { key: string; status: "error"; message: string };

const htmlPolicy = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; form-action 'none'; base-uri 'none'; frame-src 'none'";

function sandboxedHtml(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("base, script, iframe, frame, object, embed, form").forEach(element => element.remove());
  doc.querySelectorAll("meta[http-equiv]").forEach(element => {
    if (element.getAttribute("http-equiv")?.trim().toLowerCase() === "refresh") element.remove();
  });
  doc.querySelectorAll("*").forEach(element => {
    for (const attribute of Array.from(element.attributes)) {
      const name = attribute.name.toLowerCase();
      if (name === "href" || name === "xlink:href" || name === "action" || name === "formaction" || name === "src") {
        if (element.localName.toLowerCase() === "a" || !attribute.value.trim().toLowerCase().startsWith("data:")) {
          element.removeAttributeNode(attribute);
        }
      }
    }
  });
  const policy = doc.createElement("meta");
  policy.setAttribute("http-equiv", "Content-Security-Policy");
  policy.setAttribute("content", htmlPolicy);
  doc.head.prepend(policy);
  return "<!doctype html>" + doc.documentElement.outerHTML;
}

function ArtifactDialog({ name, content, close, opener }: { name: string; content: Content; close: () => void; opener: HTMLButtonElement | null }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const node = dialog.current;
    if (!node) return;
    node.showModal();
    closeButton.current?.focus();
    return () => { node.close(); opener?.focus(); };
  }, [opener]);

  return <dialog ref={dialog} className="inbox-image-dialog" aria-label={name} onCancel={event => { event.preventDefault(); event.stopPropagation(); close(); }} onClick={event => { if (event.target === event.currentTarget) close(); }}>
    <div className="inbox-image-dialog-content">
      <header><strong>{name}</strong><button ref={closeButton} type="button" onClick={close}>Close</button></header>
      <div className="inbox-image-full inbox-artifact-view">
        {content.kind === "image" ? <img src={content.url} alt={name} draggable={false} />
          : content.kind === "html" ? <iframe sandbox="" referrerPolicy="no-referrer" title={name} srcDoc={sandboxedHtml(content.text)} />
          : <InboxFeedback text={content.text} />}
      </div>
    </div>
  </dialog>;
}

export function InboxArtifact({ client, sessionId, runId, artifact }: Props) {
  const key = JSON.stringify([sessionId, runId, artifact.path, artifact.kind]);
  const [state, setState] = useState<LoadState>({ key, status: "loading" });
  const [open, setOpen] = useState(false);
  const tile = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const controller = new AbortController();
    let url: string | undefined;
    setOpen(false);
    setState({ key, status: "loading" });
    if (artifact.kind === "other") {
      setState({ key, status: "error", message: "Unsupported artifact" });
    } else {
      void client.orchestrationArtifactRead({ session_id: sessionId, run_id: runId, path: artifact.path }, controller.signal).then(media => {
        if (controller.signal.aborted) return;
        if (media.path !== artifact.path || media.kind !== artifact.kind) throw new Error("Invalid artifact response");
        const binary = atob(media.data_base64);
        const bytes = new Uint8Array(new ArrayBuffer(binary.length));
        for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
        let content: Content;
        if (media.kind === "image") {
          if (!["image/png", "image/jpeg", "image/webp"].includes(media.mime_type)) throw new Error("Invalid image response");
          url = URL.createObjectURL(new Blob([bytes], { type: media.mime_type }));
          content = { kind: "image", url };
        } else if (media.kind === "html" || media.kind === "markdown") {
          if (media.mime_type !== (media.kind === "html" ? "text/html" : "text/markdown")) throw new Error("Invalid artifact response");
          content = { kind: media.kind, text: new TextDecoder("utf-8", { fatal: true }).decode(bytes) };
        } else {
          throw new Error("Unsupported artifact");
        }
        setState({ key, status: "ready", content });
      }).catch((error: unknown) => {
        if (!controller.signal.aborted) setState({ key, status: "error", message: error instanceof Error ? error.message : String(error) });
      });
    }
    return () => { controller.abort(); if (url) URL.revokeObjectURL(url); };
  }, [client, sessionId, runId, artifact.path, artifact.kind, key]);

  const current = state.key === key ? state : { key, status: "loading" as const };
  const isDocument = artifact.kind === "markdown";
  return <>
    <button ref={tile} type="button" className={isDocument ? "inbox-doc" : `inbox-shot${artifact.kind === "html" ? " inbox-shot-html" : ""}`} aria-label={artifact.name} aria-disabled={current.status !== "ready"} title={current.status === "error" ? current.message : artifact.name} onClick={() => { if (current.status === "ready") setOpen(true); }}>
      {current.status === "error" ? <span className="inbox-artifact-unavailable">Unavailable</span>
        : isDocument ? <><UiIcon name="file" /><span>{artifact.name}</span><em>md</em></>
        : current.status === "ready" && current.content.kind === "image" ? <img src={current.content.url} alt="" draggable={false} onError={() => setState({ key, status: "error", message: "Image unavailable" })} />
        : artifact.kind === "html" ? <UiIcon name="code" /> : null}
      {!isDocument && <span className="inbox-shot-name">{artifact.name}</span>}
    </button>
    {open && current.status === "ready" && <ArtifactDialog name={artifact.name} content={current.content} close={() => setOpen(false)} opener={tile.current} />}
  </>;
}
