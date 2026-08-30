import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type SelectorSite = {
  site: string;
  status: string;
  first_observed_at: number | null;
  last_probe_at: number | null;
  last_success_at: number | null;
  last_capture_at: number | null;
  consecutive_failures: number;
  days_without_capture: number;
  banner: string | null;
};

type Snapshot = {
  connection: string;
  native_host: string;
  gmail: string;
  slack: string;
  selectors: SelectorSite[];
  last_handshake_at: number | null;
  silence_remaining_secs: number;
  opens_network_listener: boolean;
  shortcut: string;
};

type PurgeSchedule = { scheduled: boolean };

export function HealthStatus() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [reconnect, setReconnect] = useState<string | null>(null);
  const [purging, setPurging] = useState(false);
  const [purgeMessage, setPurgeMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = () => {
    void invoke<Snapshot>("health")
      .then(setSnap)
      .catch((reason: unknown) => setError(String(reason)));
  };

  useEffect(() => {
    load();
  }, []);

  return (
    <section>
      <h1>Health</h1>
      {snap ? (
        <>
          <ul>
            <li>Connection: {snap.connection}</li>
            <li>Native host: {snap.native_host}</li>
            <li>Gmail: {snap.gmail}</li>
            <li>Slack: {snap.slack}</li>
            <li>
              Last handshake:{" "}
              {snap.last_handshake_at ? String(snap.last_handshake_at) : "none"}
            </li>
            <li>Onboarding silence: {snap.silence_remaining_secs}s</li>
            <li>
              TCP/UDP listener: {snap.opens_network_listener ? "yes" : "none"}
            </li>
            <li>Global shortcut: {snap.shortcut}</li>
          </ul>
          <h2>Capture selectors</h2>
          <ul>
            {snap.selectors.map((selector) => (
              <li key={selector.site}>
                <strong>{selector.site}</strong>: {selector.status}; failures{" "}
                {selector.consecutive_failures}; last probe{" "}
                {selector.last_probe_at ?? "none"}; last capture{" "}
                {selector.last_capture_at ?? "none"}
                {selector.banner ? (
                  <p className="banner">{selector.banner}</p>
                ) : null}
              </li>
            ))}
          </ul>
        </>
      ) : (
        <p>Reading diagnostics…</p>
      )}
      <button
        type="button"
        onClick={() => {
          setError(null);
          load();
        }}
      >
        Refresh diagnostics
      </button>
      <button
        type="button"
        onClick={() => {
          setError(null);
          void invoke("open_quick_capture").catch((reason: unknown) =>
            setError(String(reason)),
          );
        }}
      >
        Open quick capture
      </button>
      <button
        type="button"
        onClick={() => {
          setError(null);
          void invoke<string>("reconnect_extension")
            .then((message) => {
              setReconnect(message);
              load();
            })
            .catch((reason: unknown) => setError(String(reason)));
        }}
      >
        Reconnect extension
      </button>
      {reconnect ? <p>{reconnect}</p> : null}
      {purgeMessage ? <p>{purgeMessage}</p> : null}
      {error ? <p className="error">{error}</p> : null}
      <p className="meta">
        Purge closes Callback, then deletes its desktop database, SQLite
        sidecars, native-host manifest and registrations, and autostart entry.
        Pending captures in Chrome extension storage are separate; remove the
        extension to clear that browser-managed queue too.
      </p>
      <button
        type="button"
        disabled={purging}
        onClick={() => {
          const confirmed = window.confirm(
            "Permanently delete Callback desktop data and disable its registrations? The app will close.",
          );
          if (!confirmed) return;
          setPurging(true);
          setPurgeMessage(null);
          setError(null);
          void invoke<PurgeSchedule>("purge_data")
            .then((result) => {
              if (!result.scheduled) {
                throw new Error("The purge helper was not scheduled.");
              }
              setPurgeMessage("Purge scheduled. Callback is closing…");
            })
            .catch((reason: unknown) => {
              setPurging(false);
              setError(String(reason));
            });
        }}
      >
        {purging ? "Scheduling purge…" : "Purge local desktop data"}
      </button>
    </section>
  );
}
