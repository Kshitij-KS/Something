import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export function Settings() {
  const [cap, setCap] = useState("3");
  const [quiet, setQuiet] = useState(false);
  const [start, setStart] = useState("22:00");
  const [end, setEnd] = useState("08:00");
  const [gmail, setGmail] = useState(true);
  const [slack, setSlack] = useState(true);
  const [autostart, setAutostart] = useState(false);
  const [shortcut, setShortcut] = useState("Ctrl+Shift+K");
  const [shortcutFallback, setShortcutFallback] = useState("Ctrl+Alt+K");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void invoke<string | null>("load_setting", {
      key: "daily_surface_cap",
    }).then((value) => value && setCap(value));
    void invoke<string | null>("load_setting", {
      key: "quiet_hours_enabled",
    }).then((value) => setQuiet(value === "true"));
    void invoke<string | null>("load_setting", {
      key: "quiet_hours_start",
    }).then((value) => value && setStart(value));
    void invoke<string | null>("load_setting", {
      key: "quiet_hours_end",
    }).then((value) => value && setEnd(value));
    void invoke<string | null>("load_setting", {
      key: "gmail_enabled",
    }).then((value) => setGmail(value !== "false"));
    void invoke<string | null>("load_setting", {
      key: "slack_enabled",
    }).then((value) => setSlack(value !== "false"));
    void invoke<string | null>("load_setting", {
      key: "autostart_enabled",
    }).then((value) => setAutostart(value === "true"));
    void invoke<string | null>("load_setting", {
      key: "global_shortcut",
    }).then((value) => value && setShortcut(value));
    void invoke<string | null>("load_setting", {
      key: "global_shortcut_fallback",
    }).then((value) => value && setShortcutFallback(value));
  }, []);

  const save = (key: string, value: string) => {
    setError(null);
    void invoke("save_setting", { key, value }).catch((reason: unknown) =>
      setError(String(reason)),
    );
  };

  return (
    <section>
      <h1>Settings</h1>
      <label>
        Daily surface cap (max 3)
        <input
          value={cap}
          onChange={(event) => {
            setCap(event.target.value);
            save("daily_surface_cap", event.target.value);
          }}
        />
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={quiet}
          onChange={(event) => {
            setQuiet(event.target.checked);
            save("quiet_hours_enabled", String(event.target.checked));
          }}
        />
        Enforce Callback quiet hours (Windows DND is left to the OS)
      </label>
      <label>
        Quiet start
        <input
          value={start}
          onChange={(event) => {
            setStart(event.target.value);
            save("quiet_hours_start", event.target.value);
          }}
        />
      </label>
      <label>
        Quiet end
        <input
          value={end}
          onChange={(event) => {
            setEnd(event.target.value);
            save("quiet_hours_end", event.target.value);
          }}
        />
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={gmail}
          onChange={(event) => {
            setGmail(event.target.checked);
            save("gmail_enabled", String(event.target.checked));
          }}
        />
        Gmail web
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={slack}
          onChange={(event) => {
            setSlack(event.target.checked);
            save("slack_enabled", String(event.target.checked));
          }}
        />
        Slack web
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={autostart}
          onChange={(event) => {
            setAutostart(event.target.checked);
            save("autostart_enabled", String(event.target.checked));
          }}
        />
        Launch Callback when I sign in to Windows
      </label>
      <label>
        Quick-capture shortcut
        <input
          value={shortcut}
          onChange={(event) => {
            setShortcut(event.target.value);
            save("global_shortcut", event.target.value);
          }}
        />
      </label>
      <label>
        Shortcut fallback if the primary key is taken
        <input
          value={shortcutFallback}
          onChange={(event) => {
            setShortcutFallback(event.target.value);
            save("global_shortcut_fallback", event.target.value);
          }}
        />
      </label>
      <p className="meta">
        Ctrl+Shift+K opens the existing quick-capture window. Callback never
        reads selected text from other apps. If the shortcut is already used by
        Windows or another program, Callback tries the fallback and reports the
        result in Health. You can also open quick capture from Health.
      </p>
      <p className="meta">
        Autostart is off by default. Enabling it writes a current-user Run key
        for this app only. Callback never bypasses Windows Focus Assist / DND.
        Turn it off here or in Task Manager → Startup apps. Confirming that
        logon actually launches the installed build still needs a human session.
      </p>
      {error ? <p className="error">{error}</p> : null}
    </section>
  );
}
