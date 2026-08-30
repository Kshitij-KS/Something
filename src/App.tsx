import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { HealthStatus } from "./components/HealthStatus";
import { Onboarding } from "./components/Onboarding";
import { ReviewQueue } from "./components/ReviewQueue";
import { Settings } from "./components/Settings";

type View = "review" | "phase0" | "settings" | "health";

type Phase0Rule = {
  id: number;
  app_match: string;
  reminder_text: string;
  enabled: boolean;
};

type KillGateStatus = "pending_user" | "passed" | "failed";
type KillGate = { id: string; status: KillGateStatus; notes: string };
type QuickCaptureResult = { capture_id: string; promise_id: number };

export default function App() {
  const params = new URLSearchParams(window.location.search);
  const isQuick = params.get("window") === "quick";
  const [onboarded, setOnboarded] = useState<boolean | null>(null);
  const [view, setView] = useState<View>("review");
  const [rules, setRules] = useState<Phase0Rule[]>([]);
  const [appMatch, setAppMatch] = useState("chrome.exe");
  const [reminder, setReminder] = useState("Follow up on the invoice");
  const [quickText, setQuickText] = useState("");
  const [quickCaptureId, setQuickCaptureId] = useState(
    () => `manual-${globalThis.crypto.randomUUID()}`,
  );
  const [quickPending, setQuickPending] = useState(false);
  const [quickMessage, setQuickMessage] = useState<string | null>(null);
  const [quickError, setQuickError] = useState<string | null>(null);
  const [gates, setGates] = useState<KillGate[]>([]);
  const [gateMessage, setGateMessage] = useState<string | null>(null);

  useEffect(() => {
    if (isQuick) return;
    void invoke<string | null>("load_setting", {
      key: "onboarding_completed_at",
    })
      .then((value) => setOnboarded(Boolean(value)))
      .catch(() => setOnboarded(false));
    void invoke<Phase0Rule[]>("list_phase0")
      .then(setRules)
      .catch(() => setRules([]));
    void invoke<KillGate[]>("list_kill_gates")
      .then(setGates)
      .catch(() => setGates([]));
  }, [isQuick]);

  const saveQuickCapture = async () => {
    const text = quickText.trim();
    setQuickMessage(null);
    setQuickError(null);
    if (!text) {
      setQuickError("Type a promise before saving.");
      return;
    }

    setQuickPending(true);
    try {
      const result = await invoke<QuickCaptureResult>("quick_capture", {
        captureId: quickCaptureId,
        text,
      });
      setQuickText("");
      setQuickCaptureId(`manual-${globalThis.crypto.randomUUID()}`);
      setQuickMessage(`Saved locally as promise ${result.promise_id}.`);
    } catch (reason: unknown) {
      setQuickError(String(reason));
    } finally {
      setQuickPending(false);
    }
  };

  const recordGate = (gate: KillGate, status: "passed" | "failed") => {
    const evidence = window.prompt(
      `Record local evidence for ${gate.id} (${status}).`,
      "",
    );
    if (!evidence?.trim()) return;
    setGateMessage(null);
    void invoke<KillGate[]>("record_kill_gate", {
      id: gate.id,
      status,
      notes: evidence.trim(),
    })
      .then((updated) => {
        setGates(updated);
        setGateMessage(`${gate.id} recorded as ${status}.`);
      })
      .catch((error: unknown) => setGateMessage(String(error)));
  };

  if (isQuick) {
    return (
      <main className="quick">
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
        {quickMessage ? <p className="success">{quickMessage}</p> : null}
        {quickError ? <p className="error">{quickError}</p> : null}
      </main>
    );
  }

  if (onboarded === null) {
    return <main className="boot">Loading local state…</main>;
  }

  if (!onboarded) {
    return (
      <Onboarding
        onDone={() => {
          setOnboarded(true);
        }}
      />
    );
  }

  return (
    <div className="shell">
      <aside>
        <p className="eyebrow">Callback</p>
        <nav>
          {(["review", "phase0", "settings", "health"] as const).map((item) => (
            <button
              key={item}
              type="button"
              className={view === item ? "active" : ""}
              onClick={() => setView(item)}
            >
              {item}
            </button>
          ))}
        </nav>
      </aside>
      <main>
        {view === "review" ? <ReviewQueue /> : null}
        {view === "phase0" ? (
          <section>
            <h1>Phase 0 rules</h1>
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
                onChange={(event) => setAppMatch(event.target.value)}
              />
            </label>
            <label>
              Reminder
              <input
                value={reminder}
                onChange={(event) => setReminder(event.target.value)}
              />
            </label>
            <button
              type="button"
              onClick={() => {
                void invoke<number>("add_phase0", {
                  appMatch,
                  reminderText: reminder,
                }).then((id) =>
                  setRules((current) => [
                    ...current,
                    {
                      id,
                      app_match: appMatch,
                      reminder_text: reminder,
                      enabled: true,
                    },
                  ]),
                );
              }}
            >
              Add rule
            </button>
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
                      disabled={gate.status === "passed"}
                      onClick={() => recordGate(gate, "passed")}
                    >
                      Record passed
                    </button>
                    <button
                      type="button"
                      disabled={gate.status === "failed"}
                      onClick={() => recordGate(gate, "failed")}
                    >
                      Record failed
                    </button>
                  </div>
                </li>
              ))}
            </ul>
            {gateMessage ? <p>{gateMessage}</p> : null}
          </section>
        ) : null}
        {view === "settings" ? <Settings /> : null}
        {view === "health" ? <HealthStatus /> : null}
      </main>
    </div>
  );
}
