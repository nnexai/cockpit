import { useRef, useState } from "react";
import type { SupervisorDialogState } from "./SupervisorDialogs";
import type { AttentionTier } from "./attention";
import type { OrchestrationSnapshot } from "../../protocol/generated/v1";
import type { SupervisorViewState } from "./supervisorViewTypes";

export function useSupervisorViewState(): SupervisorViewState {
  const [dialog, setDialog] = useState<SupervisorDialogState | null>(null);
  const [terminalError, setTerminalError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [hoverSpace, setHoverSpace] = useState<string | null>(null);
  const [hoverRun, setHoverRun] = useState<string | null>(null);
  const [detailSection, setDetailSection] = useState<"overview" | "activity" | "actions">("overview");
  const [attentionOpen, setAttentionOpen] = useState(false);
  const [focusTier, setFocusTier] = useState<AttentionTier | null>(null);
  const [focusNotice, setFocusNotice] = useState<string | null>(null);
  const [inlineQueueCap, setInlineQueueCap] = useState<number | null>(null);
  const [graphViewportHeight, setGraphViewportHeight] = useState<number | null>(null);
  const opened = useRef(false);
  const initialRootSnapshot = useRef<OrchestrationSnapshot | null>(null);
  return { dialog, setDialog, terminalError, setTerminalError, notice, setNotice, hoverSpace, setHoverSpace, hoverRun, setHoverRun, detailSection, setDetailSection, attentionOpen, setAttentionOpen, focusTier, setFocusTier, focusNotice, setFocusNotice, inlineQueueCap, setInlineQueueCap, graphViewportHeight, setGraphViewportHeight, opened, initialRootSnapshot };
}
