import selectors from "../selectors.json";
import {
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
  resolveSlackAttempt,
  slackRoute,
  slackSendFromKeyboard,
  type SlackAttempt,
} from "./content/slack";

const POLICY_KEY = "callback.sitePolicy";
type Site = "gmail" | "slack";
type SitePolicy = Record<Site, boolean>;

const composerIds = new WeakMap<Element, string>();
const composing = new WeakSet<Element>();
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
  document.addEventListener(
    "click",
    (event) => {
      if (!enabled) return;
      const target = event.target;
      if (!(target instanceof Element)) return;
      const pack = selectors[current];
      if (!matchesAny(target, pack.sendButton ?? [])) return;
      queueIntent(current, "click", target, false);
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
      queueIntent(current, "keyboard", event.target, event.isComposing);
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

function queueIntent(
  current: Site,
  via: "click" | "keyboard",
  target: Element,
  eventComposing: boolean,
) {
  const attempt =
    current === "gmail"
      ? resolveGmailAttempt(target, selectors.gmail)
      : resolveSlackAttempt(target, selectors.slack);
  if (!attempt) return;
  const body = attempt.body;
  const composerId = composerIds.get(body) ?? globalThis.crypto.randomUUID();
  composerIds.set(body, composerId);
  const decision = shouldCapture({
    ...attempt.intent,
    via,
    composing: eventComposing || composing.has(body),
    composerId,
  });
  if (decision.type !== "intent") return;

  const confirmation =
    current === "gmail"
      ? confirmGmailAttempt(
          attempt as GmailAttempt,
          selectors.gmail.successProbe ?? [],
        )
      : confirmSlackAttempt(attempt as SlackAttempt);
  void confirmation.then((succeeded) => {
    if (!enabled) {
      confirmSend(decision.key, false);
      return;
    }
    const confirm = confirmSend(decision.key, succeeded);
    chrome.runtime.sendMessage({
      type: "confirm",
      confirm,
      intent: decision.intent,
    });
    reportProbe(current);
    reportContext(current);
  });
}

function confirmGmailAttempt(
  attempt: GmailAttempt,
  successSelectors: string[],
): Promise<boolean> {
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
        (attempt.body.textContent?.trim() ?? "") === "";
      if (composeCompleted && successMutation) finish(true);
    };
    const observer = new MutationObserver((mutations) => {
      successMutation ||= mutations.some((mutation) => {
        const target = mutation.target;
        if (
          attempt.successProbe &&
          (target === attempt.successProbe ||
            attempt.successProbe.contains(target))
        ) {
          return true;
        }
        return mutation.addedNodes
          ? [...mutation.addedNodes].some(
              (node) =>
                node instanceof Element &&
                (matchesSelf(node, successSelectors) ||
                  Boolean(firstMatch(node, successSelectors))),
            )
          : false;
      });
      check();
    });
    observer.observe(document.documentElement, {
      childList: true,
      characterData: true,
      subtree: true,
    });
    const timeout = globalThis.setTimeout(() => finish(false), 2_000);
  });
}

function confirmSlackAttempt(attempt: SlackAttempt): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false;
    let settleTimer: ReturnType<typeof globalThis.setTimeout> | undefined;
    const finish = (result: boolean) => {
      if (settled) return;
      settled = true;
      observer.disconnect();
      globalThis.clearTimeout(timeout);
      if (settleTimer !== undefined) globalThis.clearTimeout(settleTimer);
      resolve(result);
    };
    const check = () => {
      const empty =
        !attempt.body.isConnected ||
        (attempt.body.textContent?.trim() ?? "") === "";
      if (empty && settleTimer === undefined) {
        settleTimer = globalThis.setTimeout(() => finish(true), 250);
      } else if (!empty && settleTimer !== undefined) {
        globalThis.clearTimeout(settleTimer);
        settleTimer = undefined;
      }
    };
    const observer = new MutationObserver(check);
    observer.observe(attempt.scope, {
      childList: true,
      characterData: true,
      subtree: true,
    });
    const timeout = globalThis.setTimeout(() => finish(false), 2_000);
  });
}

function reportProbe(current: Site) {
  if (!enabled) return;
  let required: Record<string, string[]> = {};
  if (current === "gmail") {
    const pack = selectors.gmail;
    if (firstMatch(document, pack.composeBody ?? [])) {
      required = {
        composeBody: pack.composeBody ?? [],
        composeContainer: pack.composeContainer ?? [],
        sendButton: pack.sendButton ?? [],
      };
    }
  } else {
    const pack = selectors.slack;
    if (firstMatch(document, pack.composeBody ?? [])) {
      required = {
        composeBody: pack.composeBody ?? [],
        sendButton: pack.sendButton ?? [],
      };
    }
  }
  const probe = probeSelectors(document, required);
  chrome.runtime.sendMessage({
    type: "probe",
    site: current,
    ok: probe.ok,
    missed: probe.missed,
  });
}

function reportContext(current: Site) {
  if (!enabled) return;
  chrome.runtime.sendMessage({
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
