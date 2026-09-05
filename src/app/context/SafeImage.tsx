import { useEffect, useState } from "react";

import type { ContextMediaRequest } from "../../protocol/generated/v1";
import type { CockpitClient } from "../../client/CockpitClient";

type SafeImageProps = {
  client: CockpitClient;
  sessionId: string;
  paneId: string;
  request: ContextMediaRequest;
  alt: string;
  className?: string;
};

type ImageState =
  | { status: "loading" }
  | { status: "ready"; url: string; width: number; height: number }
  | { status: "error"; message: string };

function readableError(error: unknown): string {
  return error instanceof Error && error.message ? error.message : "The image could not be loaded safely.";
}

function decodeBase64(value: string): Uint8Array<ArrayBuffer> {
  const decoded = atob(value);
  const bytes = new Uint8Array(new ArrayBuffer(decoded.length));
  for (let index = 0; index < decoded.length; index += 1) bytes[index] = decoded.charCodeAt(index);
  return bytes;
}

/** Renders only a blob created from bytes approved by the Cockpit core. */
export function SafeImage({ client, sessionId, paneId, request, alt, className }: SafeImageProps) {
  const [state, setState] = useState<ImageState>({ status: "loading" });
  const { binding_id, root_id, path, expected_revision } = request;

  useEffect(() => {
    const controller = new AbortController();
    let objectUrl: string | null = null;
    setState({ status: "loading" });
    void client.contextMedia(sessionId, paneId, { binding_id, root_id, path, expected_revision }, controller.signal)
      .then((media) => {
        if (controller.signal.aborted) return;
        objectUrl = URL.createObjectURL(new Blob([decodeBase64(media.data_base64)], { type: media.mime_type }));
        setState({ status: "ready", url: objectUrl, width: media.width, height: media.height });
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) setState({ status: "error", message: readableError(error) });
      });
    return () => {
      controller.abort();
      if (objectUrl !== null) URL.revokeObjectURL(objectUrl);
    };
  }, [binding_id, client, expected_revision, paneId, path, root_id, sessionId]);

  if (state.status === "loading") return <span className="context-media-refusal" role="status">Loading image…</span>;
  if (state.status === "error") return <span className="context-media-refusal" role="status">Image unavailable: {state.message}</span>;
  return <img className={className} src={state.url} alt={alt} width={state.width} height={state.height} />;
}
