import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { HealthStatus } from "./components/HealthStatus";
import { Onboarding } from "./components/Onboarding";
import {
  PromiseInbox,
  type PromiseInboxInteractionState,
} from "./components/PromiseInbox";
import { Settings } from "./components/Settings";

type View = "promises" | "phase0" | "settings" | "health";

type Phase0Rule = {
  id: number;
  app_match: string;
  reminder_text: string;
  enabled: boolean;
};

type KillGateStatus = "pending_user" | "passed" | "failed";
type KillGate = { id: string; status: KillGateStatus; notes: string };
type QuickCaptureResult = { capture_id: string; promise_id: number };

const NAV_ITEMS: ReadonlyArray<{ id: View; label: string }> = [
  { id: "promises", label: "Promises" },
  { id: "phase0", label: "Focus rules" },
  { id: "settings", label: "Settings" },
  { id: "health", label: "Health" },
];

const CLEAN_PROMISE_INTERACTION: PromiseInboxInteractionState = {
  busy: false,
  dirty: false,
};

function mergePhase0Rules(
  loaded: Phase0Rule[],
  current: Phase0Rule[],
): Phase0Rule[] {
  const byId = new Map(loaded.map((rule) => [rule.id, rule]));
  for (const rule of current) byId.set(rule.id, rule);
  return [...byId.values()].sort((left, right) => left.id - right.id);
}

export default function App() {
  const params = new URLSearchParams(window.location.search);
  const isQuick = params.get("window") === "quick";
  const [onboarded, setOnboarded] = useState<boolean | null>(null);
  const [view, setView] = useState<View>("promises");
  const [promiseInteraction, setPromiseInteraction] =
    useState<PromiseInboxInteractionState>(CLEAN_PROMISE_INTERACTION);
  const [rules, setRules] = useState<Phase0Rule[]>([]);
  const [appMatch, setAppMatch] = useState("chrome.exe");
  const [reminder, setReminder] = useState("Follow up on the invoice");
  const [phase0Pending, setPhase0Pending] = useState(false);
  const [phase0Message, setPhase0Message] = useState<string | null>(null);
  const [phase0Error, setPhase0Error] = useState<string | null>(null);
  const [quickText, setQuickText] = useState("");
  const [quickCaptureId, setQuickCaptureId] = useState(
    () => `manual-${globalThis.crypto.randomUUID()}`,
  );
  const [quickPending, setQuickPending] = useState(false);
  const [quickMessage, setQuickMessage] = useState<string | null>(null);
  const [quickError, setQuickError] = useState<string | null>(null);
  const [gates, setGates] = useState<KillGate[]>([]);
  const [gatePendingId, setGatePendingId] = useState<string | null>(null);
  const [gateMessage, setGateMessage] = useState<string | null>(null);
  const [gateError, setGateError] = useState<string | null>(null);
  const quickInFlight = useRef(false);
  const quickCapturePayload = useRef<string | null>(null);
  const phase0InFlight = useRef(false);
  const gateInFlight = useRef<string | null>(null);
  const gateMutationVersion = useRef(0);

  useEffect(() => {
    if (isQuick) return;
    const initialGateVersion = gateMutationVersion.current;
    void invoke<string | null>("load_setting", {
      key: "onboarding_completed_at",
    })
      .then((value) => setOnboarded(Boolean(value)))
      .catch(() => setOnboarded(false));
    void invoke<Phase0Rule[]>("list_phase0")
      .then((loaded) =>
        setRules((current) => mergePhase0Rules(loaded, current)),
      )
      .catch(() => setRules((current) => current));
    void invoke<KillGate[]>("list_kill_gates")
      .then((loaded) => {
        if (gateMutationVersion.current === initialGateVersion) {
          setGates(loaded);
        }
      })
      .catch(() => {
        // The Health view reports persistent local backend failures.
      });
  }, [isQuick]);

  const saveQuickCapture = async () => {
    if (quickInFlight.current) return;
    const submittedDraft = quickText;
    const text = submittedDraft.trim();
    setQuickMessage(null);
    setQuickError(null);
    if (!text) {
      setQuickError("Type a promise before saving.");
      return;
    }

    let captureId = quickCaptureId;
    if (
      quickCapturePayload.current !== null &&
      quickCapturePayload.current !== text
    ) {
      captureId = `manual-${globalThis.crypto.randomUUID()}`;
      setQuickCaptureId(captureId);
    }
    quickCapturePayload.current = text;
    quickInFlight.current = true;
    setQuickPending(true);
    try {
      const result = await invoke<QuickCaptureResult>("quick_capture", {
        captureId,
        text,
      });
      setQuickText((current) => (current === submittedDraft ? "" : current));
      quickCapturePayload.current = null;
      setQuickCaptureId(`manual-${globalThis.crypto.randomUUID()}`);
      setQuickMessage(`Saved locally as promise ${result.promise_id}.`);
    } catch (reason: unknown) {
      setQuickError(String(reason));
    } finally {
      quickInFlight.current = false;
      setQuickPending(false);
    }
  };

  const addPhase0Rule = async () => {
    if (phase0InFlight.current) return;
    const submittedApp = appMatch.trim();
    const submittedReminder = reminder.trim();
    setPhase0Message(null);
    setPhase0Error(null);
    if (!submittedApp || !submittedReminder) {
      setPhase0Error("Enter both an app and a reminder.");
      return;
    }
    if (
      rules.some(
        (rule) =>
          rule.app_match.toLocaleLowerCase() ===
            submittedApp.toLocaleLowerCase() &&
          rule.reminder_text === submittedReminder,
      )
    ) {
      setPhase0Error("That focus rule already exists.");
      return;
    }

    phase0InFlight.current = true;
    setPhase0Pending(true);
    try {
      const id = await invoke<number>("add_phase0", {
        appMatch: submittedApp,
        reminderText: submittedReminder,
      });
      setRules((current) =>
        current.some((rule) => rule.id === id)
          ? current
          : [
              ...current,
              {
                id,
                app_match: submittedApp,
                reminder_text: submittedReminder,
                enabled: true,
              },
            ],
      );
      setPhase0Message("Focus rule added locally.");
    } catch (reason: unknown) {
      setPhase0Error(String(reason));
    } finally {
      phase0InFlight.current = false;
      setPhase0Pending(false);
    }
  };

  const requestView = (nextView: View) => {
    if (nextView === view) return;
    if (view === "promises") {
      if (promiseInteraction.busy) return;
      if (
        promiseInteraction.dirty &&
        !window.confirm(
          "Discard the unsaved promise changes and leave Promises?",
        )
      ) {
        return;
      }
    }
    setView(nextView);
  };

  const recordGate = async (gate: KillGate, status: "passed" | "failed") => {
    if (gateInFlight.current !== null) return;
    const evidence = window.prompt(
      `Record local evidence for ${gate.id} (${status}).`,
      "",
    );
    if (!evidence?.trim()) return;

    gateInFlight.current = gate.id;
    gateMutationVersion.current += 1;
    setGatePendingId(gate.id);
    setGateMessage(null);
    setGateError(null);
    try {
      const updated = await invoke<KillGate[]>("record_kill_gate", {
        id: gate.id,
        status,
        notes: evidence.trim(),
      });
      setGates(updated);
      setGateMessage(`${gate.id} recorded as ${status}.`);
    } catch (reason: unknown) {
      setGateError(String(reason));
    } finally {
      if (gateInFlight.current === gate.id) {
        gateInFlight.current = null;
        setGatePendingId(null);
      }
    }
  };

  if (isQuick) {
    return (
      <main className="quick" aria-busy={quickPending}>
        <p className="eyebrow">Callback</p>
        <h1>Quick capture</h1>
        <p>
          Type a promise. Callback never reads selected text from other apps.
        </p>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void saveQuickCapture();
          }}
        >
          <textarea
            aria-label="Promise"
            value={quickText}
            disabled={quickPending}
            onChange={(event) => {
              setQuickText(event.target.value);
              setQuickMessage(null);
              setQuickError(null);
            }}
          />
          <button type="submit" disabled={quickPending || !quickText.trim()}>
            {quickPending ? "Saving…" : "Save locally"}
          </button>
        </form>
        {quickMessage ? (
          <p className="success" role="status" aria-live="polite">
            {quickMessage}
          </p>
        ) : null}
        {quickError ? (
          <p className="error" role="alert">
            {quickError}
          </p>
        ) : null}
      </main>
    );
  }

  if (onboarded === null) {
    return <main className="boot">Loading local state…</main>;
  }

  if (!onboarded) {
    return <Onboarding onDone={() => setOnboarded(true)} />;
  }

  return (
    <div className="shell">
      <aside className="app-sidebar">
        <div className="brand-block">
          <p className="eyebrow">Callback</p>
          <p>Promises, returned in context.</p>
        </div>
        <nav aria-label="Callback sections">
          {NAV_ITEMS.map((item) => (
            <button
              key={item.id}
              type="button"
              className={view === item.id ? "active" : ""}
              aria-current={view === item.id ? "page" : undefined}
              disabled={view === "promises" && promiseInteraction.busy}
              onClick={() => requestView(item.id)}
            >
              {item.label}
            </button>
          ))}
        </nav>
        <p className="local-note">Local-only · stored on this PC</p>
      </aside>
      <main className="app-main">
        {view === "promises" ? (
          <PromiseInbox onInteractionStateChange={setPromiseInteraction} />
        ) : null}
        {view === "phase0" ? (
          <section className="content-section" aria-busy={phase0Pending}>
            <p className="eyebrow">Context prototype</p>
            <h1>Focus rules</h1>
            <p>
              When I focus an app, show a hardcoded reminder. No extraction.
            </p>
            <ul>
              {rules.map((rule) => (
                <li key={rule.id}>
                  <strong>{rule.app_match}</strong> → {rule.reminder_text}
                </li>
              ))}
            </ul>
            <label>
              App
              <input
                value={appMatch}
                disabled={phase0Pending}
                onChange={(event) => {
                  setAppMatch(event.target.value);
                  setPhase0Message(null);
                  setPhase0Error(null);
                }}
              />
            </label>
            <label>
              Reminder
              <input
                value={reminder}
                disabled={phase0Pending}
                onChange={(event) => {
                  setReminder(event.target.value);
                  setPhase0Message(null);
                  setPhase0Error(null);
                }}
              />
            </label>
            <button
              type="button"
              disabled={phase0Pending || !appMatch.trim() || !reminder.trim()}
              onClick={() => void addPhase0Rule()}
            >
              {phase0Pending ? "Adding…" : "Add rule"}
            </button>
            {phase0Message ? (
              <p className="success" role="status" aria-live="polite">
                {phase0Message}
              </p>
            ) : null}
            {phase0Error ? (
              <p className="error" role="alert">
                {phase0Error}
              </p>
            ) : null}
            <h2>Evidence gates</h2>
            <p>
              Gates must pass in order. Extracted promises remain
              notification-silent until the 300-message precision gate is
              recorded as passed.
            </p>
            <ul>
              {gates.map((gate) => (
                <li key={gate.id}>
                  <strong>{gate.id}</strong> ({gate.status}): {gate.notes}
                  <div className="row">
                    <button
                      type="button"
                      disabled={
                        gatePendingId !== null || gate.status === "passed"
                      }
                      onClick={() => void recordGate(gate, "passed")}
                    >
                      {gatePendingId === gate.id
                        ? "Recording…"
                        : "Record passed"}
                    </button>
                    <button
                      type="button"
                      disabled={
                        gatePendingId !== null || gate.status === "failed"
                      }
                      onClick={() => void recordGate(gate, "failed")}
                    >
                      {gatePendingId === gate.id
                        ? "Recording…"
                        : "Record failed"}
                    </button>
                  </div>
                </li>
              ))}
            </ul>
            {gateMessage ? (
              <p className="success" role="status" aria-live="polite">
                {gateMessage}
              </p>
            ) : null}
            {gateError ? (
              <p className="error" role="alert">
                {gateError}
              </p>
            ) : null}
          </section>
        ) : null}
        {view === "settings" ? <Settings /> : null}
        {view === "health" ? <HealthStatus /> : null}
      </main>
    </div>
  );
}
