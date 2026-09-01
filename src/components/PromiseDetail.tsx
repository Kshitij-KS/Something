import { type RefObject, useEffect, useRef, useState } from "react";

import {
  formatMoment,
  formatRelative,
  fromDateTimeLocal,
  resolveTimeZone,
  sourceLabel,
  statusLabel,
  toDateTimeLocal,
  type CallbackPromiseDetail,
  type PromiseInboxAction,
  type PromiseTrigger,
} from "../promises";

type PromiseDetailProps = {
  detail: CallbackPromiseDetail;
  pending: boolean;
  editorFocusRef: RefObject<HTMLTextAreaElement | null>;
  onAction: (
    action: PromiseInboxAction,
    snoozeUntil?: number | null,
  ) => Promise<void>;
  onDirtyChange: (dirty: boolean) => void;
  onSave: (
    text: string,
    deadline: number | null,
    deadlineTimezone: string | null,
  ) => Promise<void>;
};

type FormError = {
  field: "text" | "deadline";
  message: string;
};

const FORM_ERROR_ID = "promise-editor-error";

function triggerLabel(trigger: PromiseTrigger): string {
  if (trigger.kind === "app_ctx_focus") {
    return `This conversation · ${trigger.match_value}`;
  }
  if (trigger.kind === "app_focus") {
    return `When ${sourceLabel(trigger.match_value)} is in focus`;
  }
  if (trigger.kind === "deadline") return "When its deadline arrives";
  if (trigger.kind === "manual")
    return "Manual promise — add a deadline to return it";
  return trigger.match_value;
}

function sourceDescription(detail: CallbackPromiseDetail): string {
  const source = sourceLabel(detail.source_app);
  return detail.recipient ? `${source} · ${detail.recipient}` : source;
}

export function PromiseDetail({
  detail,
  pending,
  editorFocusRef,
  onAction,
  onDirtyChange,
  onSave,
}: PromiseDetailProps) {
  const deadlineTimezone = resolveTimeZone(detail.deadline_tz);
  const initialDeadline = toDateTimeLocal(detail.deadline, deadlineTimezone);
  const [draftText, setDraftText] = useState(detail.text);
  const [draftDeadline, setDraftDeadline] = useState(initialDeadline);
  const [formError, setFormError] = useState<FormError | null>(null);
  const deadlineInputRef = useRef<HTMLInputElement>(null);

  const editable = ["open", "snoozed", "review"].includes(detail.status);
  const parsedDeadline = fromDateTimeLocal(draftDeadline, deadlineTimezone);
  const deadlineChanged = draftDeadline !== initialDeadline;
  const changed = draftText.trim() !== detail.text || deadlineChanged;

  useEffect(() => {
    onDirtyChange(changed);
    return () => onDirtyChange(false);
  }, [changed, onDirtyChange]);

  const save = async () => {
    setFormError(null);
    if (!draftText.trim()) {
      setFormError({
        field: "text",
        message: "Promise text cannot be empty.",
      });
      editorFocusRef.current?.focus();
      return;
    }
    if (deadlineChanged && draftDeadline && parsedDeadline === null) {
      setFormError({
        field: "deadline",
        message:
          "Choose an unambiguous deadline. Daylight-saving gaps and repeated times cannot be used.",
      });
      deadlineInputRef.current?.focus();
      return;
    }
    const deadline = deadlineChanged
      ? parsedDeadline
      : (detail.deadline ?? null);
    await onSave(
      draftText.trim(),
      deadline,
      deadline === null ? null : deadlineTimezone,
    );
  };

  const reject = async () => {
    if (
      window.confirm(
        "Mark this as not a promise? Callback will also learn to avoid similar wording.",
      )
    ) {
      await onAction("not_a_promise");
    }
  };

  return (
    <article
      className="promise-detail"
      aria-busy={pending}
      aria-labelledby="promise-detail-title"
    >
      <header className="promise-detail-header">
        <div>
          <p className="detail-kicker">Promise {detail.id}</p>
          <h2 id="promise-detail-title">{detail.text}</h2>
        </div>
        <span className={`status-badge status-${detail.status}`}>
          {statusLabel(detail.status)}
        </span>
      </header>

      <ol className="callback-trace" aria-label="Promise callback path">
        <li>
          <span className="trace-step">Said</span>
          <strong>{sourceDescription(detail)}</strong>
          <small>
            {formatMoment(detail.sent_at)} · {formatRelative(detail.sent_at)}
          </small>
        </li>
        <li>
          <span className="trace-step">Kept locally</span>
          <strong>
            {detail.source_app === "manual"
              ? "Saved by you"
              : `${Math.round(detail.confidence * 100)}% detection confidence`}
          </strong>
          <small>Score {detail.score} · never sent to a Callback server</small>
        </li>
        <li>
          <span className="trace-step">Returns when</span>
          <strong>
            {detail.deadline
              ? `${formatRelative(detail.deadline)} · ${formatMoment(detail.deadline, deadlineTimezone)}`
              : detail.triggers[0]
                ? triggerLabel(detail.triggers[0])
                : "A return context is added"}
          </strong>
          <small>
            {detail.surface_count === 0
              ? "Not surfaced yet"
              : `Surfaced ${detail.surface_count} ${detail.surface_count === 1 ? "time" : "times"}`}
          </small>
        </li>
      </ol>

      {editable ? (
        <section
          className="promise-editor"
          aria-labelledby="edit-promise-heading"
        >
          <div className="section-heading-row">
            <div>
              <p className="detail-kicker">Edit</p>
              <h3 id="edit-promise-heading">What should come back?</h3>
            </div>
            {changed ? <span className="unsaved-marker">Unsaved</span> : null}
          </div>
          <label>
            Promise
            <textarea
              ref={editorFocusRef}
              value={draftText}
              aria-describedby={
                formError?.field === "text" ? FORM_ERROR_ID : undefined
              }
              aria-invalid={formError?.field === "text"}
              disabled={pending}
              onChange={(event) => {
                const nextText = event.target.value;
                setDraftText(nextText);
                onDirtyChange(
                  nextText.trim() !== detail.text || deadlineChanged,
                );
                setFormError(null);
              }}
              rows={4}
              maxLength={10_000}
            />
          </label>
          <div className="deadline-field">
            <label>
              Deadline ({deadlineTimezone})
              <input
                ref={deadlineInputRef}
                type="datetime-local"
                value={draftDeadline}
                aria-describedby={
                  formError?.field === "deadline" ? FORM_ERROR_ID : undefined
                }
                aria-invalid={formError?.field === "deadline"}
                disabled={pending}
                onChange={(event) => {
                  const nextDeadline = event.target.value;
                  setDraftDeadline(nextDeadline);
                  onDirtyChange(
                    draftText.trim() !== detail.text ||
                      nextDeadline !== initialDeadline,
                  );
                  setFormError(null);
                }}
              />
            </label>
            <button
              type="button"
              className="button-quiet"
              disabled={pending || !draftDeadline}
              onClick={() => {
                setDraftDeadline("");
                onDirtyChange(
                  draftText.trim() !== detail.text || initialDeadline !== "",
                );
              }}
            >
              Clear deadline
            </button>
          </div>
          <button
            type="button"
            disabled={pending || !changed || !draftText.trim()}
            onClick={() => void save()}
          >
            {pending ? "Saving…" : "Save changes"}
          </button>
          {formError ? (
            <p id={FORM_ERROR_ID} className="error" role="alert">
              {formError.message}
            </p>
          ) : null}
        </section>
      ) : null}

      <section
        className="detail-section"
        aria-labelledby="return-context-heading"
      >
        <p className="detail-kicker">Return context</p>
        <h3 id="return-context-heading">Why Callback will bring this back</h3>
        {detail.triggers.length > 0 ? (
          <ul className="trigger-list">
            {detail.triggers.map((trigger) => (
              <li key={`${trigger.kind}:${trigger.match_value}`}>
                <span>{triggerLabel(trigger)}</span>
                <small>Priority {trigger.priority}</small>
              </li>
            ))}
          </ul>
        ) : (
          <p className="empty">
            {detail.status === "review"
              ? "Review items receive return contexts after you promote them."
              : "No focus context is attached. Add a deadline so this promise can return."}
          </p>
        )}
        {detail.source_ctx ? (
          <p className="context-code">
            Captured context <code>{detail.source_ctx}</code>
          </p>
        ) : null}
      </section>

      <section
        className="detail-section"
        aria-labelledby="promise-actions-heading"
      >
        <p className="detail-kicker">Next action</p>
        <h3 id="promise-actions-heading">Move this promise forward</h3>
        <div className="promise-actions">
          {detail.status === "review" ? (
            <>
              <button
                type="button"
                disabled={pending}
                onClick={() => void onAction("promote")}
              >
                Promote to open
              </button>
              <button
                type="button"
                className="button-danger"
                disabled={pending}
                onClick={() => void reject()}
              >
                Not a promise
              </button>
            </>
          ) : null}
          {detail.status === "open" ? (
            <>
              <button
                type="button"
                disabled={pending}
                onClick={() => void onAction("done")}
              >
                Mark done
              </button>
              <button
                type="button"
                className="button-secondary"
                disabled={pending}
                onClick={() => void onAction("snooze")}
              >
                Snooze 1 hour
              </button>
              <button
                type="button"
                className="button-quiet"
                disabled={pending}
                onClick={() => void onAction("ignore")}
              >
                Skip this reminder
              </button>
              <button
                type="button"
                className="button-danger"
                disabled={pending}
                onClick={() => void reject()}
              >
                Not a promise
              </button>
            </>
          ) : null}
          {detail.status === "snoozed" ? (
            <>
              <button
                type="button"
                disabled={pending}
                onClick={() => void onAction("resume")}
              >
                Resume now
              </button>
              <button
                type="button"
                className="button-danger"
                disabled={pending}
                onClick={() => void reject()}
              >
                Not a promise
              </button>
            </>
          ) : null}
        </div>
        {detail.status === "open" ? (
          <p className="action-note">
            Skip keeps it open. Callback archives it after three skips.
          </p>
        ) : null}
        {["done", "dismissed", "archived"].includes(detail.status) ? (
          <p className="empty">
            This resolved record is read-only. Safe reopening will arrive with a
            reversible activity history.
          </p>
        ) : null}
      </section>

      <footer className="detail-facts">
        <span>Created {formatMoment(detail.created_at)}</span>
        {detail.last_shown_at ? (
          <span>Last surfaced {formatMoment(detail.last_shown_at)}</span>
        ) : null}
        {detail.deadline ? <span>Timezone {deadlineTimezone}</span> : null}
      </footer>
    </article>
  );
}
