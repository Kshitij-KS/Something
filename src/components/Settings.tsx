import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const SETTING_KEYS = [
  "daily_surface_cap",
  "min_gap_minutes",
  "retention_days",
  "timezone",
  "quiet_hours_enabled",
  "quiet_hours_start",
  "quiet_hours_end",
  "gmail_enabled",
  "slack_enabled",
  "autostart_enabled",
  "global_shortcut",
  "global_shortcut_fallback",
] as const;

export function Settings() {
  const [cap, setCap] = useState("3");
  const [minGap, setMinGap] = useState("90");
  const [retentionDays, setRetentionDays] = useState("365");
  const [timezone, setTimezone] = useState("UTC");
  const [quiet, setQuiet] = useState(false);
  const [start, setStart] = useState("22:00");
  const [end, setEnd] = useState("08:00");
  const [gmail, setGmail] = useState(true);
  const [slack, setSlack] = useState(true);
  const [autostart, setAutostart] = useState(false);
  const [shortcut, setShortcut] = useState("Ctrl+Shift+K");
  const [shortcutFallback, setShortcutFallback] = useState("Ctrl+Alt+K");
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void Promise.all(
      SETTING_KEYS.map((key) => invoke<string | null>("load_setting", { key })),
    )
      .then(
        ([
          loadedCap,
          loadedMinGap,
          loadedRetention,
          loadedTimezone,
          loadedQuiet,
          loadedStart,
          loadedEnd,
          loadedGmail,
          loadedSlack,
          loadedAutostart,
          loadedShortcut,
          loadedShortcutFallback,
        ]) => {
          if (!active) return;
          if (loadedCap) setCap(loadedCap);
          if (loadedMinGap) setMinGap(loadedMinGap);
          if (loadedRetention) setRetentionDays(loadedRetention);
          if (loadedTimezone) setTimezone(loadedTimezone);
          setQuiet(loadedQuiet === "true");
          if (loadedStart) setStart(loadedStart);
          if (loadedEnd) setEnd(loadedEnd);
          setGmail(loadedGmail !== "false");
          setSlack(loadedSlack !== "false");
          setAutostart(loadedAutostart === "true");
          if (loadedShortcut) setShortcut(loadedShortcut);
          if (loadedShortcutFallback)
            setShortcutFallback(loadedShortcutFallback);
        },
      )
      .catch((reason: unknown) => {
        if (active) setError(`Could not load settings: ${String(reason)}`);
      });
    return () => {
      active = false;
    };
  }, []);

  const save = async (key: string, value: string): Promise<boolean> => {
    setError(null);
    setSaved(null);
    try {
      await invoke("save_setting", { key, value });
      setSaved("Settings saved locally.");
      return true;
    } catch (reason: unknown) {
      setError(String(reason));
      return false;
    }
  };

  const updateBoolean = (
    key: string,
    next: boolean,
    previous: boolean,
    setValue: (value: boolean) => void,
  ) => {
    setValue(next);
    void save(key, String(next)).then((ok) => {
      if (!ok) setValue(previous);
    });
  };

  return (
    <section>
      <h1>Settings</h1>
      <label>
        Daily surface cap (1–3)
        <input
          type="number"
          min="1"
          max="3"
          value={cap}
          onChange={(event) => setCap(event.target.value)}
          onBlur={() => void save("daily_surface_cap", cap)}
        />
      </label>
      <label>
        Minimum gap between reminders (minutes, at least 90)
        <input
          type="number"
          min="90"
          value={minGap}
          onChange={(event) => setMinGap(event.target.value)}
          onBlur={() => void save("min_gap_minutes", minGap)}
        />
      </label>
      <label>
        Retain local source context (days, 1–3650)
        <input
          type="number"
          min="1"
          max="3650"
          value={retentionDays}
          onChange={(event) => setRetentionDays(event.target.value)}
          onBlur={() => void save("retention_days", retentionDays)}
        />
      </label>
      <p className="meta">
        When context expires, resolved and review items are removed. Open and
        snoozed promises remain, but original message context is redacted. Old
        retry metadata is removed once no retained reminder depends on it.
      </p>
      <label>
        Local timezone (IANA name)
        <input
          value={timezone}
          placeholder="America/New_York"
          onChange={(event) => setTimezone(event.target.value)}
          onBlur={() => void save("timezone", timezone)}
        />
      </label>
      <p className="meta">
        Relative deadlines use this timezone, including daylight-saving rules.
      </p>
      <label className="check">
        <input
          type="checkbox"
          checked={quiet}
          onChange={(event) =>
            updateBoolean(
              "quiet_hours_enabled",
              event.target.checked,
              quiet,
              setQuiet,
            )
          }
        />
        Enforce Callback quiet hours (Windows DND is left to the OS)
      </label>
      <label>
        Quiet start
        <input
          type="time"
          value={start}
          onChange={(event) => setStart(event.target.value)}
          onBlur={() => void save("quiet_hours_start", start)}
        />
      </label>
      <label>
        Quiet end
        <input
          type="time"
          value={end}
          onChange={(event) => setEnd(event.target.value)}
          onBlur={() => void save("quiet_hours_end", end)}
        />
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={gmail}
          onChange={(event) =>
            updateBoolean(
              "gmail_enabled",
              event.target.checked,
              gmail,
              setGmail,
            )
          }
        />
        Gmail web
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={slack}
          onChange={(event) =>
            updateBoolean(
              "slack_enabled",
              event.target.checked,
              slack,
              setSlack,
            )
          }
        />
        Slack web
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={autostart}
          onChange={(event) =>
            updateBoolean(
              "autostart_enabled",
              event.target.checked,
              autostart,
              setAutostart,
            )
          }
        />
        Launch Callback when I sign in to Windows
      </label>
      <label>
        Quick-capture shortcut
        <input
          value={shortcut}
          onChange={(event) => setShortcut(event.target.value)}
          onBlur={() => void save("global_shortcut", shortcut)}
        />
      </label>
      <label>
        Shortcut fallback if the primary key is taken
        <input
          value={shortcutFallback}
          onChange={(event) => setShortcutFallback(event.target.value)}
          onBlur={() => void save("global_shortcut_fallback", shortcutFallback)}
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
      {saved ? <p className="success">{saved}</p> : null}
      {error ? <p className="error">{error}</p> : null}
    </section>
  );
}
