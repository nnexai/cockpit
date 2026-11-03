// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import { CockpitClientError } from "../../client/CockpitClient";
import type { MessageImage } from "../../protocol/generated/v1";
import { InboxImages } from "./InboxImages";

const images: MessageImage[] = [
  { image_id: "first", mime_type: "image/png", width: 120, height: 80, bytes: 100 },
  { image_id: "second", mime_type: "image/jpeg", width: 80, height: 120, bytes: 200 },
];

it("loads only structured bytes, navigates the modal, and releases object URLs", async () => {
  let nextUrl = 0;
  const create = vi.fn(() => `blob:test-${++nextUrl}`);
  const revoke = vi.fn();
  const originalCreate = URL.createObjectURL, originalRevoke = URL.revokeObjectURL;
  URL.createObjectURL = create; URL.revokeObjectURL = revoke;
  const show = HTMLDialogElement.prototype.showModal, close = HTMLDialogElement.prototype.close;
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute("open"); };
  const read = vi.fn(async ({ image_id }: { image_id: string }) => ({ image: images.find(image => image.image_id === image_id)!, data_base64: "AA==" }));
  const client = { orchestrationImageRead: read } as unknown as CockpitClient;
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<InboxImages client={client} sessionId="session" images={images} label="Your request images" />));
    expect(host.querySelector(".inbox-images-label")?.textContent).toBe("Your request images");
    expect(read).toHaveBeenCalledTimes(2);
    expect(read.mock.calls[0][0]).toEqual({ session_id: "session", image_id: "first" });
    expect([...host.querySelectorAll("img")].every(image => image.getAttribute("src")?.startsWith("blob:"))).toBe(true);
    const thumbnail = host.querySelector<HTMLButtonElement>('[aria-label="View image 1 of 2"]')!;
    thumbnail.focus();
    await act(async () => thumbnail.click());
    expect(host.querySelector('dialog[open]')?.getAttribute("aria-label")).toBe("Image 1 of 2");
    expect(document.activeElement?.textContent).toBe("Close");
    await act(async () => host.querySelector("dialog")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })));
    expect(host.querySelector('dialog[open]')?.getAttribute("aria-label")).toBe("Image 2 of 2");
    await act(async () => host.querySelector("dialog")!.dispatchEvent(new Event("cancel", { bubbles: true, cancelable: true })));
    expect(host.querySelector("dialog")).toBeNull();
    expect(document.activeElement).toBe(thumbnail);
  } finally {
    await act(async () => root.unmount());
    expect(revoke).toHaveBeenCalledTimes(create.mock.calls.length);
    host.remove(); URL.createObjectURL = originalCreate; URL.revokeObjectURL = originalRevoke;
    HTMLDialogElement.prototype.showModal = show; HTMLDialogElement.prototype.close = close;
  }
});

it("offers retry for transient failures but not images that are no longer available", async () => {
  const originalCreate = URL.createObjectURL, originalRevoke = URL.revokeObjectURL;
  URL.createObjectURL = vi.fn(() => "blob:retry"); URL.revokeObjectURL = vi.fn();
  const read = vi.fn().mockRejectedValueOnce(new Error("offline"))
    .mockRejectedValueOnce(new CockpitClientError("http_error", "missing", { operationCode: "image_not_found" }))
    .mockResolvedValue({ image: images[0], data_base64: "AA==" });
  const client = { orchestrationImageRead: read } as unknown as CockpitClient;
  const host = document.createElement("div"); const root = createRoot(host);
  try {
    await act(async () => root.render(<InboxImages client={client} sessionId="session" images={images} label="Images" />));
    expect(host.textContent).toContain("Image no longer available");
    expect(host.querySelectorAll(".inbox-image-error button")).toHaveLength(1);
    expect(host.querySelector(".inbox-image-open")).toBeNull();
    expect(host.querySelector(".inbox-image-error button")?.closest(".inbox-image-open")).toBeNull();
    await act(async () => host.querySelector<HTMLButtonElement>(".inbox-image-error button")!.click());
    expect(read).toHaveBeenCalledTimes(3);
    expect(host.querySelector("img")?.getAttribute("src")).toBe("blob:retry");
  } finally {
    await act(async () => root.unmount());
    URL.createObjectURL = originalCreate; URL.revokeObjectURL = originalRevoke;
  }
});
