import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type {
  ContextDirectory,
  ContextDocument,
  ContextFileIndex,
  ContextFileIndexMode,
  ContextInvalidationRequest,
  ContextInvalidationResponse,
  ContextMedia,
  ContextRoot,
  ContextSearchRequest,
  ContextSearchResponse,
  ViewerContext,
} from "../../protocol/generated/v1";
import { viewerErrorCode } from "../layout/viewerLifecycle";

/** Client-side id of the Library root in every Context root selector; the server root id stays inside the reader. */
export const LIBRARY_ROOT_ID = "library";

export type ContextDirectoryRead = { root_id: string; path: string; offset?: number; revision?: string };
export type ContextDocumentRead = { root_id: string; path: string; expected_revision: string | null; offset?: number };
export type ContextMediaRead = { root_id: string; path: string; expected_revision: string | null };

/**
 * One place the Context viewer reads files from: a viewer's authorized roots or
 * the session-independent Library. `identity` changes whenever reads go to a
 * different authority, so late responses from the previous one are discarded.
 */
export interface ContextReader {
  readonly identity: string;
  directory(request: ContextDirectoryRead, signal?: AbortSignal): Promise<ContextDirectory>;
  fileIndex(rootId: string, mode: ContextFileIndexMode, signal?: AbortSignal): Promise<ContextFileIndex>;
  document(request: ContextDocumentRead, signal?: AbortSignal): Promise<ContextDocument>;
  media(request: ContextMediaRead, signal?: AbortSignal): Promise<ContextMedia>;
  search?(request: ContextSearchRequest, signal: AbortSignal): Promise<ContextSearchResponse>;
  invalidate?(request: ContextInvalidationRequest, signal: AbortSignal): Promise<ContextInvalidationResponse>;
}

export function viewerReader(client: CockpitClient, context: ViewerContext, onViewerError?: (error: unknown) => void): ContextReader {
  const { session_id: sessionId, viewer_id: viewerId, binding_id: bindingId } = context;
  const read = async <T,>(request: Promise<T>, signal?: AbortSignal): Promise<T> => {
    try {
      return await request;
    } catch (error) {
      if (!signal?.aborted && viewerErrorCode(error) === "viewer_not_found") onViewerError?.(error);
      throw error;
    }
  };
  return {
    identity: `viewer\u0000${sessionId}\u0000${viewerId}\u0000${bindingId}`,
    directory: (request, signal) => read(client.contextDirectory(sessionId, viewerId, { binding_id: bindingId, ...request }, signal), signal),
    fileIndex: (rootId, mode, signal) => read(client.contextFileIndex(sessionId, viewerId, { binding_id: bindingId, root_id: rootId, mode }, signal), signal),
    document: (request, signal) => read(client.contextDocument(sessionId, viewerId, { binding_id: bindingId, ...request }, signal), signal),
    media: (request, signal) => read(client.contextMedia(sessionId, viewerId, { binding_id: bindingId, ...request }, signal), signal),
    search: (request, signal) => read(client.contextSearch(sessionId, viewerId, request, signal), signal),
    invalidate: (request, signal) => read(client.contextInvalidate(sessionId, viewerId, request, signal), signal),
  };
}

/**
 * Reads the Library root returned by the listing. Responses must come from
 * that same server root; they are presented under `LIBRARY_ROOT_ID` so the
 * viewer's per-root state does not depend on the Library's filesystem identity.
 * Library roots have no search or change polling.
 */
export function libraryReader(client: CockpitClient, root: ContextRoot): ContextReader {
  const own = <T extends { root_id: string }>(value: T): T => {
    if (value.root_id !== root.root_id) throw new CockpitClientError("malformed_response", "The Library root changed; reload the Library.");
    return { ...value, root_id: LIBRARY_ROOT_ID };
  };
  return {
    identity: `library\u0000${root.root_id}`,
    directory: async ({ path, offset, revision }, signal) => own(await client.libraryDirectory({ path, offset: offset ?? null, revision: revision ?? null }, signal)),
    fileIndex: async (_rootId, mode, signal) => own(await client.libraryFileIndex({ mode }, signal)),
    document: async ({ path, expected_revision, offset }, signal) => own(await client.libraryDocument({ path, expected_revision, offset: offset ?? null }, signal)),
    media: async ({ path, expected_revision }, signal) => own(await client.libraryMedia({ path, expected_revision }, signal)),
  };
}
