import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { HealthStatus } from "./components/HealthStatus";
import { Onboarding } from "./components/Onboarding";
import {
  PromiseInbox,
  type PromiseInboxInteractionState,
} from "./components/PromiseInbox";
import { Settings } from "./components/Settings";
import type { PromiseRouteRequest } from "./promises";

type View = "promises" | "phase0" | "settings" | "health";

type Phase0Rule = {
  id: number;
  app_match: string;
  reminder_text: string;
  enabled: boolean;
};

type FocusApp = { executable: string };
type FocusAppsState = "idle" | "loading" | "ready" | "error";

type KillGateStatus = "pending_user" | "passed" | "failed";
type KillGate = { id: string; status: KillGateStatus; notes: string };
type QuickCaptureResult = { capture_id: string; promise_id: number };

const PROMISE_ROUTE_READY_EVENT = "promise-route-ready";
const FOCUS_APP_REFRESH_TIMEOUT_MS = 5_000;

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

function focusAppStatusText(state: FocusAppsState, count: number): string {
  if (state === "loading") {
    return "Finding visible apps… You can keep typing manually.";
  }
  if (state === "error") {
    return count > 0
      ? "Could not refresh visible apps. Existing choices remain, or type an executable name."
      : "Could not list visible apps. Type an executable name manually.";
  }
  if (state === "ready") {
    return count === 0
      ? "No visible apps found. Type an executable name manually."
      : `${count} visible ${count === 1 ? "app" : "apps"} found. Select one or type an executable name.`;
  }
  return "Visible apps load when you open this page. You can always type an executable name.";
}

export default function App() {
  const params = new URLSearchParams(window.location.search);
  const isQuick = params.get("window") === "quick";
  const [onboarded, setOnboarded] = useState<boolean | null>(null);
  const [view, setView] = useState<View>("promises");
  const [promiseInteraction, setPromiseInteraction] =
    useState<PromiseInboxInteractionState>(CLEAN_PROMISE_INTERACTION);
  const [promiseRoute, setPromiseRoute] = useState<PromiseRouteRequest | null>(
    null,
  );
  const [promiseRouteError, setPromiseRouteError] = useState<string | null>(
    null,
  );
  const [routeListenerVersion, setRouteListenerVersion] = useState(0);
  const [rules, setRules] = useState<Phase0Rule[]>([]);
  const [phase0RulesState, setPhase0RulesState] = useState<
    "loading" | "ready" | "error"
  >("loading");
  const [focusApps, setFocusApps] = useState<FocusApp[]>([]);
  const [focusAppsState, setFocusAppsState] = useState<FocusAppsState>("idle");
  const [appMatch, setAppMatch] = useState("chrome.exe");
  const [reminder, setReminder] = useState("Follow up on the invoice");
  const [phase0Pending, setPhase0Pending] = useState(false);
  const [phase0Message, setPhase0Message] = useState<string | null>(null);
  const [phase0Error, setPhase0Error] = useState<string | null>(null);
  const [phase0RulePendingIds, setPhase0RulePendingIds] = useState<
    ReadonlySet<number>
  >(() => new Set());
  const [phase0RuleFeedback, setPhase0RuleFeedback] = useState<
    Record<number, { tone: "success" | "error"; text: string }>
  >({});
  const phase0RuleInFlight = useRef(new Set<number>());
  const phase0RulesLoadOwner = useRef<symbol | null>(null);
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
  const focusAppsInFlight = useRef(false);
  const gateInFlight = useRef<string | null>(null);
  const gateMutationVersion = useRef(0);
  const routePeekInFlight = useRef<Promise<void> | null>(null);
  const routePeekAgain = useRef(false);
  const acknowledgedRouteIds = useRef(new Set<string>());

  const peekPendingPromiseRoute = useCallback((): Promise<void> => {
    if (isQuick) return Promise.resolve();
    const inFlight = routePeekInFlight.current;
    if (inFlight !== null) {
      routePeekAgain.current = true;
      return inFlight;
    }

    let finishDrain: () => void = () => undefined;
    const drain = new Promise<void>((resolve) => {
      finishDrain = resolve;
    });
    routePeekInFlight.current = drain;

    const runDrain = async () => {
      try {
        do {
          routePeekAgain.current = false;
          try {
            const next = await invoke<PromiseRouteRequest | null>(
              "peek_pending_promise_route",
            );
            if (next && !acknowledgedRouteIds.current.has(next.route_id)) {
              setView("promises");
            }
            setPromiseRoute((current) => {
              const retained =
                current && !acknowledgedRouteIds.current.has(current.route_id)
                  ? current
                  : null;
              if (retained) return retained;
              return next && !acknowledgedRouteIds.current.has(next.route_id)
                ? next
                : null;
            });
            setPromiseRouteError(null);
          } catch {
            setPromiseRouteError(
              "Callback could not read the clicked reminder. Retry without closing the app.",
            );
          }
        } while (routePeekAgain.current);
      } catch {
        setPromiseRouteError(
          "Callback could not read the clicked reminder. Retry without closing the app.",
        );
      } finally {
        if (routePeekInFlight.current === drain) {
          routePeekInFlight.current = null;
        }
        finishDrain();
      }
    };
    void runDrain();
    return drain;
  }, [isQuick]);

  const acknowledgePromiseRoute = useCallback(
    async (routeId: string) => {
      try {
        await invoke<void>("ack_pending_promise_route", { routeId });
        acknowledgedRouteIds.current.add(routeId);
        if (acknowledgedRouteIds.current.size > 128) {
          const oldest = acknowledgedRouteIds.current.values().next().value;
          if (oldest !== undefined) acknowledgedRouteIds.current.delete(oldest);
        }
        setPromiseRoute((current) =>
          current?.route_id === routeId ? null : current,
        );
        setPromiseRouteError(null);
        await peekPendingPromiseRoute();
      } catch (reason: unknown) {
        setPromiseRouteError(
          "Callback opened the reminder but could not finish its local route. Retry from the reminder banner.",
        );
        throw reason instanceof Error ? reason : new Error(String(reason));
      }
    },
    [peekPendingPromiseRoute],
  );

  const refreshFocusApps = useCallback(async () => {
    if (focusAppsInFlight.current) return;
    focusAppsInFlight.current = true;
    setFocusAppsState("loading");
    let timeoutId: number | undefined;
    try {
      const timeout = new Promise<never>((_, reject) => {
        timeoutId = window.setTimeout(
          () => reject(new Error("focus app refresh timed out")),
          FOCUS_APP_REFRESH_TIMEOUT_MS,
        );
      });
      const next = await Promise.race([
        invoke<FocusApp[]>("list_focus_apps"),
        timeout,
      ]);
      setFocusApps(next);
      setFocusAppsState("ready");
    } catch {
      setFocusAppsState("error");
    } finally {
      if (timeoutId !== undefined) window.clearTimeout(timeoutId);
      focusAppsInFlight.current = false;
    }
  }, []);

  useEffect(() => {
    if (isQuick) return;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;

    void listen<void>(PROMISE_ROUTE_READY_EVENT, () => {
      if (!disposed) void peekPendingPromiseRoute();
    })
      .then((stop) => {
        if (disposed) {
          stop();
          return;
        }
        unlisten = stop;
        void peekPendingPromiseRoute();
      })
      .catch(() => {
        if (!disposed) {
          setPromiseRouteError(
            "Callback could not start notification routing. Retry without closing the app.",
          );
        }
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [isQuick, peekPendingPromiseRoute, routeListenerVersion]);

  useEffect(() => {
    if (isQuick) return;
    const initialGateVersion = gateMutationVersion.current;
    void invoke<string | null>("load_setting", {
      key: "onboarding_completed_at",
    })
      .then((value) => setOnboarded(Boolean(value)))
      .catch(() => setOnboarded(false));
    if (phase0RulesLoadOwner.current === null) {
      const owner = Symbol("initial-phase0-rules-load");
      phase0RulesLoadOwner.current = owner;
      void invoke<Phase0Rule[]>("list_phase0")
        .then((loaded) => {
          if (phase0RulesLoadOwner.current !== owner) return;
          setRules(loaded);
          setPhase0RulesState("ready");
        })
        .catch(() => {
          if (phase0RulesLoadOwner.current === owner) {
            setPhase0RulesState("error");
          }
        })
        .finally(() => {
          if (phase0RulesLoadOwner.current === owner) {
            phase0RulesLoadOwner.current = null;
          }
        });
    }
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

  const retryPhase0Rules = async () => {
    if (
      phase0RulesLoadOwner.current !== null ||
      phase0InFlight.current ||
      phase0RuleInFlight.current.size > 0
    ) {
      return;
    }
    const owner = Symbol("phase0-rules-retry");
    phase0RulesLoadOwner.current = owner;
    setPhase0RulesState("loading");
    try {
      const loaded = await invoke<Phase0Rule[]>("list_phase0");
      if (phase0RulesLoadOwner.current !== owner) return;
      setRules(loaded);
      setPhase0RulesState("ready");
    } catch {
      if (phase0RulesLoadOwner.current === owner) {
        setPhase0RulesState("error");
      }
    } finally {
      if (phase0RulesLoadOwner.current === owner) {
        phase0RulesLoadOwner.current = null;
      }
    }
  };

  const addPhase0Rule = async () => {
    if (
      phase0InFlight.current ||
      phase0RulesLoadOwner.current !== null ||
      phase0RuleInFlight.current.size > 0
    ) {
      return;
    }
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
      const updated = await invoke<Phase0Rule>("add_phase0", {
        appMatch: submittedApp,
        reminderText: submittedReminder,
      });
      setRules((current) => {
        const next = current.some((rule) => rule.id === updated.id)
          ? current.map((rule) => (rule.id === updated.id ? updated : rule))
          : [...current, updated];
        return next.sort((left, right) => left.id - right.id);
      });
      setPhase0Message(
        updated.enabled
          ? "Focus rule saved locally."
          : "That focus rule already exists and is paused.",
      );
    } catch (reason: unknown) {
      setPhase0Error(String(reason));
    } finally {
      phase0InFlight.current = false;
      setPhase0Pending(false);
    }
  };

  const togglePhase0Rule = async (rule: Phase0Rule) => {
    const { id } = rule;
    const inFlight = phase0RuleInFlight.current;
    if (
      phase0RulesLoadOwner.current !== null ||
      phase0InFlight.current ||
      inFlight.has(id)
    ) {
      return;
    }

    const enabled = !rule.enabled;
    const action = enabled ? "resume" : "pause";
    inFlight.add(id);
    setPhase0RulePendingIds((current) => {
      const next = new Set(current);
      next.add(id);
      return next;
    });
    setPhase0RuleFeedback((current) => {
      const next = { ...current };
      delete next[id];
      return next;
    });

    try {
      const updated = await invoke<Phase0Rule>("set_phase0_rule_enabled", {
        id,
        enabled,
      });
      setRules((current) => {
        const next = current.some((candidate) => candidate.id === updated.id)
          ? current.map((candidate) =>
              candidate.id === updated.id ? updated : candidate,
            )
          : [...current, updated];
        return next.sort((left, right) => left.id - right.id);
      });
      setPhase0RuleFeedback((current) => ({
        ...current,
        [id]: {
          tone: "success",
          text: updated.enabled ? "Rule resumed." : "Rule paused.",
        },
      }));
    } catch {
      setPhase0RuleFeedback((current) => ({
        ...current,
        [id]: {
          tone: "error",
          text: `Could not ${action} this rule. Try again; if it still fails, restart Callback to refresh local rules.`,
        },
      }));
    } finally {
      inFlight.delete(id);
      setPhase0RulePendingIds((current) => {
        const next = new Set(current);
        next.delete(id);
        return next;
      });
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
    if (nextView === "phase0" && focusAppsState === "idle") {
      void refreshFocusApps();
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
        {promiseRouteError ? (
          <div className="inline-notice error route-notice" role="alert">
            <span>{promiseRouteError}</span>
            <button
              type="button"
              className="button-quiet"
              onClick={() => {
                setPromiseRouteError(null);
                setRouteListenerVersion((current) => current + 1);
              }}
            >
              Retry routing
            </button>
          </div>
        ) : null}
        {view === "promises" ? (
          <PromiseInbox
            routeRequest={promiseRoute}
            onInteractionStateChange={setPromiseInteraction}
            onRouteHandled={acknowledgePromiseRoute}
          />
        ) : null}
        {view === "phase0" ? (
          <section
            className="content-section"
            aria-busy={phase0Pending || phase0RulesState === "loading"}
          >
            <p className="eyebrow">Context prototype</p>
            <h1>Focus rules</h1>
            <p>
              When I focus an app, show a hardcoded reminder. No extraction.
            </p>
            {phase0RulesState === "error" ? (
              <div
                className="inline-notice error phase0-rules-notice"
                role="alert"
              >
                <span>
                  Callback could not load every local focus rule. Retry before
                  relying on this list.
                </span>
                <button
                  type="button"
                  className="button-quiet"
                  disabled={phase0Pending || phase0RulePendingIds.size > 0}
                  onClick={() => void retryPhase0Rules()}
                >
                  Retry loading rules
                </button>
              </div>
            ) : null}
            {rules.length > 0 ? (
              <ul className="phase0-rule-list">
                {rules.map((rule) => {
                  const pending = phase0RulePendingIds.has(rule.id);
                  const feedback = phase0RuleFeedback[rule.id];
                  const actionLabel = rule.enabled ? "Pause" : "Resume";
                  const appLabelId = `phase0-rule-app-${rule.id}`;
                  const actionLabelId = `phase0-rule-action-${rule.id}`;
                  return (
                    <li
                      key={rule.id}
                      className="phase0-rule"
                      aria-busy={pending}
                    >
                      <div className="phase0-rule-copy">
                        <p>
                          <strong id={appLabelId} translate="no">
                            {rule.app_match}
                          </strong>{" "}
                          → {rule.reminder_text}
                        </p>
                        <small className="meta phase0-rule-status">
                          Status: {rule.enabled ? "Active" : "Paused"}
                        </small>
                      </div>
                      <button
                        type="button"
                        className="button-quiet"
                        disabled={
                          pending ||
                          phase0Pending ||
                          phase0RulesState === "loading"
                        }
                        aria-labelledby={`${actionLabelId} ${appLabelId}`}
                        onClick={() => void togglePhase0Rule(rule)}
                      >
                        <span id={actionLabelId}>
                          {pending
                            ? rule.enabled
                              ? "Pausing…"
                              : "Resuming…"
                            : actionLabel}
                        </span>
                      </button>
                      <p
                        className={`phase0-rule-feedback${feedback ? ` ${feedback.tone}` : ""}`}
                        role={feedback?.tone === "error" ? "alert" : "status"}
                        aria-live={
                          feedback?.tone === "error" ? "assertive" : "polite"
                        }
                        aria-atomic="true"
                      >
                        {feedback?.text ?? ""}
                      </p>
                    </li>
                  );
                })}
              </ul>
            ) : phase0RulesState === "loading" ? (
              <p className="meta" role="status" aria-live="polite">
                Loading focus rules…
              </p>
            ) : phase0RulesState === "error" ? null : (
              <p className="empty" role="status" aria-live="polite">
                No focus rules yet. Add one below to start the five-day trial.
              </p>
            )}
            <div className="focus-app-picker">
              <label htmlFor="phase0-app">App executable</label>
              <div className="focus-app-control">
                <input
                  id="phase0-app"
                  name="phase0-app"
                  type="text"
                  list="phase0-app-options"
                  value={appMatch}
                  disabled={phase0Pending}
                  autoComplete="off"
                  spellCheck={false}
                  translate="no"
                  aria-describedby="focus-app-status"
                  onChange={(event) => {
                    setAppMatch(event.target.value);
                    setPhase0Message(null);
                    setPhase0Error(null);
                  }}
                />
                <button
                  type="button"
                  className="button-quiet"
                  disabled={phase0Pending || focusAppsState === "loading"}
                  onClick={() => void refreshFocusApps()}
                >
                  {focusAppsState === "loading"
                    ? "Finding apps…"
                    : "Refresh apps"}
                </button>
              </div>
              <datalist id="phase0-app-options">
                {focusApps.map((app) => (
                  <option
                    key={app.executable}
                    value={app.executable}
                    translate="no"
                  />
                ))}
              </datalist>
              <small
                id="focus-app-status"
                className="focus-app-status"
                role="status"
                aria-live="polite"
              >
                {focusAppStatusText(focusAppsState, focusApps.length)}
              </small>
            </div>
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
              disabled={
                phase0Pending ||
                phase0RulePendingIds.size > 0 ||
                phase0RulesState === "loading" ||
                !appMatch.trim() ||
                !reminder.trim()
              }
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
