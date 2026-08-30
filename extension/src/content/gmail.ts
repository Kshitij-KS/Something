import type { SelectorPack, SendIntent } from "../capture";
import { firstMatch } from "../capture";

export type GmailAttempt = {
  intent: Omit<SendIntent, "via" | "composing" | "composerId">;
  body: Element;
  container: Element;
  successProbe: Element | null;
};

export function resolveGmailAttempt(
  target: Element,
  selectors: SelectorPack["gmail"],
): GmailAttempt | null {
  const bodyFromTarget = closestAny(target, selectors.composeBody ?? []);
  const container = closestAny(
    bodyFromTarget ?? target,
    selectors.composeContainer ?? [],
  );
  if (!container) return null;
  const body =
    bodyFromTarget ?? firstMatch(container, selectors.composeBody ?? []);
  if (!body || !container.contains(body)) return null;

  const to = firstMatch(container, selectors.toField ?? []);
  const recipient =
    to instanceof HTMLInputElement
      ? to.value.trim()
      : to?.textContent?.trim() || undefined;
  return {
    intent: {
      sourceApp: "gmail",
      sourceCtx: document.location.pathname,
      recipient: recipient || undefined,
      rawMessage: authoredText(body),
    },
    body,
    container,
    successProbe: firstMatch(document, selectors.successProbe ?? []),
  };
}

export function gmailSendFromKeyboard(event: KeyboardEvent): boolean {
  return (
    event.key === "Enter" &&
    (event.ctrlKey || event.metaKey) &&
    !event.isComposing
  );
}

function closestAny(node: Element, selectors: string[]): Element | null {
  for (const selector of selectors) {
    const match = node.closest(selector);
    if (match) return match;
  }
  return null;
}

function authoredText(body: Element): string {
  const clone = body.cloneNode(true) as Element;
  for (const quoted of clone.querySelectorAll(
    "blockquote, .gmail_quote, [data-smartmail='gmail_quote']",
  )) {
    quoted.remove();
  }
  return clone.textContent?.trim() ?? "";
}
