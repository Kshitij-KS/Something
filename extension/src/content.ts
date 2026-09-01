import selectors from "../selectors.json";
import {
  canonicalCaptureText,
  confirmSend,
  firstMatch,
  probeSelectors,
  shouldCapture,
  siteFromUrl,
} from "./capture";
import {
  gmailSendFromKeyboard,
  resolveGmailAttempt,
  type GmailAttempt,
} from "./content/gmail";
import {
  canonicalSlackBody,
  resolveSlackAttempt,
  slackBodiesWithin,
  slackRoute,
  slackSendFromKeyboard,
  type SlackAttempt,
} from "./content/slack";

const POLICY_KEY = "callback.sitePolicy";
const SEND_CONFIRM_TIMEOUT_MS = 10_000;
const SEND_SETTLE_MS = 250;
const SLACK_CLICK_HANDOFF_MS = 2_000;
type Site = "gmail" | "slack";
type SitePolicy = Record<Site, boolean>;
type PendingSlackClick = { body: Element; at: number };

const composerIds = new WeakMap<Element, string>();
const composing = new WeakSet<Element>();
let pendingSlackClick: PendingSlackClick | null = null;
const site = siteFromUrl(location.href);
let enabled = false;

if (site) void initialize(site);

async function initialize(current: Site) {
  const stored = await chrome.storage.local.get(POLICY_KEY);
  enabled = isSitePolicy(stored[POLICY_KEY]) && stored[POLICY_KEY][current];
  bind(current);
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area !== "local") return;
    const next = changes[POLICY_KEY]?.newValue;
    if (!isSitePolicy(next)) return;
    const wasEnabled = enabled;
    enabled = next[current];
    if (!wasEnabled && enabled) {
      reportProbe(current);
      reportContext(current);
    }
  });
}

function isSitePolicy(value: unknown): value is SitePolicy {
  if (!value || typeof value !== "object") return false;
  const policy = value as Partial<SitePolicy>;
  return typeof policy.gmail === "boolean" && typeof policy.slack === "boolean";
}

function bind(current: Site) {
  document.addEventListener(
    "compositionstart",
    (event) => {
      if (event.target instanceof Element) composing.add(event.target);
    },
    true,
  );
  document.addEventListener(
    "compositionend",
    (event) => {
      if (event.target instanceof Element) composing.delete(event.target);
    },
    true,
  );
  if (current === "slack") {
    document.addEventListener(
      "pointerdown",
      (event) => {
        pendingSlackClick = null;
        const target = event.target;
        if (
          !(target instanceof Element) ||
          !matchesAny(target, selectors.slack.sendButton ?? [])
        ) {
          return;
        }
        const active = document.activeElement;
        if (!(active instanceof Element)) return;
        const body = canonicalSlackBody(active, selectors.slack);
        if (body) pendingSlackClick = { body, at: Date.now() };
      },
      true,
    );
  }
  document.addEventListener(
    "click",
    (event) => {
      if (!enabled) return;
      const target = event.target;
      if (!(target instanceof Element)) return;
      const pack = selectors[current];
      if (!matchesAny(target, pack.sendButton ?? [])) return;
      queueIntent(
        current,
        "click",
        target,
        false,
        current === "slack" ? takePendingSlackBody() : null,
      );
    },
    true,
  );
  document.addEventListener(
    "keydown",
    (event) => {
      if (!enabled || event.isComposing) return;
      const send =
        current === "gmail"
          ? gmailSendFromKeyboard(event)
          : slackSendFromKeyboard(event);
      if (!send || !(event.target instanceof Element)) return;
      queueIntent(current, "keyboard", event.target, event.isComposing, null);
    },
    true,
  );

  for (const eventName of [
    "visibilitychange",
    "focus",
    "blur",
    "hashchange",
    "popstate",
  ]) {
    globalThis.addEventListener(eventName, () => reportContext(current));
  }
  reportProbe(current);
  reportContext(current);
  globalThis.setInterval(() => reportContext(current), 2_000);
  globalThis.setInterval(() => reportProbe(current), 30_000);
}

function takePendingSlackBody(): Element | null {
  const pending = pendingSlackClick;
  pendingSlackClick = null;
  if (
    !pending ||
    !pending.body.isConnected ||
    Date.now() - pending.at > SLACK_CLICK_HANDOFF_MS
  ) {
    return null;
  }
  return pending.body;
}

function queueIntent(
  current: Site,
  via: "click" | "keyboard",
  target: Element,
  eventComposing: boolean,
  preferredSlackBody: Element | null,
) {
  if (current === "slack") reportSlackStage("gesture_seen", { via });
  const attempt =
    current === "gmail"
      ? resolveGmailAttempt(target, selectors.gmail)
      : resolveSlackAttempt(target, selectors.slack, preferredSlackBody);
  if (!attempt) {
    if (current === "slack") reportSlackStage("attempt_missing", { via });
    reportProbe(current, ["composeScope"]);
    return;
  }
  const body = attempt.body;
  const composerId = composerIds.get(body) ?? globalThis.crypto.randomUUID();
  composerIds.set(body, composerId);
  if (current === "slack") {
    reportSlackStage("attempt_resolved", {
      via,
      bodyConnected: body.isConnected,
      scopeConnected: "scope" in attempt && attempt.scope.isConnected,
    });
  }
  const decision = shouldCapture({
    ...attempt.intent,
    via,
    composing: eventComposing || composing.has(body),
    composerId,
  });
  if (decision.type !== "intent") {
    if (current === "slack" && decision.type === "ignore") {
      reportSlackStage("intent_ignored", { via, reason: decision.reason });
    }
    return;
  }
  if (current === "slack") reportSlackStage("confirmation_waiting", { via });

  const confirmation =
    "container" in attempt
      ? confirmGmailAttempt(attempt)
      : confirmSlackAttempt(attempt);
  void confirmation.then((succeeded) => {
    if (!enabled) {
      confirmSend(decision.key, false);
      return;
    }
    const confirm = confirmSend(decision.key, succeeded);
    if (confirm.type === "confirm") {
      if (current === "slack") reportSlackStage("confirm_emitted", { via });
      void chrome.runtime.sendMessage({
        type: "confirm",
        confirm,
        intent: decision.intent,
      });
      reportProbe(current);
    } else {
      if (current === "slack") {
        reportSlackStage("confirmation_timeout", { via });
      }
      reportProbe(current, ["sendConfirmation"]);
    }
    reportContext(current);
  });
}

function confirmGmailAttempt(attempt: GmailAttempt): Promise<boolean> {
  return new Promise((resolve) => {
    let successMutation = false;
    let settled = false;
    const finish = (result: boolean) => {
      if (settled) return;
      settled = true;
      observer.disconnect();
      globalThis.clearTimeout(timeout);
      resolve(result);
    };
    const check = () => {
      const composeCompleted =
        !attempt.container.isConnected ||
        !attempt.body.isConnected ||
        canonicalCaptureText(attempt.body.textContent ?? "") === "";
      if (composeCompleted && successMutation) finish(true);
    };
    const observer = new MutationObserver((mutations) => {
      successMutation ||= gmailMutationSignalsSend(
        mutations,
        attempt,
        selectors.gmail.successProbe ?? [],
      );
      check();
    });
    const timeout = globalThis.setTimeout(
      () => finish(false),
      SEND_CONFIRM_TIMEOUT_MS,
    );
    observer.observe(document.documentElement, {
      childList: true,
      characterData: true,
      subtree: true,
    });
    check();
  });
}

function confirmSlackAttempt(initialAttempt: SlackAttempt): Promise<boolean> {
  return new Promise((resolve) => {
    let activeAttempt = initialAttempt;
    let reportedDetached = false;
    let reportedSuccessor = false;
    let reportedEmpty = false;
    let settled = false;
    let settleTimer: ReturnType<typeof globalThis.setTimeout> | undefined;

    const completed = () => {
      if (
        !activeAttempt.body.isConnected ||
        !activeAttempt.scope.isConnected ||
        !activeAttempt.scope.contains(activeAttempt.body)
      ) {
        if (!reportedDetached) {
          reportedDetached = true;
          reportSlackStage("body_detached", {
            bodyConnected: activeAttempt.body.isConnected,
            scopeConnected: activeAttempt.scope.isConnected,
          });
        }
        const replacement = findReplacementSlackAttempt(
          initialAttempt,
          activeAttempt,
        );
        if (replacement) {
          activeAttempt = replacement;
          if (!reportedSuccessor) {
            reportedSuccessor = true;
            reportSlackStage("successor_adopted", {
              bodyConnected: replacement.body.isConnected,
              scopeConnected: replacement.scope.isConnected,
            });
          }
        }
      }
      const empty =
        activeAttempt.body.isConnected &&
        activeAttempt.scope.isConnected &&
        activeAttempt.scope.contains(activeAttempt.body) &&
        canonicalCaptureText(activeAttempt.body.textContent ?? "") === "";
      if (empty && !reportedEmpty) {
        reportedEmpty = true;
        reportSlackStage("body_emptied", {
          bodyConnected: true,
          scopeConnected: true,
        });
      }
      return empty;
    };
    const finish = (result: boolean) => {
      if (settled) return;
      settled = true;
      observer.disconnect();
      globalThis.clearTimeout(timeout);
      if (settleTimer !== undefined) globalThis.clearTimeout(settleTimer);
      resolve(result);
    };
    const check = () => {
      if (completed()) {
        settleTimer ??= globalThis.setTimeout(() => {
          settleTimer = undefined;
          if (completed()) finish(true);
        }, SEND_SETTLE_MS);
      } else if (settleTimer !== undefined) {
        globalThis.clearTimeout(settleTimer);
        settleTimer = undefined;
      }
    };
    const observer = new MutationObserver(check);
    const timeout = globalThis.setTimeout(
      () => finish(false),
      SEND_CONFIRM_TIMEOUT_MS,
    );
    observer.observe(document.documentElement, {
      childList: true,
      characterData: true,
      subtree: true,
    });
    check();
  });
}

function findReplacementSlackAttempt(
  initialAttempt: SlackAttempt,
  activeAttempt: SlackAttempt,
): SlackAttempt | null {
  if (activeAttempt.scope.isConnected) {
    const local = chooseReplacementSlackAttempt(
      slackAttemptsWithin(activeAttempt.scope, initialAttempt, activeAttempt),
      initialAttempt.intent.rawMessage,
    );
    if (local) return local;
  }
  return chooseReplacementSlackAttempt(
    slackAttemptsWithin(document, initialAttempt, activeAttempt),
    initialAttempt.intent.rawMessage,
  );
}

function slackAttemptsWithin(
  root: ParentNode,
  initialAttempt: SlackAttempt,
  activeAttempt: SlackAttempt,
): SlackAttempt[] {
  return slackBodiesWithin(root, selectors.slack)
    .filter((body) => body !== activeAttempt.body)
    .map((body) => resolveSlackAttempt(body, selectors.slack, body))
    .filter((candidate): candidate is SlackAttempt => candidate !== null)
    .filter(
      (candidate) =>
        candidate.body.isConnected &&
        candidate.scope.isConnected &&
        candidate.intent.sourceCtx === initialAttempt.intent.sourceCtx,
    );
}

function chooseReplacementSlackAttempt(
  attempts: SlackAttempt[],
  expectedText: string,
): SlackAttempt | null {
  const expected = canonicalCaptureText(expectedText);
  const exact = attempts.filter(
    (attempt) =>
      canonicalCaptureText(attempt.body.textContent ?? "") === expected,
  );
  if (exact.length === 1) return exact[0] ?? null;
  return attempts.length === 1 ? (attempts[0] ?? null) : null;
}

function gmailMutationSignalsSend(
  mutations: MutationRecord[],
  attempt: GmailAttempt,
  successSelectors: string[],
): boolean {
  return mutations.some((mutation) => {
    const target = mutation.target;
    if (
      attempt.successProbe &&
      (target === attempt.successProbe || attempt.successProbe.contains(target))
    ) {
      return true;
    }
    return [...mutation.addedNodes].some(
      (node) =>
        node instanceof Element &&
        (matchesSelf(node, successSelectors) ||
          Boolean(firstMatch(node, successSelectors))),
    );
  });
}

type SlackCaptureStage =
  | "gesture_seen"
  | "attempt_missing"
  | "attempt_resolved"
  | "intent_ignored"
  | "confirmation_waiting"
  | "body_emptied"
  | "body_detached"
  | "successor_adopted"
  | "confirmation_timeout"
  | "confirm_emitted";

type SlackStageDetails = {
  via?: "click" | "keyboard";
  reason?: string;
  bodyConnected?: boolean;
  scopeConnected?: boolean;
};

function reportSlackStage(
  stage: SlackCaptureStage,
  details: SlackStageDetails = {},
) {
  if (!enabled) return;
  void chrome.runtime.sendMessage({
    type: "captureStage",
    site: "slack",
    stage,
    at: Date.now(),
    via: details.via,
    reason: details.reason,
    bodyConnected: details.bodyConnected,
    scopeConnected: details.scopeConnected,
  });
}

function reportProbe(current: Site, forcedMissed: string[] = []) {
  if (!enabled) return;
  if (forcedMissed.length > 0) {
    emitProbe(current, false, forcedMissed);
    return;
  }

  if (current === "gmail") {
    const pack = selectors.gmail;
    if (!firstMatch(document, pack.composeBody ?? [])) return;
    const probe = probeSelectors(document, {
      composeBody: pack.composeBody ?? [],
      composeContainer: pack.composeContainer ?? [],
      sendButton: pack.sendButton ?? [],
    });
    emitProbe(current, probe.ok, probe.missed);
    return;
  }

  const pack = selectors.slack;
  if (!firstMatch(document, pack.composeBody ?? [])) return;
  const probe = probeSelectors(document, {
    composeBody: pack.composeBody ?? [],
    sendButton: pack.sendButton ?? [],
  });
  emitProbe(current, probe.ok, probe.missed);
}

function emitProbe(current: Site, ok: boolean, missed: string[]) {
  void chrome.runtime.sendMessage({
    type: "probe",
    site: current,
    ok,
    missed,
  });
}

function reportContext(current: Site) {
  if (!enabled) return;
  void chrome.runtime.sendMessage({
    type: "context",
    sourceApp: current,
    sourceCtx:
      current === "gmail"
        ? document.location.pathname
        : slackRoute(document.location.pathname),
    visible: document.visibilityState === "visible",
    active: document.hasFocus(),
  });
}

function matchesAny(node: Element, selectorsList: string[]): boolean {
  return selectorsList.some((selector) => node.closest(selector));
}

function matchesSelf(node: Element, selectorsList: string[]): boolean {
  return selectorsList.some((selector) => node.matches(selector));
}
