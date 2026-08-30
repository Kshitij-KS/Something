import type { SelectorPack, SendIntent } from "../capture";
import { firstMatch } from "../capture";

export type SlackAttempt = {
  intent: Omit<SendIntent, "via" | "composing" | "composerId">;
  body: Element;
  scope: Element;
};

export function resolveSlackAttempt(
  target: Element,
  selectors: SelectorPack["slack"],
): SlackAttempt | null {
  const bodyFromTarget = closestAny(target, selectors.composeBody ?? []);
  const resolved = bodyFromTarget
    ? {
        body: bodyFromTarget,
        scope: nearestStableScope(bodyFromTarget, selectors),
      }
    : bodyWithinClickScope(target, selectors);
  if (!resolved?.scope || !resolved.scope.contains(resolved.body)) return null;

  const channel = firstMatch(document, selectors.channelContext ?? []);
  const thread = closestAny(resolved.scope, selectors.threadContext ?? []);
  const route = slackRoute(document.location.pathname);
  const context = thread?.id
    ? `${route ?? "slack"}:thread:${thread.id}`
    : route;
  return {
    intent: {
      sourceApp: "slack",
      sourceCtx: context,
      recipient: channel?.textContent?.trim() || undefined,
      rawMessage: authoredText(resolved.body),
    },
    body: resolved.body,
    scope: resolved.scope,
  };
}

export function slackSendFromKeyboard(event: KeyboardEvent): boolean {
  return event.key === "Enter" && !event.shiftKey && !event.isComposing;
}

export function slackRoute(pathname: string): string | undefined {
  const parts = pathname.split("/").filter(Boolean);
  const client = parts.indexOf("client");
  const team = parts[client + 1];
  const conversation = parts[client + 2];
  return team && conversation ? `${team}:${conversation}` : undefined;
}

function bodyWithinClickScope(
  target: Element,
  selectors: SelectorPack["slack"],
): { body: Element; scope: Element } | null {
  let scope: Element | null = target;
  for (let depth = 0; scope && depth < 10; depth += 1) {
    const bodies = allMatches(scope, selectors.composeBody ?? []);
    const [body] = bodies;
    if (bodies.length === 1 && body) return { body, scope };
    scope = scope.parentElement;
  }
  return null;
}

function nearestStableScope(
  body: Element,
  selectors: SelectorPack["slack"],
): Element {
  const thread = closestAny(body, selectors.threadContext ?? []);
  if (thread) return thread;
  let scope = body.parentElement ?? body;
  for (let depth = 0; scope.parentElement && depth < 6; depth += 1) {
    if (allMatches(scope, selectors.sendButton ?? []).length > 0) return scope;
    scope = scope.parentElement;
  }
  return body.parentElement ?? body;
}

function allMatches(root: ParentNode, selectors: string[]): Element[] {
  const matches = new Set<Element>();
  for (const selector of selectors) {
    for (const element of root.querySelectorAll(selector)) matches.add(element);
  }
  return [...matches];
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
  for (const quoted of clone.querySelectorAll("blockquote")) quoted.remove();
  return clone.textContent?.trim() ?? "";
}
