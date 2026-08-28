import selectors from "../selectors.json";
import {
  probeSelectors,
  shouldCapture,
  siteFromUrl,
  confirmSend,
} from "./capture";
import { snapshotGmail, gmailSendFromKeyboard } from "./content/gmail";
import { snapshotSlack, slackSendFromKeyboard } from "./content/slack";

const site = siteFromUrl(location.href);
if (site) {
  bind(site);
}

function bind(current: "gmail" | "slack") {
  const pack = selectors[current];
  const probe = probeSelectors(document, pack);
  chrome.runtime.sendMessage({
    type: "probe",
    site: current,
    ok: probe.ok,
    missed: probe.missed,
  });

  document.addEventListener(
    "click",
    (event) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (!matchesAny(target, pack.sendButton ?? [])) return;
      queueIntent(current, "click");
    },
    true,
  );

  document.addEventListener(
    "keydown",
    (event) => {
      if (event.isComposing) return;
      const send =
        current === "gmail"
          ? gmailSendFromKeyboard(event)
          : slackSendFromKeyboard(event);
      if (!send) return;
      queueIntent(current, "keyboard");
    },
    true,
  );

  reportContext(current);
  setInterval(() => reportContext(current), 2000);
}

function queueIntent(current: "gmail" | "slack", via: "click" | "keyboard") {
  const snap =
    current === "gmail"
      ? snapshotGmail(document, selectors.gmail)
      : snapshotSlack(document, selectors.slack);
  const decision = shouldCapture({ ...snap, via, composing: false });
  if (decision.type !== "intent") return;
  chrome.runtime.sendMessage({ type: "intent", decision });
  window.setTimeout(() => {
    const stillOpen =
      current === "gmail"
        ? snapshotGmail(document, selectors.gmail).rawMessage ===
          snap.rawMessage
        : snapshotSlack(document, selectors.slack).rawMessage ===
          snap.rawMessage;
    const confirm = confirmSend(decision.key, !stillOpen);
    chrome.runtime.sendMessage({
      type: "confirm",
      confirm,
      intent: decision.intent,
    });
  }, 400);
}

function reportContext(current: "gmail" | "slack") {
  const snap =
    current === "gmail"
      ? snapshotGmail(document, selectors.gmail)
      : snapshotSlack(document, selectors.slack);
  chrome.runtime.sendMessage({
    type: "context",
    sourceApp: current,
    sourceCtx: snap.sourceCtx,
    visible: document.visibilityState === "visible",
    active: document.hasFocus(),
  });
}

function matchesAny(node: Element, selectorsList: string[]): boolean {
  return selectorsList.some((selector) => node.closest(selector));
}
