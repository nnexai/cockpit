import { useEffect, useId, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ImageLimits, MessageImage } from "../../protocol/generated/v1";
import "./inbox-compose-images.css";

type DraftImage = { id: string; file: File; status: "preparing" | "ready" | "error"; error?: string; retryable?: boolean; image?: MessageImage; url?: string; controller?: AbortController };
type Drafts = Record<string, DraftImage[]>;
const nameOf = (file: File) => (file.name || "Pasted image").replace(/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069/\\]/g, "_");
const sizeOf = (bytes: number) => bytes < 1024 ? `${bytes} B` : `${Math.ceil(bytes / 1024)} KB`;
function readFile(file: File, signal: AbortSignal): Promise<string> {
  // ES2022 does not expose Promise.withResolvers.
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    const abort = () => reader.abort();
    signal.addEventListener("abort", abort, { once: true });
    reader.onload = () => { signal.removeEventListener("abort", abort); const result = reader.result; typeof result === "string" ? resolve(result.slice(result.indexOf(",") + 1)) : reject(new Error("Could not read image")); };
    reader.onerror = () => { signal.removeEventListener("abort", abort); reject(new Error("Could not read image")); };
    reader.onabort = () => { signal.removeEventListener("abort", abort); reject(new Error("Cancelled")); };
    if (signal.aborted) reject(new Error("Cancelled")); else reader.readAsDataURL(file);
  });
}

export function useInboxImages(client: CockpitClient, sessionId: string, key: string, limits: ImageLimits | undefined) {
  const [drafts, setDrafts] = useState<Drafts>({});
  const draftsRef = useRef<Drafts>({});
  const [announcement, setAnnouncement] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const attach = useRef<HTMLButtonElement>(null);
  const tray = useRef<HTMLUListElement>(null);
  const limitDescription = useId();
  const imageDescription = useId();
  const update = (fn: (current: Drafts) => Drafts) => { const next = fn(draftsRef.current); draftsRef.current = next; setDrafts(next); };
  useEffect(() => () => { for (const entries of Object.values(draftsRef.current)) for (const entry of entries) { entry.controller?.abort(); if (entry.url) URL.revokeObjectURL(entry.url); } }, []);
  const start = (draftKey: string, id: string, file: File) => {
    const controller = new AbortController();
    update(current => ({ ...current, [draftKey]: (current[draftKey] ?? []).map(item => item.id === id ? { ...item, status: "preparing", error: undefined, controller } : item) }));
    void (async () => {
      try {
        const data_base64 = await readFile(file, controller.signal);
        const image = await client.orchestrationImagePut({ session_id: sessionId, data_base64 }, controller.signal);
        if (controller.signal.aborted) return;
        const entries = draftsRef.current[draftKey] ?? [];
        if (!entries.some(item => item.id === id && item.controller === controller)) return;
        if (entries.some(item => item.id !== id && item.image?.image_id === image.image_id)) {
          update(current => ({ ...current, [draftKey]: (current[draftKey] ?? []).map(item => item.id === id && item.controller === controller ? { ...item, status: "error", error: "This image is already attached.", retryable: false, controller: undefined } : item) }));
          setAnnouncement(`${nameOf(file)} is already attached`);
          return;
        }
        const url = URL.createObjectURL(new Blob([file], { type: image.mime_type }));
        update(current => ({ ...current, [draftKey]: (current[draftKey] ?? []).map(item => item.id === id && item.controller === controller ? { ...item, status: "ready", image, url, controller: undefined } : item) }));
        setAnnouncement(`${nameOf(file)} ready`);
      } catch (error) {
        if (controller.signal.aborted) return;
        update(current => ({ ...current, [draftKey]: (current[draftKey] ?? []).map(item => item.id === id && item.controller === controller ? { ...item, status: "error", error: error instanceof Error ? error.message : "Upload failed", retryable: true, controller: undefined } : item) }));
        setAnnouncement(`${nameOf(file)} could not be attached`);
      }
    })();
  };
  const add = (files: FileList | File[]) => {
    if (!limits) return;
    for (const file of Array.from(files)) {
      const id = crypto.randomUUID();
      const count = (draftsRef.current[key] ?? []).filter(item => item.status !== "error" || item.retryable).length;
      const error = count >= limits.max_count ? `${limits.max_count} images maximum.` : !limits.mime_types.includes(file.type) ? "PNG or JPEG images only." : file.size > limits.max_bytes ? `Image exceeds ${sizeOf(limits.max_bytes)}.` : undefined;
      update(current => ({ ...current, [key]: [...(current[key] ?? []), { id, file, status: error ? "error" : "preparing", error }] }));
      if (!error) start(key, id, file);
    }
    setAnnouncement(`${files.length} image${files.length === 1 ? "" : "s"} added`);
  };
  const remove = (id: string) => {
    const entries = draftsRef.current[key] ?? [];
    const index = entries.findIndex(item => item.id === id);
    const item = entries[index];
    if (!item) return;
    item.controller?.abort(); if (item.url) URL.revokeObjectURL(item.url);
    update(current => ({ ...current, [key]: (current[key] ?? []).filter(entry => entry.id !== id) }));
    setAnnouncement(`${nameOf(item.file)} removed`);
    requestAnimationFrame(() => { const buttons = tray.current?.querySelectorAll<HTMLButtonElement>(".inbox-image-remove"); (buttons?.[Math.min(index, buttons.length - 1)] ?? attach.current)?.focus(); });
  };
  const clear = (draftKey: string, imageIds: string[]) => {
    const entries = draftsRef.current[draftKey] ?? [];
    if (entries.length !== imageIds.length || entries.some((item, i) => item.image?.image_id !== imageIds[i])) return;
    for (const item of entries) { item.controller?.abort(); if (item.url) URL.revokeObjectURL(item.url); }
    update(current => ({ ...current, [draftKey]: [] }));
  };
  const entries = drafts[key] ?? [];
  const ready = entries.every(item => item.status === "ready");
  const ids = entries.flatMap(item => item.image ? [item.image.image_id] : []);
  const atLimit = !!limits && entries.filter(item => item.status !== "error" || item.retryable).length >= limits.max_count;
  const controls = (disabled: boolean) => <>
    {entries.length > 0 && <ul ref={tray} className="inbox-image-tray" aria-label={`Attached images, ${entries.length} of ${limits?.max_count ?? 0}`}>{entries.map(item => <li key={item.id} data-error={item.status === "error"}>
      {item.url ? <img src={item.url} alt="" draggable={false} /> : <span className="inbox-image-placeholder" aria-hidden="true" />}
      <span className="inbox-image-info"><strong title={item.file.name}>{nameOf(item.file)}</strong><span>{item.status === "preparing" ? "Preparing…" : item.status === "ready" ? `Ready · ${sizeOf(item.file.size)}` : `Not attached: ${item.error}`}</span></span>
      {item.status === "error" && item.retryable && <button type="button" disabled={disabled} onClick={() => start(key, item.id, item.file)}>Retry</button>}
      <button type="button" className="inbox-image-remove" aria-label={`Remove ${nameOf(item.file)}`} disabled={disabled} onClick={() => remove(item.id)}>Remove</button>
    </li>)}</ul>}
    {entries.length > 0 && <p id={imageDescription} className="inbox-compose-hint">Images are shared with the owner and workers on this request. Image metadata may be retained.</p>}
    {atLimit && <p id={limitDescription} className="inbox-compose-hint">{limits?.max_count} images maximum. Remove an image to add another.</p>}
    <span className="inbox-sr" role="status">{announcement}</span>
  </>;
  const picker = (disabled: boolean) => <><input ref={input} hidden type="file" accept={limits?.mime_types.join(",")} multiple onChange={event => { if (event.currentTarget.files) add(event.currentTarget.files); event.currentTarget.value = ""; }} /><button ref={attach} type="button" className="inbox-btn inbox-attach" aria-label="Attach images" aria-describedby={[entries.length > 0 && imageDescription, atLimit && limitDescription].filter(Boolean).join(" ") || undefined} disabled={disabled || !limits || atLimit} onClick={() => input.current?.click()}>Attach</button></>;
  return { entries, ready, ids, add, clear, controls, picker };
}
