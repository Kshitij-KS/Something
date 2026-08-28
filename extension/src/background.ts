import {
  acknowledge,
  captureIdFor,
  enqueue,
  pending,
  type OutboxItem,
  type StorageAdapter,
} from "./outbox";

const HOST = "com.callback.host";
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

function connect() {
  try {
    port = chrome.runtime.connectNative(HOST);
    port.onMessage.addListener(
      (message: {
        kind?: string;
        id?: string;
        payload?: { committed?: boolean };
      }) => {
        if (
          message.kind === "ack" &&
          message.payload?.committed &&
          message.id
        ) {
          void acknowledge(message.id, storage);
        }
      },
    );
    port.onDisconnect.addListener(() => {
      port = undefined;
      globalThis.setTimeout(connect, backoff);
      backoff = Math.min(backoff * 2, 10_000);
    });
    backoff = 500;
    flush().catch(() => undefined);
  } catch {
    globalThis.setTimeout(connect, backoff);
    backoff = Math.min(backoff * 2, 10_000);
  }
}

async function flush() {
  if (!port) return;
  for (const item of await pending(storage)) {
    port.postMessage({
      protocol_version: 1,
      kind: "capture",
      id: item.captureId,
      payload: item,
    });
  }
}

chrome.runtime.onMessage.addListener(
  (message: {
    type: string;
    confirm?: { type: string; key: string };
    intent?: {
      sourceApp: "gmail" | "slack";
      sourceCtx?: string;
      recipient?: string;
      rawMessage: string;
    };
    sourceApp?: string;
    sourceCtx?: string;
    visible?: boolean;
    active?: boolean;
    site?: string;
    ok?: boolean;
  }) => {
    if (
      message.type === "confirm" &&
      message.confirm?.type === "confirm" &&
      message.intent
    ) {
      const item: OutboxItem = {
        captureId: captureIdFor(
          message.intent.sourceApp,
          message.intent.sourceCtx ?? "",
          message.intent.rawMessage,
        ),
        sourceApp: message.intent.sourceApp,
        sourceCtx: message.intent.sourceCtx,
        recipient: message.intent.recipient,
        rawMessage: message.intent.rawMessage,
        sentAt: Date.now(),
        bytes: new TextEncoder().encode(message.intent.rawMessage).length,
      };
      void enqueue(item, storage).then(flush);
    }
    if (message.type === "context" && port) {
      port.postMessage({
        protocol_version: 1,
        kind: "context",
        id: `ctx-${Date.now()}`,
        payload: {
          source_app: message.sourceApp,
          source_ctx: message.sourceCtx,
          visible: message.visible,
          active: message.active,
        },
      });
    }
    if (message.type === "probe") {
      void chrome.storage.local.set({
        [`callback.probe.${message.site}`]: { ok: message.ok, at: Date.now() },
      });
    }
  },
);

connect();
