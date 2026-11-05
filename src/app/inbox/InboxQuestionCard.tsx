import type { CockpitClient } from "../../client/CockpitClient";
import type { InboxQuestion } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { InboxFeedback } from "./InboxFeedback";
import { InboxImages } from "./InboxImages";
import { Time } from "./InboxTime";
import { questionKey } from "./inboxDrafts";

type Props = {
  question: InboxQuestion;
  label: string;
  imagesLabel: string;
  index: number;
  total: number;
  selected: boolean;
  /** The owner replaced the question this draft was written for. */
  updated: boolean;
  client: CockpitClient;
  sessionId: string;
  onSelect(pointer: boolean): void;
};

/** The single copy of an open question, in the timeline at the point it was asked. */
export function InboxQuestionCard({ question, label, imagesLabel, index, total, selected, updated, client, sessionId, onSelect }: Props) {
  return <section className="inbox-question" data-selected={selected} data-open-question={questionKey(question)} tabIndex={-1} aria-label={`Open question ${index + 1} of ${total} from ${label}`}>
    <header className="inbox-card-h"><strong>{label} asks{total > 1 ? ` · Q${index + 1}` : ""}</strong><span className="inbox-pill">Waiting for you</span><span className="inbox-sp" /><Time value={question.asked_at} /></header>
    {updated && <p className="inbox-q-updated" role="note"><UiIcon name="edit" />Updated by {label}. Your earlier draft was carried over to this version.</p>}
    <div className="inbox-q-text"><InboxFeedback text={question.text.text} /></div>
    <InboxImages client={client} sessionId={sessionId} images={question.text.images ?? []} label={imagesLabel} />
    <div className="inbox-q-act">{selected
      ? <span className="inbox-pill is-on"><UiIcon name="check" />Answering this</span>
      : <button type="button" className="inbox-btn inbox-q-pick" onClick={event => onSelect(event.detail > 0)}><UiIcon name="reply" />Answer this</button>}</div>
  </section>;
}
