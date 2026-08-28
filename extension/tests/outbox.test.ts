import { describe, expect, it } from "vitest";
import {
  acknowledge,
  enqueue,
  pending,
  type OutboxItem,
  type StorageAdapter,
} from "../src/outbox";

function memory(): StorageAdapter {
  const data = new Map<string, unknown>();
  return {
    async get(key) {
      return data.get(key);
    },
    async set(key, value) {
      data.set(key, value);
    },
  };
}

function item(id: string, bytes = 10): OutboxItem {
  return {
    captureId: id,
    sourceApp: "gmail",
    rawMessage: "x".repeat(bytes),
    sentAt: Date.now(),
    bytes,
  };
}

describe("durable outbox", () => {
  it("retries until ack and isolates profiles by storage adapter", async () => {
    const a = memory();
    const b = memory();
    await enqueue(item("cap-1"), a);
    await enqueue(item("cap-2"), b);
    expect((await pending(a)).map((row) => row.captureId)).toEqual(["cap-1"]);
    await acknowledge("cap-1", a);
    expect(await pending(a)).toEqual([]);
    expect((await pending(b)).map((row) => row.captureId)).toEqual(["cap-2"]);
  });

  it("drops oldest items on overflow", async () => {
    const storage = memory();
    const huge: OutboxItem = {
      captureId: "old",
      sourceApp: "slack",
      rawMessage: "old",
      sentAt: 1,
      bytes: 5 * 1024 * 1024,
    };
    await enqueue(huge, storage);
    await enqueue(item("new", 100), storage);
    const ids = (await pending(storage)).map((row) => row.captureId);
    expect(ids).toContain("new");
    expect(ids).not.toContain("old");
  });
});
