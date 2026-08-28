import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Snapshot = {
  connection: string;
  native_host: string;
  gmail: string;
  slack: string;
  last_handshake_at: number | null;
  silence_remaining_secs: number;
  opens_network_listener: boolean;
  shortcut: string;
};

export function HealthStatus() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [reconnect, setReconnect] = useState<string | null>(null);

  const load = () => {
    void invoke<Snapshot>("health").then(setSnap);
    void invoke<string | null>("health_banner", {
      site: "gmail",
      status: "healthy",
      days: 0,
    }).then(setBanner);
  };

  useEffect(() => {
    load();
  }, []);

  return (
    <section>
      <h1>Health</h1>
      {snap ? (
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
      ) : (
        <p>Reading diagnostics…</p>
      )}
      {banner ? <p className="banner">{banner}</p> : null}
      <button
        type="button"
        onClick={() => {
          void invoke("open_quick_capture");
        }}
      >
        Open quick capture
      </button>
      <button
        type="button"
        onClick={() => {
          void invoke<string>("reconnect_extension").then(setReconnect);
        }}
      >
        Reconnect extension
      </button>
      {reconnect ? <p>{reconnect}</p> : null}
      <button
        type="button"
        onClick={() => {
          void invoke("purge_data");
        }}
      >
        Purge local data
      </button>
    </section>
  );
}
