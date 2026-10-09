import type { ContextViewerProps } from "../context/ContextViewer";
import type { LibraryViewerSource } from "./useLibraryViewerSource";
import type { LibraryViewerController } from "./useLibraryViewerController";
import { LibraryMenu, type LibraryMenuEntry } from "./LibraryTree";
import { AddContextDialog } from "./AddContextDialog";
import { LibraryConfirmDialog } from "./LibraryConfirmDialog";
import { ContextResources } from "../context/ContextResources";
import { providerFamily } from "./libraryState";
import { announceLibraryChanged } from "./useLibraryOperation";
import { copyText } from "./clipboard";
import { formatShortcut } from "../input/shortcuts";
export function LibraryViewerDialogs({ client, context, value, onChange, onOpenRepository, source, controller,
  refresh, resourcesOpen, setResourcesOpen, selectedPath }: Pick<ContextViewerProps, "client" | "context" | "value" | "onChange" | "onOpenRepository"> & {
  source: LibraryViewerSource; controller: LibraryViewerController; refresh: () => void;
  resourcesOpen: boolean; setResourcesOpen: (open: boolean) => void;
  selectedPath: string | null;
}) {
  const { library } = source;
  const { compactToolbar, displaySpace, libraryBusy, refreshLibrary, setLibraryAdd, providerCredentials,
    spaceListing, requestLibraryItem, libraryToolbarMenu, setLibraryToolbarMenu, libraryAdd,
    libraryConfirm, setLibraryConfirm, setLibraryReportVerb, setLibraryReportDismissed,
    setLibraryPendingIds, startLibraryOperation } = controller;
  const refreshAllDisabled = libraryBusy || !library.listing || library.listing.items.length === 0;
  // Local re-read (not a provider refresh) and the path; the worded commands join them when the toolbar is compact.
  const libraryMenuEntries: LibraryMenuEntry[] = [
    ...(compactToolbar ? [
      { label: "Add…", onSelect: () => setLibraryAdd(displaySpace?.live ? "space" : "library") },
      { label: "Refresh all", onSelect: refreshLibrary, disabled: refreshAllDisabled },
      "separator" as const,
    ] : []),
    { label: "Provider token…", onSelect: () => providerCredentials.actions.open("") },
    "separator" as const,
    { label: "Reload listing", shortcut: formatShortcut("reload-listing"), onSelect: refresh },
    { label: "Copy Library folder path", onSelect: () => { if (library.listing) void copyText(library.listing.root.path); }, disabled: !library.listing },
  ];
  return <>
      {resourcesOpen && context ? <ContextResources client={client} space={displaySpace} spaceListing={spaceListing} onAdd={() => setLibraryAdd("space")} onClose={() => setResourcesOpen(false)} onOpenItem={(item) => { requestLibraryItem(item.item_id); setResourcesOpen(false); }} onOpenRepository={onOpenRepository} /> : null}
      {libraryToolbarMenu ? <LibraryMenu x={libraryToolbarMenu.x} y={libraryToolbarMenu.y} label="Library actions" onDismiss={() => setLibraryToolbarMenu(null)} entries={libraryMenuEntries} /> : null}
      {providerCredentials.dialog}
      {libraryAdd ? <AddContextDialog client={client} onClose={() => setLibraryAdd(null)} space={displaySpace}
        onOpenItem={(itemId) => { requestLibraryItem(itemId); }} defaultDestination={libraryAdd} /> : null}
      {libraryConfirm?.kind === "remove" ? <LibraryConfirmDialog title={`Remove "${libraryConfirm.item.title}" from the Library?`} safeLabel="Cancel" confirmLabel="Remove from Library" destructive
        body={<p>Removes this item from the Library. Selected items remain available while a Space references them. {providerFamily(library.providers, libraryConfirm.item.provider_id).name} isn't changed. You can add it again from its link.</p>}
        onClose={() => setLibraryConfirm(null)}
        onConfirm={async () => {
          const item = libraryConfirm.item;
          await client.libraryRemove({ mode: "item", item_id: item.item_id, expected_revision: item.revision });
          if (selectedPath === item.document_path) onChange({ ...value, path: null });
          setLibraryConfirm(null);
          announceLibraryChanged();
        }} /> : null}
      {libraryConfirm?.kind === "replace" ? <LibraryConfirmDialog title="Replace the edited Library file?" safeLabel="Keep file" confirmLabel="Replace with source version" destructive
        body={<p>This Library file was changed outside Cockpit. Replacing it fetches the source version and discards those changes. Spaces selecting it read the new Library version.</p>}
        onClose={() => setLibraryConfirm(null)}
        onConfirm={async () => {
          const item = libraryConfirm.item;
          setLibraryConfirm(null);
          setLibraryReportVerb("Replace");
          setLibraryReportDismissed(false);
          setLibraryPendingIds(new Set([item.item_id]));
          await startLibraryOperation(() => client.libraryReplace({ item_id: item.item_id, confirmed: item.conflict }));
        }} /> : null}
  </>;
}
