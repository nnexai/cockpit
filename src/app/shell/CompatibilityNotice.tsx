import type { CockpitStatus } from "../../client/CockpitClient";
import type { StatusError } from "./model";
import { plainError } from "./plainErrors";

export function CompatibilityNotice({ status, error, retry, onOpenLibrary }: { status: CockpitStatus | null; error: StatusError | null; retry: () => void; onOpenLibrary: () => void }) {
  const herdr = status?.herdr;
  const title = error ? "Cockpit unavailable" : herdr?.status === "incompatible" ? "Herdr is incompatible" : "Herdr is unavailable";
  const message = error?.message ?? (herdr && herdr.status !== "compatible" ? herdr.message : undefined);
  const code = error?.code ?? (herdr && herdr.status !== "compatible" ? herdr.code : undefined);
  const described = plainError(code, message ?? "Could not read the Herdr compatibility status.");
  return <main className="compatibility-main" aria-live="polite"><section className="notice notice-error" role="alert"><p className="eyebrow">Cockpit</p><h1>{title}</h1><p title={error?.detail ?? described.detail}>{described.text}</p><div className="notice-actions"><button type="button" className="action-button" onClick={retry}>Retry status</button><OpenLibraryButton onOpen={onOpenLibrary} /></div></section></main>;
}

/** The Library needs no Herdr session, so every no-session screen can open it. */
export function OpenLibraryButton({ onOpen }: { onOpen: () => void }) {
  return <button type="button" className="action-button" data-library-opener onClick={onOpen}>Open Library</button>;
}
