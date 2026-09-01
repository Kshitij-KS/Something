import {
  acknowledge,
  captureIdFor,
  enqueue,
  pending,
  removeSource,
  type OutboxItem,
  type StorageAdapter,
} from "./outbox";

const HOST = "com.callback.host";
const POLICY_KEY = "callback.sitePolicy";
const SLACK_CAPTURE_STAGE_KEY = "callback.captureStage.slack";
const SLACK_CAPTURE_STAGES = new Set([
  "gesture_seen",
  "attempt_missing",
  "attempt_resolved",
  "intent_ignored",
  "confirmation_waiting",
  "body_emptied",
  "body_detached",
  "successor_adopted",
  "confirmation_timeout",
  "confirm_emitted",
]);
const CAPTURE_IGNORE_REASONS = new Set(["empty", "ime", "duplicate"]);
type Site = "gmail" | "slack";
type SitePolicy = Record<Site, boolean>;

const storage: StorageAdapter = {
  async get(key) {
    const result = await chrome.storage.local.get(key);
    return result[key];
  },
  async set(key, value) {
    await chrome.storage.local.set({ [key]: value });
  },
};

let port: chrome.runtime.Port | undefined;
let backoff = 500;
let sitePolicy: SitePolicy = { gmail: false, slack: false };

function isSitePolicy(value: unknown): value is SitePolicy {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<SitePolicy>;
  return (
    typeof candidate.gmail === "boolean" && typeof candidate.slack === "boolean"
  );
}

async function applySitePolicy(next: SitePolicy): Promise<void> {
  sitePolicy = next;
  await storage.set(POLICY_KEY, next);
  await Promise.all(
    (Object.entries(next) as [Site, boolean][])
      .filter(([, enabled]) => !enabled)
      .map(([site]) => removeSource(site, storage)),
  );
}

function connect() {
  try {
    const connectedPort = chrome.runtime.connectNative(HOST);
    const handshakeId = `hs-${globalThis.crypto.randomUUID()}`;
    port = connectedPort;
    connectedPort.onMessage.addListener(
      (message: {
        kind?: string;
        id?: string;
        payload?: {
          committed?: boolean;
          discard?: boolean;
          site_policy?: unknown;
        };
      }) => {
        if (message.kind !== "ack" || !message.id) return;
        const updatePolicy = isSitePolicy(message.payload?.site_policy)
          ? applySitePolicy(message.payload.site_policy)
          : Promise.resolve();
        if (message.id === handshakeId) {
          backoff = 500;
          void updatePolicy.then(() => flush(connectedPort));
          return;
        }
        if (
          message.id.startsWith("cap-") &&
          (message.payload?.committed || message.payload?.discard)
        ) {
          void updatePolicy.then(() => acknowledge(message.id!, storage));
        }
      },
    );
    connectedPort.onDisconnect.addListener(() => {
      void chrome.runtime.lastError;
      if (port === connectedPort) port = undefined;
      scheduleReconnect();
    });
    connectedPort.postMessage({
      protocol_version: 1,
      kind: "handshake",
      id: handshakeId,
      payload: {
        extension_version: chrome.runtime.getManifest().version,
      },
    });
  } catch {
    scheduleReconnect();
  }
}

function scheduleReconnect() {
  const delay = backoff;
  backoff = Math.min(backoff * 2, 10_000);
  globalThis.setTimeout(() => {
    if (!port) connect();
  }, delay);
}

async function flush(target = port) {
  if (!target) return;
  for (const item of await pending(storage)) {
    if (!sitePolicy[item.sourceApp]) continue;
    target.postMessage({
      protocol_version: 1,
      kind: "capture",
      id: item.captureId,
      payload: item,
    });
  }
}

function senderMatchesSite(site: Site, senderUrl?: string): boolean {
  if (!senderUrl) return false;
  try {
    const host = new URL(senderUrl).hostname;
    return site === "gmail"
      ? host === "mail.google.com"
      : host === "app.slack.com";
  } catch {
    return false;
  }
}

type RuntimeMessage = {
  type: string;
  confirm?: { type: string; key: string };
  intent?: {
    sourceApp: Site;
    sourceCtx?: string;
    recipient?: string;
    rawMessage: string;
  };
  sourceApp?: Site;
  sourceCtx?: string;
  visible?: boolean;
  active?: boolean;
  site?: Site;
  ok?: boolean;
  missed?: string[];
  stage?: string;
  at?: number;
  via?: string;
  reason?: string;
  bodyConnected?: boolean;
  scopeConnected?: boolean;
};

async function hydrateSitePolicy(): Promise<void> {
  const stored = await storage.get(POLICY_KEY);
  if (isSitePolicy(stored)) sitePolicy = stored;
}

const policyReady = hydrateSitePolicy().catch(() => undefined);

async function handleRuntimeMessage(
  message: RuntimeMessage,
  sender: chrome.runtime.MessageSender,
): Promise<void> {
  await policyReady;
  if (
    message.type === "confirm" &&
    message.confirm?.type === "confirm" &&
    message.intent &&
    sitePolicy[message.intent.sourceApp] &&
    senderMatchesSite(message.intent.sourceApp, sender.tab?.url)
  ) {
    const sentAt = Date.now();
    const item: OutboxItem = {
      captureId: captureIdFor(message.confirm.key),
      sourceApp: message.intent.sourceApp,
      sourceCtx: message.intent.sourceCtx,
      recipient: message.intent.recipient,
      rawMessage: message.intent.rawMessage,
      sentAt,
      bytes: new TextEncoder().encode(message.intent.rawMessage).length,
    };
    await enqueue(item, storage);
    if (message.intent.sourceApp === "slack") {
      await storage.set(SLACK_CAPTURE_STAGE_KEY, {
        stage: "background_accepted",
        at: Date.now(),
      });
    }
    await flush();
  }
  if (
    message.type === "context" &&
    message.sourceApp &&
    sitePolicy[message.sourceApp] &&
    port &&
    sender.tab?.active &&
    senderMatchesSite(message.sourceApp, sender.tab.url)
  ) {
    port.postMessage({
      protocol_version: 1,
      kind: "context",
      id: `ctx-${Date.now()}-${sender.tab.id ?? "unknown"}`,
      payload: {
        source_app: message.sourceApp,
        source_ctx: message.sourceCtx,
        visible: message.visible,
        active: message.active,
      },
    });
  }
  if (
    message.type === "captureStage" &&
    message.site === "slack" &&
    message.stage &&
    SLACK_CAPTURE_STAGES.has(message.stage) &&
    sitePolicy.slack &&
    senderMatchesSite("slack", sender.tab?.url)
  ) {
    const stageRecord: Record<string, string | number | boolean> = {
      stage: message.stage,
      at: Date.now(),
    };
    if (message.via === "click" || message.via === "keyboard") {
      stageRecord.via = message.via;
    }
    if (message.reason && CAPTURE_IGNORE_REASONS.has(message.reason)) {
      stageRecord.reason = message.reason;
    }
    if (typeof message.bodyConnected === "boolean") {
      stageRecord.body_connected = message.bodyConnected;
    }
    if (typeof message.scopeConnected === "boolean") {
      stageRecord.scope_connected = message.scopeConnected;
    }
    await storage.set(SLACK_CAPTURE_STAGE_KEY, stageRecord);
  }
  if (
    message.type === "probe" &&
    message.site &&
    sitePolicy[message.site] &&
    senderMatchesSite(message.site, sender.tab?.url)
  ) {
    const observedAt = Date.now();
    await storage.set(`callback.probe.${message.site}`, {
      ok: message.ok,
      at: observedAt,
    });
    port?.postMessage({
      protocol_version: 1,
      kind: "probe",
      id: `probe-${message.site}-${observedAt}`,
      payload: {
        site: message.site,
        ok: message.ok,
        missed_count: message.missed?.length ?? 0,
        observed_at: observedAt,
      },
    });
  }
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  void handleRuntimeMessage(message as RuntimeMessage, sender).then(
    () => sendResponse({ ok: true }),
    () => sendResponse({ ok: false }),
  );
  return true;
});

async function initialize() {
  await policyReady;
  connect();
}

// Keeps policy fresh even when disabled content scripts emit no envelopes.
globalThis.setInterval(() => {
  port?.postMessage({
    protocol_version: 1,
    kind: "reconnect",
    id: `policy-${Date.now()}`,
    payload: {},
  });
}, 30_000);

void initialize();
