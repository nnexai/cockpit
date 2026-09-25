import { useEffect, useRef, useState } from "react";
import { mermaidFrame, mermaidRefusal } from "./mermaidFrame";

export function MermaidView({ source }: { source: string }) {
  const [frame, setFrame] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>("Rendering diagram…");
  const [height, setHeight] = useState(120);
  const frameRef = useRef<HTMLIFrameElement>(null);
  useEffect(() => {
    setFrame(null); setHeight(120);
    const refusal = mermaidRefusal(source);
    if (refusal) { setStatus(refusal); return; }
    setStatus("Rendering diagram…"); let active = true;
    const nonce = crypto.randomUUID().replaceAll("-", "");
    const timer = window.setTimeout(() => { active = false; setFrame(null); setStatus("Diagram preview timed out. Source is retained."); }, 5000);
    const receive = (event: MessageEvent) => {
      if (!active || event.source !== frameRef.current?.contentWindow || event.data?.kind !== "cockpit-mermaid" || event.data?.nonce !== nonce) return;
      clearTimeout(timer);
      if (event.data.ok === true && Number.isFinite(event.data.height)) { setHeight(Math.max(80, Math.min(640, event.data.height))); setStatus(null); }
      else { setFrame(null); setStatus("Diagram could not be rendered. Source is retained."); }
    };
    window.addEventListener("message", receive);
    void import("mermaid/dist/mermaid.min.js?raw").then(library => { if (active) setFrame(mermaidFrame(library.default, source, nonce)); }).catch(() => { if (active) { clearTimeout(timer); setStatus("Diagram renderer unavailable. Source is retained."); } });
    return () => { active = false; clearTimeout(timer); window.removeEventListener("message", receive); };
  }, [source]);
  return <div className="context-mermaid">
    {frame ? <iframe ref={frameRef} title="Mermaid diagram preview" sandbox="allow-scripts" referrerPolicy="no-referrer" srcDoc={frame} style={{ width: "100%", height, border: 0, pointerEvents: "none" }} tabIndex={-1} /> : null}
    {status ? <p role="status">{status}</p> : null}
    <details><summary>Diagram source</summary><pre><code>{source}</code></pre></details>
  </div>;
}
