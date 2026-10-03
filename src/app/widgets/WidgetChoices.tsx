import { useId, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { WidgetChoicesSpec, WidgetSelectRequest, WidgetSummary } from "../../protocol/generated/v1";

type ChoiceRequest = Omit<WidgetSelectRequest, "value"> & {
  value: Extract<WidgetSelectRequest["value"], { type: "choice" }>;
};

export type WidgetChoicesProps = {
  client: CockpitClient;
  widget: WidgetSummary;
  spec: WidgetChoicesSpec;
  inputBlocked: boolean;
};

export function WidgetChoices(props: WidgetChoicesProps) {
  const { key, revision } = props.widget;
  return <ChoicesContent key={JSON.stringify([key.session_id, key.tab_id, key.id, revision])} {...props} />;
}

function ChoicesContent({ client, widget, spec, inputBlocked }: WidgetChoicesProps) {
  const id = useId();
  const busy = useRef(false);
  const [pending, setPending] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [failure, setFailure] = useState<{ request: ChoiceRequest; message: string } | null>(null);

  const select = async (request: ChoiceRequest) => {
    if (busy.current || inputBlocked || window.document.body.classList.contains("is-pane-dragging")) return;
    busy.current = true;
    setPending(request.value.choice_id);
    setFailure(null);
    try {
      await client.widgetSelect(request);
      setSelected(request.value.choice_id);
    } catch (error) {
      setFailure({ request, message: error instanceof Error ? error.message : "Could not store this choice." });
    } finally {
      busy.current = false;
      setPending(null);
    }
  };

  return <div className="widget-choices" data-input-blocked={inputBlocked || undefined}>
    <fieldset role="radiogroup" aria-labelledby={`${id}-prompt`} aria-busy={pending !== null} aria-describedby={failure ? `${id}-error` : undefined}
      disabled={inputBlocked || pending !== null}>
      <legend id={`${id}-prompt`}>{spec.prompt || widget.title}</legend>
      {spec.choices.map((choice, index) => <label className="widget-choice" key={choice.id}>
        <input type="radio" name={id} value={choice.id} checked={(pending ?? selected) === choice.id}
          aria-describedby={choice.detail ? `${id}-detail-${index}` : undefined}
          onChange={() => { void select({ key: widget.key, revision: widget.revision, value: { type: "choice", choice_id: choice.id } }); }} />
        <span><strong>{choice.label}</strong>{choice.detail ? <span id={`${id}-detail-${index}`} className="widget-choice-detail">{choice.detail}</span> : null}</span>
      </label>)}
    </fieldset>
    {failure ? <div id={`${id}-error`} className="widget-choice-error" role="alert">
      <span>{failure.message}</span>
      <button type="button" disabled={inputBlocked || pending !== null} onClick={() => { void select(failure.request); }}>Retry</button>
    </div> : null}
  </div>;
}
