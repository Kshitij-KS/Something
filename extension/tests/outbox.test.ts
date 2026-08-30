import { describe, expect, it } from "vitest";
import {
  acknowledge,
  captureIdFor,
  enqueue,
  pending,
  removeSource,
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

function item(
  id: string,
  bytes = 10,
  sourceApp: OutboxItem["sourceApp"] = "gmail",
): OutboxItem {
  return {
    captureId: id,
    sourceApp,
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

  it("serializes concurrent storage updates without losing captures", async () => {
    const storage = memory();
    await Promise.all([
      enqueue(item("cap-concurrent-1"), storage),
      enqueue(item("cap-concurrent-2"), storage),
    ]);
    expect((await pending(storage)).map((row) => row.captureId)).toEqual([
      "cap-concurrent-1",
      "cap-concurrent-2",
    ]);
  });

  it("serializes concurrent source removals", async () => {
    const storage = memory();
    await enqueue(item("gmail-old"), storage);
    await enqueue(item("slack-old", 10, "slack"), storage);

    await Promise.all([
      removeSource("gmail", storage),
      removeSource("slack", storage),
    ]);

    expect(await pending(storage)).toEqual([]);
  });

  it("serializes source removal with enqueue", async () => {
    const storage = memory();
    await enqueue(item("gmail-old"), storage);
    await enqueue(item("slack-old", 10, "slack"), storage);

    await Promise.all([
      removeSource("gmail", storage),
      enqueue(item("slack-new", 10, "slack"), storage),
    ]);

    expect(
      (await pending(storage)).map(({ captureId, sourceApp }) => ({
        captureId,
        sourceApp,
      })),
    ).toEqual([
      { captureId: "slack-old", sourceApp: "slack" },
      { captureId: "slack-new", sourceApp: "slack" },
    ]);
  });

  it("keeps retries stable while distinguishing separate send intents", () => {
    expect(captureIdFor("intent-a")).toBe(captureIdFor("intent-a"));
    expect(captureIdFor("intent-a")).not.toBe(captureIdFor("intent-b"));
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
