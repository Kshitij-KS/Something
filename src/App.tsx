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

type KillGate = { id: string; status: string; notes: string };

export default function App() {
  const params = new URLSearchParams(window.location.search);
  const isQuick = params.get("window") === "quick";
  const [onboarded, setOnboarded] = useState<boolean | null>(null);
  const [view, setView] = useState<View>("review");
  const [rules, setRules] = useState<Phase0Rule[]>([]);
  const [appMatch, setAppMatch] = useState("chrome.exe");
  const [reminder, setReminder] = useState("Follow up on the invoice");
  const [quickText, setQuickText] = useState("");
  const [gates, setGates] = useState<KillGate[]>([]);

  useEffect(() => {
    void invoke<string | null>("load_setting", {
      key: "onboarding_completed_at",
    }).then((value) => setOnboarded(Boolean(value)));
    void invoke<Phase0Rule[]>("list_phase0")
      .then(setRules)
      .catch(() => setRules([]));
    void invoke<KillGate[]>("list_kill_gates")
      .then(setGates)
      .catch(() => setGates([]));
  }, []);

  if (isQuick) {
    return (
      <main className="quick">
        <h1>Quick capture</h1>
        <p>
          Type a promise. Callback never reads selected text from other apps.
        </p>
        <textarea
          value={quickText}
          onChange={(event) => setQuickText(event.target.value)}
        />
        <button
          type="button"
          onClick={() => {
            void invoke("quick_capture", { text: quickText }).then(() =>
              setQuickText(""),
            );
          }}
        >
          Save locally
        </button>
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
            <h2>Pending user gates</h2>
            <ul>
              {gates.map((gate) => (
                <li key={gate.id}>
                  <strong>{gate.id}</strong> ({gate.status}): {gate.notes}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
        {view === "settings" ? <Settings /> : null}
        {view === "health" ? <HealthStatus /> : null}
      </main>
    </div>
  );
}
