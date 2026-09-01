import type { SelectorPack, SendIntent } from "../capture";
import { firstMatch } from "../capture";

const SLACK_EDITABLE_SELECTOR = "[contenteditable='true'][role='textbox']";

export type SlackAttempt = {
  intent: Omit<SendIntent, "via" | "composing" | "composerId">;
  body: Element;
  scope: Element;
};

export function resolveSlackAttempt(
  target: Element,
  selectors: SelectorPack["slack"],
  preferredBody?: Element | null,
): SlackAttempt | null {
  const bodyFromTarget = canonicalSlackBody(target, selectors);
  const preferred =
    preferredBody?.isConnected === true
      ? canonicalSlackBody(preferredBody, selectors)
      : null;
  const resolved = bodyFromTarget
    ? {
        body: bodyFromTarget,
        scope: nearestStableScope(bodyFromTarget, selectors),
      }
    : (bodyWithinClickScope(target, selectors) ??
      (preferred
        ? {
            body: preferred,
            scope: nearestStableScope(preferred, selectors),
          }
        : null));
  if (!resolved?.scope || !resolved.scope.contains(resolved.body)) return null;

  const channel = firstMatch(document, selectors.channelContext ?? []);
  const thread = closestAny(resolved.body, selectors.threadContext ?? []);
  const route = slackRoute(document.location.pathname);
  const context = thread
    ? `${route ?? "slack"}:thread:${thread.id || "active"}`
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

export function canonicalSlackBody(
  node: Element,
  selectors: SelectorPack["slack"],
): Element | null {
  const editable = node.closest(SLACK_EDITABLE_SELECTOR);
  if (editable) return editable;
  const candidate = closestAny(node, selectors.composeBody ?? []);
  return candidate ? canonicalizeCandidate(candidate) : null;
}

export function slackBodiesWithin(
  root: ParentNode,
  selectors: SelectorPack["slack"],
): Element[] {
  const candidates = new Set<Element>();
  if (root instanceof Element) {
    for (const selector of selectors.composeBody ?? []) {
      if (root.matches(selector)) candidates.add(root);
    }
  }
  for (const selector of selectors.composeBody ?? []) {
    for (const candidate of root.querySelectorAll(selector)) {
      candidates.add(candidate);
    }
  }

  const canonical = new Set<Element>();
  for (const candidate of candidates) {
    canonical.add(canonicalizeCandidate(candidate));
  }
  return [...canonical];
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
  while (scope) {
    const bodies = slackBodiesWithin(scope, selectors);
    const [body] = bodies;
    if (bodies.length === 1 && body) return { body, scope };
    if (bodies.length > 1) return null;
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

  const messageInput = body.closest("[data-qa='message_input']");
  if (messageInput) {
    return messageInput === body
      ? (messageInput.parentElement ?? messageInput)
      : messageInput;
  }

  let scope = body.parentElement ?? body;
  while (scope.parentElement) {
    if (allMatches(scope, selectors.sendButton ?? []).length > 0) return scope;
    scope = scope.parentElement;
  }
  return body.parentElement ?? body;
}

function canonicalizeCandidate(candidate: Element): Element {
  if (candidate.matches(SLACK_EDITABLE_SELECTOR)) return candidate;
  return candidate.querySelector(SLACK_EDITABLE_SELECTOR) ?? candidate;
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
