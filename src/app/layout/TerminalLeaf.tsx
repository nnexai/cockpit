import { useEffect, useState } from "react";
import { TerminalPane, type TerminalPaneProps } from "../TerminalPane";

export type TerminalLeafProps = TerminalPaneProps & { onPrepared(): void };

/** Attachment lifetime follows painted membership, never local selection. */
export function TerminalLeaf({ onPrepared, onReady, ...props }: TerminalLeafProps) {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    if (ready && !props.deferAttachment) onPrepared();
  }, [ready, props.deferAttachment, onPrepared]);
  return <div className="terminal-surface"><TerminalPane {...props} onReady={() => { setReady(true); onReady?.(); }} /></div>;
}
