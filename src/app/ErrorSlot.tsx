import type { ReactNode } from "react";
import { UiIcon } from "./UiIcon";
import "./errorSlot.css";

/** Reserved status chrome: messages never insert a new row above the work. */
export function ErrorSlot({ placement, message, error = true, actions, className = "" }: {
  placement: "dialog" | "pane";
  message?: ReactNode;
  error?: boolean;
  actions?: ReactNode;
  className?: string;
}) {
  const empty = !message;
  return <div className={`error-slot error-slot-${placement}${error ? " is-error" : ""} ${className}`}>
    <div className="error-slot-content" style={empty ? { visibility: "hidden" } : undefined}>
      <span className="error-slot-message" role={empty ? undefined : error ? "alert" : "status"}>
        {message ? <UiIcon name={error ? "info" : "refresh"} /> : null}
        <span tabIndex={empty ? undefined : 0} title={typeof message === "string" ? message : undefined}>{message}</span>
      </span>
      {actions ? <span className="error-slot-actions">{actions}</span> : null}
    </div>
  </div>;
}
