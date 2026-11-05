import type { InboxReceipt } from "../../protocol/generated/v1";

/** What a stored/read/acked receipt does and does not say: transport state, never that a model consumed the message. */
export const deliveryLabel = (stage: InboxReceipt["stage"]) => stage === "stored" ? "waiting to be picked up" : stage === "read" ? "picked up" : "captured, processing not confirmed";

/** The receipt line under a sent answer. */
export const answerReceiptLine = (stage: InboxReceipt["stage"]) => stage === "stored" ? "Stored · waiting to be picked up" : stage === "read" ? "Delivered · picked up" : "Delivered · received";
