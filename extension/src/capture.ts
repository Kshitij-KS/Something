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
};

export type CaptureDecision =
  | { type: "ignore"; reason: string }
  | { type: "intent"; key: string; intent: SendIntent }
  | { type: "confirm"; key: string };

const recent = new Map<string, number>();

export function shouldCapture(event: SendIntent): CaptureDecision {
  if (event.composing) {
    return { type: "ignore", reason: "ime" };
  }
  if (!event.rawMessage.trim()) {
    return { type: "ignore", reason: "empty" };
  }
  if (
    event.rawMessage.split("\n").some((line) => line.trim().startsWith(">"))
  ) {
    return { type: "ignore", reason: "quoted" };
  }
  const key = `${event.sourceApp}:${event.sourceCtx ?? ""}:${event.rawMessage}`;
  const last = recent.get(key);
  const now = Date.now();
  if (last && now - last < 2000) {
    return { type: "ignore", reason: "duplicate" };
  }
  recent.set(key, now);
  return { type: "intent", key, intent: event };
}

export function confirmSend(key: string, succeeded: boolean): CaptureDecision {
  if (!succeeded) {
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
