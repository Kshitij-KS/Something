export type OutboxItem = {
  captureId: string;
  sourceApp: "gmail" | "slack";
  sourceCtx?: string;
  recipient?: string;
  rawMessage: string;
  sentAt: number;
  bytes: number;
};

export type StorageAdapter = {
  get: (key: string) => Promise<unknown>;
  set: (key: string, value: unknown) => Promise<void>;
};

const KEY = "callback.outbox";
const MAX_COUNT = 500;
const MAX_BYTES = 5 * 1024 * 1024;

export async function enqueue(
  item: OutboxItem,
  storage: StorageAdapter,
): Promise<void> {
  const items = await pending(storage);
  const next = [
    ...items.filter((existing) => existing.captureId !== item.captureId),
    item,
  ];
  const bounded = bound(next);
  await storage.set(KEY, bounded);
}

export async function acknowledge(
  captureId: string,
  storage: StorageAdapter,
): Promise<void> {
  const items = await pending(storage);
  await storage.set(
    KEY,
    items.filter((item) => item.captureId !== captureId),
  );
}

export async function pending(storage: StorageAdapter): Promise<OutboxItem[]> {
  const value = await storage.get(KEY);
  return Array.isArray(value) ? (value as OutboxItem[]) : [];
}

function bound(items: OutboxItem[]): OutboxItem[] {
  let bytes = items.reduce((sum, item) => sum + item.bytes, 0);
  const next = [...items];
  while (next.length > MAX_COUNT || bytes > MAX_BYTES) {
    const removed = next.shift();
    if (!removed) break;
    bytes -= removed.bytes;
  }
  return next;
}

export function captureIdFor(
  sourceApp: string,
  sourceCtx: string,
  body: string,
): string {
  const digest = simpleHash(`${sourceApp}|${sourceCtx}|${body}`);
  return `cap-${digest}`;
}

function simpleHash(value: string): string {
  let hash = 2166136261;
  for (const char of value) {
    hash ^= char.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(16);
}
