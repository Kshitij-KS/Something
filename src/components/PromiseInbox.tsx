import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import {
  PROMISE_TABS,
  formatMoment,
  formatRelative,
  sourceLabel,
  statusLabel,
  tabForStatus,
  type CallbackPromiseDetail,
  type CallbackPromiseSummary,
  type PromiseInboxAction,
  type PromiseTab,
} from "../promises";
import { PromiseDetail } from "./PromiseDetail";

export type PromiseInboxInteractionState = {
  busy: boolean;
  dirty: boolean;
};

type PromiseInboxProps = {
  onInteractionStateChange?: (state: PromiseInboxInteractionState) => void;
};

type ListState = "loading" | "ready" | "error";

type DetailState =
  | { kind: "idle" }
  | { kind: "loading"; id: number }
  | { kind: "ready"; detail: CallbackPromiseDetail }
  | { kind: "error"; id: number; message: string };

const CLEAN_INTERACTION_STATE: PromiseInboxInteractionState = {
  busy: false,
  dirty: false,
};

export function PromiseInbox({ onInteractionStateChange }: PromiseInboxProps) {
  const [tab, setTab] = useState<PromiseTab>("open");
  const [items, setItems] = useState<CallbackPromiseSummary[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detailState, setDetailState] = useState<DetailState>({ kind: "idle" });
  const [listState, setListState] = useState<ListState>("loading");
  const [pending, setPending] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [listRefreshKey, setListRefreshKey] = useState(0);
  const [detailRefreshKey, setDetailRefreshKey] = useState(0);
  const [editorGeneration, setEditorGeneration] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const mutationGeneration = useRef(0);
  const activeMutation = useRef<number | null>(null);
  const mounted = useRef(true);
  const dirtyRef = useRef(false);
  const selectedIdRef = useRef<number | null>(null);
  const tabRef = useRef<PromiseTab>("open");
  const editorFocusRef = useRef<HTMLTextAreaElement>(null);
  const deferredFocusGeneration = useRef(0);
  const tabButtons = useRef<
    Partial<Record<PromiseTab, HTMLButtonElement | null>>
  >({});

  const detail = detailState.kind === "ready" ? detailState.detail : null;
  const detailLoading = detailState.kind === "loading";
  const workspaceBusy = listState === "loading" || detailLoading || pending;

  const updateSelectedId = useCallback((id: number | null) => {
    selectedIdRef.current = id;
    setSelectedId(id);
  }, []);

  const updateTab = useCallback((nextTab: PromiseTab) => {
    tabRef.current = nextTab;
    setTab(nextTab);
  }, []);

  const scheduleFocus = useCallback((target: () => HTMLElement | null) => {
    const generation = deferredFocusGeneration.current + 1;
    const activeAtSchedule = document.activeElement;
    deferredFocusGeneration.current = generation;
    window.requestAnimationFrame(() => {
      const activeNow = document.activeElement;
      if (
        !mounted.current ||
        deferredFocusGeneration.current !== generation ||
        (activeNow !== null &&
          activeNow !== document.body &&
          activeNow !== activeAtSchedule)
      ) {
        return;
      }
      target()?.focus();
    });
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      activeMutation.current = null;
      mutationGeneration.current += 1;
      deferredFocusGeneration.current += 1;
    };
  }, []);

  useEffect(() => {
    onInteractionStateChange?.({ busy: pending, dirty });
  }, [dirty, onInteractionStateChange, pending]);

  useEffect(
    () => () => onInteractionStateChange?.(CLEAN_INTERACTION_STATE),
    [onInteractionStateChange],
  );

  useEffect(() => {
    const requestedTab = tab;
    let cancelled = false;
    void invoke<CallbackPromiseSummary[]>("list_promises", {
      tab: requestedTab,
    })
      .then((next) => {
        if (cancelled || tabRef.current !== requestedTab) return;
        if (dirtyRef.current) {
          setListState("ready");
          setMessage(
            "The inbox changed while you were editing. Save this draft or Refresh to discard it.",
          );
          return;
        }
        setItems(next);
        const current = selectedIdRef.current;
        const nextId =
          current !== null && next.some((item) => item.id === current)
            ? current
            : (next[0]?.id ?? null);
        if (nextId !== current) {
          updateSelectedId(nextId);
          setDetailState(
            nextId === null
              ? { kind: "idle" }
              : { kind: "loading", id: nextId },
          );
        }
        setListState("ready");
      })
      .catch((reason: unknown) => {
        if (cancelled || tabRef.current !== requestedTab) return;
        setListState("error");
        setError(String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [listRefreshKey, tab, updateSelectedId]);

  useEffect(() => {
    if (selectedId === null) return;

    const requestedId = selectedId;
    const requestedTab = tabRef.current;
    let cancelled = false;
    void invoke<CallbackPromiseDetail | null>("get_promise", {
      id: requestedId,
    })
      .then((next) => {
        if (
          cancelled ||
          selectedIdRef.current !== requestedId ||
          tabRef.current !== requestedTab
        ) {
          return;
        }
        if (!next) {
          setDetailState({ kind: "idle" });
          updateSelectedId(null);
          setMessage(
            "That promise is no longer available. The inbox was refreshed.",
          );
          setListState("loading");
          setListRefreshKey((current) => current + 1);
          return;
        }
        if (tabForStatus(next.status) !== requestedTab) {
          setDetailState({ kind: "idle" });
          updateSelectedId(null);
          setMessage(
            "That promise changed status outside this view. The inbox was refreshed.",
          );
          setListState("loading");
          setListRefreshKey((current) => current + 1);
          return;
        }
        setItems((current) =>
          current.map((item) => (item.id === next.id ? next : item)),
        );
        setDetailState({ kind: "ready", detail: next });
        setEditorGeneration((current) => current + 1);
      })
      .catch((reason: unknown) => {
        if (
          cancelled ||
          selectedIdRef.current !== requestedId ||
          tabRef.current !== requestedTab
        ) {
          return;
        }
        setDetailState({
          kind: "error",
          id: requestedId,
          message: String(reason),
        });
      });
    return () => {
      cancelled = true;
    };
  }, [detailRefreshKey, selectedId, updateSelectedId]);

  const updateDirty = useCallback((next: boolean) => {
    dirtyRef.current = next;
    setDirty(next);
  }, []);

  const confirmDiscard = () =>
    !dirtyRef.current ||
    window.confirm("Discard the unsaved changes to this promise and continue?");

  const refresh = () => {
    if (activeMutation.current !== null || !confirmDiscard()) return;
    deferredFocusGeneration.current += 1;
    updateDirty(false);
    setMessage(null);
    setError(null);
    setListState("loading");
    if (selectedId !== null) {
      setDetailState({ kind: "loading", id: selectedId });
    }
    setListRefreshKey((current) => current + 1);
    setDetailRefreshKey((current) => current + 1);
  };

  const chooseTab = (nextTab: PromiseTab) => {
    if (activeMutation.current !== null || nextTab === tab) return;
    if (!confirmDiscard()) return;
    deferredFocusGeneration.current += 1;
    updateDirty(false);
    updateTab(nextTab);
    setItems([]);
    updateSelectedId(null);
    setDetailState({ kind: "idle" });
    setMessage(null);
    setError(null);
    setListState("loading");
  };

  const choosePromise = (id: number) => {
    if (activeMutation.current !== null) return;
    deferredFocusGeneration.current += 1;
    if (id === selectedId) {
      if (detailState.kind === "error") {
        setMessage(null);
        setError(null);
        setDetailState({ kind: "loading", id });
        setDetailRefreshKey((current) => current + 1);
      }
      return;
    }
    if (!confirmDiscard()) return;
    updateDirty(false);
    updateSelectedId(id);
    setDetailState({ kind: "loading", id });
    setMessage(null);
    setError(null);
  };

  const beginMutation = (): number | null => {
    if (activeMutation.current !== null) return null;
    deferredFocusGeneration.current += 1;
    const generation = mutationGeneration.current + 1;
    mutationGeneration.current = generation;
    activeMutation.current = generation;
    setPending(true);
    setError(null);
    setMessage(null);
    return generation;
  };

  const isCurrentMutation = (generation: number) =>
    mounted.current && activeMutation.current === generation;

  const finishMutation = (generation: number) => {
    if (!isCurrentMutation(generation)) return;
    activeMutation.current = null;
    setPending(false);
  };

  const save = async (
    text: string,
    deadline: number | null,
    deadlineTimezone: string | null,
  ) => {
    if (!detail) return;
    const snapshot = detail;
    const generation = beginMutation();
    if (generation === null) return;
    try {
      const updated = await invoke<CallbackPromiseDetail>("update_promise", {
        id: snapshot.id,
        expectedStatus: snapshot.status,
        expectedIgnoreCount: snapshot.ignore_count,
        text,
        deadline,
        deadlineTimezone,
      });
      if (!isCurrentMutation(generation)) return;
      setDetailState({ kind: "ready", detail: updated });
      updateDirty(false);
      setEditorGeneration((current) => current + 1);
      setMessage("Changes saved locally.");
      setListState("loading");
      setListRefreshKey((current) => current + 1);
      scheduleFocus(() => editorFocusRef.current);
    } catch (reason: unknown) {
      if (isCurrentMutation(generation)) setError(String(reason));
    } finally {
      finishMutation(generation);
    }
  };

  const act = async (
    action: PromiseInboxAction,
    snoozeUntil?: number | null,
  ) => {
    if (!detail) return;
    if (!confirmDiscard()) return;
    const snapshot = detail;
    const originTab = tab;
    const generation = beginMutation();
    if (generation === null) return;
    try {
      const updated = await invoke<CallbackPromiseDetail>("act_on_promise", {
        id: snapshot.id,
        expectedStatus: snapshot.status,
        expectedIgnoreCount: snapshot.ignore_count,
        action,
        snoozeUntil: snoozeUntil ?? null,
      });
      if (!isCurrentMutation(generation)) return;
      const nextTab = tabForStatus(updated.status);
      if (nextTab !== originTab) setItems([]);
      setDetailState({ kind: "ready", detail: updated });
      updateSelectedId(updated.id);
      updateTab(nextTab);
      updateDirty(false);
      setEditorGeneration((current) => current + 1);
      setMessage(actionMessage(action, updated.status));
      setListState("loading");
      setListRefreshKey((current) => current + 1);
      scheduleFocus(() => tabButtons.current[nextTab] ?? null);
    } catch (reason: unknown) {
      if (isCurrentMutation(generation)) setError(String(reason));
    } finally {
      finishMutation(generation);
    }
  };

  const activeTab = PROMISE_TABS.find((item) => item.id === tab);

  return (
    <section className="promise-page" aria-labelledby="promises-heading">
      <header className="promise-page-header">
        <div>
          <p className="eyebrow">Commitment memory</p>
          <h1 id="promises-heading">Promises</h1>
          <p className="promise-page-intro">
            What you said you would do, kept locally until the right context
            returns.
          </p>
        </div>
        <button
          type="button"
          className="button-quiet"
          disabled={pending}
          onClick={refresh}
        >
          Refresh
        </button>
      </header>

      <div className="promise-tabs" role="group" aria-label="Promise status">
        {PROMISE_TABS.map((item) => (
          <button
            key={item.id}
            ref={(node) => {
              tabButtons.current[item.id] = node;
            }}
            type="button"
            aria-pressed={tab === item.id}
            className={tab === item.id ? "active" : ""}
            disabled={pending}
            onClick={() => chooseTab(item.id)}
          >
            {item.label}
          </button>
        ))}
      </div>

      {message ? (
        <p className="inline-notice success" role="status" aria-live="polite">
          {message}
        </p>
      ) : null}
      {error ? (
        <p className="inline-notice error" role="alert">
          {error}
        </p>
      ) : null}

      <div className="promise-workspace" aria-busy={workspaceBusy}>
        <div className="promise-list-pane">
          <div className="list-heading">
            <span>{activeTab?.label}</span>
            <small role="status" aria-live="polite">
              {listState === "loading"
                ? "Loading…"
                : listState === "error"
                  ? "Unavailable"
                  : `${items.length} total`}
            </small>
          </div>
          {listState === "ready" && items.length === 0 ? (
            <div className="promise-empty">
              <span className="empty-ring" aria-hidden="true" />
              <p>{activeTab?.empty}</p>
            </div>
          ) : null}
          <ul
            className="promise-list"
            aria-label={`${activeTab?.label} promises`}
          >
            {items.map((item) => (
              <li key={item.id}>
                <button
                  type="button"
                  className={selectedId === item.id ? "selected" : ""}
                  aria-current={selectedId === item.id ? "true" : undefined}
                  disabled={pending}
                  onClick={() => choosePromise(item.id)}
                >
                  <span className="promise-list-topline">
                    <span>{sourceLabel(item.source_app)}</span>
                    <span>{statusLabel(item.status)}</span>
                  </span>
                  <strong>{item.text}</strong>
                  <span className="promise-list-meta">
                    {item.recipient ? `${item.recipient} · ` : ""}
                    {listTimeLabel(item)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>

        <div className="promise-detail-pane">
          {detailState.kind === "loading" ? (
            <div className="detail-placeholder" role="status">
              Loading promise…
            </div>
          ) : null}
          {detailState.kind === "error" ? (
            <div className="detail-placeholder" role="alert">
              <p>{detailState.message}</p>
              <button
                type="button"
                className="button-secondary"
                disabled={pending}
                onClick={() => choosePromise(detailState.id)}
              >
                Try again
              </button>
            </div>
          ) : null}
          {detailState.kind === "idle" ? (
            <div className="detail-placeholder">
              <p className="eyebrow">Promise detail</p>
              <h2>Select a promise</h2>
              <p>Its context, deadline, and next actions will appear here.</p>
            </div>
          ) : null}
          {detail ? (
            <PromiseDetail
              key={`${detail.id}:${editorGeneration}`}
              detail={detail}
              pending={pending}
              editorFocusRef={editorFocusRef}
              onAction={act}
              onDirtyChange={updateDirty}
              onSave={save}
            />
          ) : null}
        </div>
      </div>
    </section>
  );
}

function listTimeLabel(item: CallbackPromiseSummary): string {
  if (item.status === "snoozed" && item.snooze_until) {
    return `Returns ${formatRelative(item.snooze_until)}`;
  }
  if (item.deadline) return `Due ${formatRelative(item.deadline)}`;
  if (item.resolved_at) return formatMoment(item.resolved_at);
  return formatRelative(item.created_at);
}

function actionMessage(
  action: PromiseInboxAction,
  status: CallbackPromiseDetail["status"],
): string {
  if (action === "done") return "Promise completed.";
  if (action === "snooze") return "Promise snoozed for one hour.";
  if (action === "resume") return "Promise returned to Open.";
  if (action === "promote")
    return "Promise promoted and return contexts added.";
  if (action === "not_a_promise") return "Dismissed and learned locally.";
  if (status === "archived") return "Archived after the third skip.";
  return "Skipped for now.";
}
