import { useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, SpaceContextListing, SpaceTarget } from "../../protocol/generated/v1";
import type { ItemSpaceState } from "./LibraryItemHeader";
import { sameSpaceTarget, type LibrarySpace } from "./libraryState";
import { announceLibraryChanged, useLibraryOperation, useSpaceContextListing } from "./useLibraryOperation";
export function useLibraryViewerSpace(client: CockpitClient, space: LibrarySpace | null) {
  const spaceLive = space?.live ? space : null;
  const spaceListing = useSpaceContextListing(client, spaceLive?.target ?? null, spaceLive !== null);
  const displaySpace = space ? { ...space, label: spaceListing.listing?.space_label ?? space.label } : null;
  // Keep the pending state until a fresh listing confirms the completed selection.
  const spaceListingRef = useRef(spaceListing.listing);
  spaceListingRef.current = spaceListing.listing;
  const [listingBeforeSpaceAdd, setListingBeforeSpaceAdd] = useState<SpaceContextListing | null | undefined>(undefined);
  const spaceAdd = useLibraryOperation(client, () => setListingBeforeSpaceAdd(spaceListingRef.current));
  const [spaceAddRequest, setSpaceAddRequest] = useState<{ itemId: string; target: SpaceTarget } | null>(null);
  const [spaceRemoving, setSpaceRemoving] = useState<{ itemId: string; target: SpaceTarget; listing: SpaceContextListing | null; finished: boolean } | null>(null);
  const [spaceRemoveError, setSpaceRemoveError] = useState<{ itemId: string; target: SpaceTarget; message: string } | null>(null);
  useEffect(() => {
    if (spaceRemoving?.finished && (spaceListing.listing !== spaceRemoving.listing || spaceListing.status === "error")) setSpaceRemoving(null);
  }, [spaceListing.listing, spaceListing.status, spaceRemoving]);
  const startSpaceAdd = (item: LibraryItemSummary) => {
    if (!spaceLive || spaceAdd.starting || spaceAdd.running || spaceRemoving) return;
    const target = spaceLive.target;
    setSpaceAddRequest({ itemId: item.item_id, target });
    setListingBeforeSpaceAdd(undefined);
    void spaceAdd.start(() => client.librarySpaceAdd({ target, item_ids: [item.item_id] }));
  };
  const startSpaceRemove = async (item: LibraryItemSummary) => {
    if (!spaceLive || spaceRemoving || spaceAdd.starting || spaceAdd.running) return;
    const request = { itemId: item.item_id, target: spaceLive.target, listing: spaceListing.listing, finished: false };
    setSpaceRemoving(request); setSpaceRemoveError(null);
    try {
      await client.librarySpaceRemove({ target: request.target, item_ids: [item.item_id] });
      setSpaceRemoving({ ...request, listing: spaceListingRef.current, finished: true });
      announceLibraryChanged(); spaceListing.reload();
    } catch (cause) {
      setSpaceRemoveError({ ...request, message: cause instanceof Error ? cause.message : "The selection could not be removed." });
      setSpaceRemoving(null);
    }
  };
  const itemSpace = (item: LibraryItemSummary): ItemSpaceState | null => {
    const listing = spaceListing.listing;
    if (!spaceLive || !listing) return null;
    const selected = listing.items.some((candidate) => candidate.item_id === item.item_id);
    const mine = spaceAddRequest?.itemId === item.item_id && sameSpaceTarget(spaceAddRequest.target, spaceLive.target);
    const unconfirmed = listingBeforeSpaceAdd !== undefined && listing === listingBeforeSpaceAdd && spaceListing.status === "error";
    const settling = listingBeforeSpaceAdd !== undefined && listing === listingBeforeSpaceAdd && spaceListing.status !== "error";
    const stopped = mine && spaceAdd.operation?.finished ? spaceAdd.operation.phases.find((phase) => phase.phase === "space" && (phase.state === "failed" || phase.state === "cancelled")) : null;
    const removing = spaceRemoving?.itemId === item.item_id && sameSpaceTarget(spaceRemoving.target, spaceLive.target);
    const removeError = spaceRemoveError?.itemId === item.item_id && sameSpaceTarget(spaceRemoveError.target, spaceLive.target) ? spaceRemoveError.message : null;
    return {
      label: listing.space_label,
      selected,
      adding: mine && (spaceAdd.starting || spaceAdd.running || settling),
      busy: Boolean(removing || (mine && (spaceAdd.starting || spaceAdd.running || settling))),
      error: selected ? removeError : mine ? spaceAdd.error ?? stopped?.error?.message ?? (unconfirmed ? spaceListing.error : null) : null,
      onAdd: () => startSpaceAdd(item),
      onRemove: () => void startSpaceRemove(item),
    };
  };
  return { spaceLive, spaceListing, displaySpace, spaceRemoving, spaceAdd, itemSpace };
}
