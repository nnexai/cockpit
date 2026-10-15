const MODAL_SELECTOR = 'dialog[open], [role="dialog"][aria-modal="true"]';

/** File payloads must never trigger the webview's default file navigation. */
export function isExternalFileDrag(event: { dataTransfer: DataTransfer | null }): boolean {
  const transfer = event.dataTransfer;
  return transfer !== null && (
    Array.from(transfer.types).includes("Files")
    || transfer.files.length > 0
    || Array.from(transfer.items).some((item) => item.kind === "file")
  );
}

/** Only the terminal under the pointer may accept a file drop. */
export function isTerminalFileDropTarget(target: EventTarget | null): boolean {
  const element = target instanceof Element ? target : null;
  const host = element?.closest<HTMLElement>(".terminal-host");
  if (!host || host.dataset.closed === "true" || host.closest("[inert]")) return false;
  return Array.from(host.ownerDocument.querySelectorAll(MODAL_SELECTOR)).every((modal) => modal.contains(host));
}

export function installFileDropNavigationGuard(target: Window = window): () => void {
  const preventFileNavigation = (event: DragEvent) => {
    if (!isExternalFileDrag(event)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = isTerminalFileDropTarget(event.target) ? "copy" : "none";
    // Keep propagation intact: the pane under the drop still owns paste routing.
  };
  target.addEventListener("dragover", preventFileNavigation, true);
  target.addEventListener("drop", preventFileNavigation, true);
  return () => {
    target.removeEventListener("dragover", preventFileNavigation, true);
    target.removeEventListener("drop", preventFileNavigation, true);
  };
}
