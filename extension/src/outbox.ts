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
const mutationQueues = new WeakMap<StorageAdapter, Promise<void>>();

export async function enqueue(
  item: OutboxItem,
  storage: StorageAdapter,
): Promise<void> {
  await mutate(storage, async () => {
    const items = await pending(storage);
    const next = [
      ...items.filter((existing) => existing.captureId !== item.captureId),
      item,
    ];
    await storage.set(KEY, bound(next));
  });
}

export async function acknowledge(
  captureId: string,
  storage: StorageAdapter,
): Promise<void> {
  await mutate(storage, async () => {
    const items = await pending(storage);
    await storage.set(
      KEY,
      items.filter((item) => item.captureId !== captureId),
    );
  });
}

export async function removeSource(
  sourceApp: "gmail" | "slack",
  storage: StorageAdapter,
): Promise<void> {
  await mutate(storage, async () => {
    const items = await pending(storage);
    await storage.set(
      KEY,
      items.filter((item) => item.sourceApp !== sourceApp),
    );
  });
}

export async function pending(storage: StorageAdapter): Promise<OutboxItem[]> {
  const value = await storage.get(KEY);
  return Array.isArray(value) ? (value as OutboxItem[]) : [];
}

function mutate(
  storage: StorageAdapter,
  operation: () => Promise<void>,
): Promise<void> {
  const previous = mutationQueues.get(storage) ?? Promise.resolve();
  const result = previous.then(operation, operation);
  mutationQueues.set(
    storage,
    result.then(
      () => undefined,
      () => undefined,
    ),
  );
  return result;
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

/** Creates a retry-stable capture ID from one unique send-intent key. */
export function captureIdFor(intentKey: string): string {
  return `cap-${intentKey}`;
}
