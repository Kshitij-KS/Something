export type SelectorPack = {
  version: number;
  gmail: Record<string, string[]>;
  slack: Record<string, string[]>;
};

export type SendIntent = {
  sourceApp: "gmail" | "slack";
  sourceCtx?: string;
  recipient?: string;
  rawMessage: string;
  via: "click" | "keyboard";
  composing: boolean;
  composerId?: string;
};

export type CaptureDecision =
  | { type: "ignore"; reason: string }
  | { type: "intent"; key: string; intent: SendIntent }
  | { type: "confirm"; key: string };

const recent = new Map<string, number>();
const pendingDedupe = new Map<string, string>();
const pendingKeys = new Set<string>();

export function canonicalCaptureText(text: string): string {
  return text.replaceAll("\u200B", "").replace(/\s+/g, " ").trim();
}

export function shouldCapture(event: SendIntent): CaptureDecision {
  if (event.composing) {
    return { type: "ignore", reason: "ime" };
  }
  if (!canonicalCaptureText(event.rawMessage)) {
    return { type: "ignore", reason: "empty" };
  }
  const dedupeKey = JSON.stringify([
    event.sourceApp,
    event.composerId ?? "unknown",
    event.sourceCtx ?? "",
    canonicalCaptureText(event.recipient ?? ""),
    event.rawMessage.replaceAll("\u200B", ""),
  ]);
  const last = recent.get(dedupeKey);
  const now = Date.now();
  if (pendingKeys.has(dedupeKey) || (last && now - last < 2000)) {
    return { type: "ignore", reason: "duplicate" };
  }
  recent.set(dedupeKey, now);
  pendingKeys.add(dedupeKey);
  const key = `${now.toString(36)}-${globalThis.crypto.randomUUID()}`;
  pendingDedupe.set(key, dedupeKey);
  return { type: "intent", key, intent: event };
}

export function confirmSend(key: string, succeeded: boolean): CaptureDecision {
  const dedupeKey = pendingDedupe.get(key);
  pendingDedupe.delete(key);
  if (dedupeKey) pendingKeys.delete(dedupeKey);
  if (!succeeded) {
    if (dedupeKey) recent.delete(dedupeKey);
    return { type: "ignore", reason: "failed_send" };
  }
  return { type: "confirm", key };
}

export function firstMatch(
  root: ParentNode,
  selectors: string[],
): Element | null {
  for (const selector of selectors) {
    const node = root.querySelector(selector);
    if (node) return node;
  }
  return null;
}

export function probeSelectors(
  root: ParentNode,
  pack: Record<string, string[]>,
): { ok: boolean; missed: string[] } {
  const missed: string[] = [];
  for (const [name, selectors] of Object.entries(pack)) {
    if (!firstMatch(root, selectors)) missed.push(name);
  }
  return { ok: missed.length === 0, missed };
}

export function siteFromUrl(url: string): "gmail" | "slack" | null {
  if (url.includes("mail.google.com")) return "gmail";
  if (url.includes("app.slack.com")) return "slack";
  return null;
}
