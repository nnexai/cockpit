import { useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { MessageImage } from "../../protocol/generated/v1";
import "./InboxImages.css";

type Props = { client: CockpitClient; sessionId: string; images: readonly MessageImage[]; label: string };
type ImageState = { status: "loading" } | { status: "ready"; url: string } | { status: "error"; gone: boolean };

function ImagePixels({ client, sessionId, image, alt, lazy = false, open, openLabel }: { client: CockpitClient; sessionId: string; image: MessageImage; alt: string; lazy?: boolean; open?: () => void; openLabel?: string }) {
  const container = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(!lazy);
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<ImageState>({ status: "loading" });
  useEffect(() => {
    if (!lazy || typeof IntersectionObserver === "undefined") { setVisible(true); return; }
    const node = container.current;
    if (!node) return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) { setVisible(true); observer.disconnect(); }
    }, { rootMargin: "200px" });
    observer.observe(node);
    return () => observer.disconnect();
  }, [lazy]);
  useEffect(() => {
    if (!visible) return;
    const controller = new AbortController();
    let url: string | undefined;
    setState({ status: "loading" });
    void client.orchestrationImageRead({ session_id: sessionId, image_id: image.image_id }, controller.signal).then(media => {
      if (controller.signal.aborted) return;
      if (media.image.image_id !== image.image_id || (media.image.mime_type !== "image/png" && media.image.mime_type !== "image/jpeg")) throw new Error("Invalid image response");
      const binary = atob(media.data_base64);
      const bytes = new Uint8Array(new ArrayBuffer(binary.length));
      for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
      url = URL.createObjectURL(new Blob([bytes], { type: media.image.mime_type }));
      setState({ status: "ready", url });
    }).catch((error: unknown) => {
      if (controller.signal.aborted) return;
      const code = typeof error === "object" && error !== null && "operationCode" in error ? error.operationCode : undefined;
      setState({ status: "error", gone: code === "image_not_found" || code === "image_forbidden" });
    });
    return () => { controller.abort(); if (url) URL.revokeObjectURL(url); };
  }, [client, sessionId, image.image_id, visible, attempt]);
  const pixels = state.status === "ready" ? <img src={state.url} alt={alt} width={image.width} height={image.height} draggable={false} referrerPolicy="no-referrer" onError={() => setState({ status: "error", gone: false })} /> : null;
  return <span ref={container} className="inbox-image-pixels">
    {state.status === "loading" ? <span className="inbox-image-loading" role="status">Loading image…</span> : state.status === "error" ? <span className="inbox-image-error" role="status">{state.gone ? "Image no longer available" : "Image unavailable"}{!state.gone && <button type="button" onClick={() => setAttempt(value => value + 1)}>Retry</button>}</span> : open ? <button className="inbox-image-open" type="button" aria-label={openLabel} onClick={open}>{pixels}</button> : pixels}
  </span>;
}

function Lightbox({ client, sessionId, images, index, setIndex, close }: Props & { index: number; setIndex: (index: number) => void; close: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const opener = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  useEffect(() => {
    const node = dialog.current;
    if (!node) return;
    node.showModal();
    closeButton.current?.focus();
    return () => { node.close(); opener.current?.focus(); };
  }, []);
  const image = images[index];
  if (!image) return null;
  return <dialog ref={dialog} className="inbox-image-dialog" aria-label={`Image ${index + 1} of ${images.length}`} onCancel={event => { event.preventDefault(); event.stopPropagation(); close(); }} onClick={event => { if (event.target === event.currentTarget) close(); }} onKeyDown={event => {
    if (event.key === "ArrowLeft" && index > 0) { event.preventDefault(); event.stopPropagation(); setIndex(index - 1); }
    if (event.key === "ArrowRight" && index < images.length - 1) { event.preventDefault(); event.stopPropagation(); setIndex(index + 1); }
  }}>
    <div className="inbox-image-dialog-content">
      <header><strong>Image {index + 1} of {images.length}</strong><button ref={closeButton} type="button" onClick={close}>Close</button></header>
      <div className="inbox-image-full"><ImagePixels key={`${sessionId}:${image.image_id}`} client={client} sessionId={sessionId} image={image} alt={`Image ${index + 1} of ${images.length}`} /></div>
      <footer><span>{image.width} × {image.height} · {(image.bytes / 1024).toFixed(1)} KB</span>{images.length > 1 && <span><button type="button" disabled={index === 0} onClick={() => setIndex(index - 1)}>Previous</button><button type="button" disabled={index === images.length - 1} onClick={() => setIndex(index + 1)}>Next</button></span>}</footer>
    </div>
  </dialog>;
}

/** Displays only structured, session-authorized image bytes, never text-derived URLs. */
export function InboxImages({ client, sessionId, images, label }: Props) {
  const [selected, setSelected] = useState<number | null>(null);
  useEffect(() => { setSelected(null); }, [sessionId]);
  if (!images.length) return null;
  return <div className="inbox-images"><p className="inbox-images-label">{label}</p><ul aria-label={label}>{images.map((image, index) => <li key={image.image_id}>
    <div className="inbox-image-tile" style={{ aspectRatio: `${image.width} / ${image.height}` }}>
      <ImagePixels key={`${sessionId}:${image.image_id}`} client={client} sessionId={sessionId} image={image} alt="" lazy open={() => setSelected(index)} openLabel={`View image ${index + 1} of ${images.length}`} />
    </div>
  </li>)}</ul>{selected !== null && selected < images.length && <Lightbox client={client} sessionId={sessionId} images={images} label={label} index={selected} setIndex={setSelected} close={() => setSelected(null)} />}</div>;
}
