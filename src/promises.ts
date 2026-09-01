export type PromiseStatus =
  "review" | "open" | "snoozed" | "done" | "dismissed" | "archived";

export type PromiseTab = "open" | "snoozed" | "review" | "resolved";

export type PromiseInboxAction =
  "promote" | "done" | "snooze" | "not_a_promise" | "ignore" | "resume";

export type CallbackPromiseSummary = {
  id: number;
  text: string;
  status: PromiseStatus;
  source_app: string;
  recipient?: string | null;
  deadline?: number | null;
  snooze_until?: number | null;
  ignore_count: number;
  created_at: number;
  resolved_at?: number | null;
};

export type PromiseTrigger = {
  kind: string;
  match_value: string;
  priority: number;
};

export type CallbackPromiseDetail = CallbackPromiseSummary & {
  source_ctx?: string | null;
  sent_at: number;
  score: number;
  confidence: number;
  deadline_tz?: string | null;
  deadline_precision?: string | null;
  deadline_escalated_at?: number | null;
  surface_count: number;
  last_shown_at?: number | null;
  triggers: PromiseTrigger[];
};

export const PROMISE_TABS: ReadonlyArray<{
  id: PromiseTab;
  label: string;
  empty: string;
}> = [
  {
    id: "open",
    label: "Open",
    empty: "No open promises. New commitments will collect here.",
  },
  {
    id: "snoozed",
    label: "Snoozed",
    empty: "Nothing is snoozed.",
  },
  {
    id: "review",
    label: "Review",
    empty: "Nothing needs your review.",
  },
  {
    id: "resolved",
    label: "Resolved",
    empty: "Completed and archived promises will appear here.",
  },
];

export function tabForStatus(status: PromiseStatus): PromiseTab {
  if (status === "open") return "open";
  if (status === "snoozed") return "snoozed";
  if (status === "review") return "review";
  return "resolved";
}

export function sourceLabel(source: string): string {
  if (source === "gmail") return "Gmail";
  if (source === "slack") return "Slack";
  if (source === "manual") return "Quick capture";
  return source;
}

export function statusLabel(status: PromiseStatus): string {
  if (status === "dismissed") return "Not a promise";
  return `${status.slice(0, 1).toUpperCase()}${status.slice(1)}`;
}

export function formatMoment(
  timestamp?: number | null,
  timeZone?: string | null,
): string {
  if (!timestamp) return "Not set";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: timeZone ? resolveTimeZone(timeZone) : undefined,
  }).format(new Date(timestamp * 1000));
}

export function formatRelative(timestamp?: number | null): string {
  if (!timestamp) return "";
  const seconds = timestamp - Math.floor(Date.now() / 1000);
  const absolute = Math.abs(seconds);
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  if (absolute < 90 * 60)
    return formatter.format(Math.round(seconds / 60), "minute");
  if (absolute < 36 * 60 * 60) {
    return formatter.format(Math.round(seconds / (60 * 60)), "hour");
  }
  return formatter.format(Math.round(seconds / (24 * 60 * 60)), "day");
}

type WallClock = {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
};

const wallClockFormatters = new Map<string, Intl.DateTimeFormat>();

export function resolveTimeZone(preferred?: string | null): string {
  const local = Intl.DateTimeFormat().resolvedOptions().timeZone;
  for (const candidate of [preferred, local, "UTC"]) {
    if (!candidate) continue;
    try {
      new Intl.DateTimeFormat("en", { timeZone: candidate }).format(0);
      return candidate;
    } catch {
      // Try the next local-only fallback.
    }
  }
  return "UTC";
}

function wallClockFormatter(timeZone: string): Intl.DateTimeFormat {
  const existing = wallClockFormatters.get(timeZone);
  if (existing) return existing;
  const formatter = new Intl.DateTimeFormat("en-CA-u-hc-h23", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
  wallClockFormatters.set(timeZone, formatter);
  return formatter;
}

function wallClockAt(timestampMs: number, timeZone: string): WallClock {
  const values = new Map(
    wallClockFormatter(timeZone)
      .formatToParts(new Date(timestampMs))
      .map((part) => [part.type, part.value]),
  );
  return {
    year: Number(values.get("year")),
    month: Number(values.get("month")),
    day: Number(values.get("day")),
    hour: Number(values.get("hour")),
    minute: Number(values.get("minute")),
  };
}

function wallClockUtcMs(value: WallClock): number | null {
  const date = new Date(0);
  date.setUTCFullYear(value.year, value.month - 1, value.day);
  date.setUTCHours(value.hour, value.minute, 0, 0);
  if (
    date.getUTCFullYear() !== value.year ||
    date.getUTCMonth() !== value.month - 1 ||
    date.getUTCDate() !== value.day ||
    date.getUTCHours() !== value.hour ||
    date.getUTCMinutes() !== value.minute
  ) {
    return null;
  }
  return date.getTime();
}

function sameWallClock(left: WallClock, right: WallClock): boolean {
  return (
    left.year === right.year &&
    left.month === right.month &&
    left.day === right.day &&
    left.hour === right.hour &&
    left.minute === right.minute
  );
}

function parseWallClock(value: string): WallClock | null {
  const match = /^(\d{4,})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(value);
  if (!match) return null;
  const wallClock = {
    year: Number(match[1]),
    month: Number(match[2]),
    day: Number(match[3]),
    hour: Number(match[4]),
    minute: Number(match[5]),
  };
  return wallClockUtcMs(wallClock) === null ? null : wallClock;
}

export function toDateTimeLocal(
  timestamp?: number | null,
  timeZone?: string | null,
): string {
  if (!timestamp) return "";
  const wallClock = wallClockAt(timestamp * 1000, resolveTimeZone(timeZone));
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${String(wallClock.year).padStart(4, "0")}-${pad(wallClock.month)}-${pad(wallClock.day)}T${pad(wallClock.hour)}:${pad(wallClock.minute)}`;
}

export function fromDateTimeLocal(
  value: string,
  timeZone?: string | null,
): number | null {
  if (!value) return null;
  const desired = parseWallClock(value);
  if (!desired) return null;
  const desiredAsUtc = wallClockUtcMs(desired);
  if (desiredAsUtc === null) return null;

  const zone = resolveTimeZone(timeZone);
  let candidate = desiredAsUtc;
  for (let attempt = 0; attempt < 6; attempt += 1) {
    const displayedAsUtc = wallClockUtcMs(wallClockAt(candidate, zone));
    if (displayedAsUtc === null) return null;
    const next = desiredAsUtc - (displayedAsUtc - candidate);
    if (next === candidate) break;
    candidate = next;
  }
  if (!sameWallClock(wallClockAt(candidate, zone), desired)) return null;

  const offsets = new Set<number>();
  for (const dayDelta of [-2, -1, 0, 1, 2]) {
    const sample = candidate + dayDelta * 24 * 60 * 60 * 1000;
    const displayedAsUtc = wallClockUtcMs(wallClockAt(sample, zone));
    if (displayedAsUtc !== null) offsets.add(displayedAsUtc - sample);
  }
  const matches = new Set<number>();
  for (const offset of offsets) {
    const instant = desiredAsUtc - offset;
    if (sameWallClock(wallClockAt(instant, zone), desired)) {
      matches.add(instant);
    }
  }

  if (matches.size !== 1) return null;
  const instant = matches.values().next().value;
  return instant === undefined ? null : Math.floor(instant / 1000);
}
