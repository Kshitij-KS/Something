import { afterEach, describe, expect, it } from "vitest";
import {
  confirmSend,
  probeSelectors,
  shouldCapture,
  siteFromUrl,
} from "../src/capture";
import { resolveGmailAttempt } from "../src/content/gmail";
import { resolveSlackAttempt } from "../src/content/slack";

afterEach(() => {
  document.body.replaceChildren();
  window.history.replaceState({}, "", "/");
});

describe("capture decisions", () => {
  it("captures gmail compose and reply keyboard send after confirmation", () => {
    const intent = shouldCapture({
      sourceApp: "gmail",
      sourceCtx: "thread-1",
      rawMessage: "I will send the invoice tomorrow",
      via: "keyboard",
      composing: false,
    });
    expect(intent.type).toBe("intent");
    if (intent.type !== "intent") return;
    expect(confirmSend(intent.key, true).type).toBe("confirm");
  });

  it("ignores IME composition", () => {
    expect(
      shouldCapture({
        sourceApp: "slack",
        rawMessage: "I will send it",
        via: "keyboard",
        composing: true,
      }),
    ).toEqual({ type: "ignore", reason: "ime" });
  });

  it("captures authored text beginning with a greater-than sign", () => {
    const decision = shouldCapture({
      sourceApp: "gmail",
      sourceCtx: "thread-greater-than",
      composerId: "compose-greater-than",
      rawMessage: "> quoted reply",
      via: "click",
      composing: false,
    });

    expect(decision).toMatchObject({
      type: "intent",
      intent: { rawMessage: "> quoted reply" },
    });
    if (decision.type === "intent") confirmSend(decision.key, false);
  });

  it("allows an immediate retry after failed send confirmation", () => {
    const event = {
      sourceApp: "slack" as const,
      sourceCtx: "T1:C1",
      composerId: "composer-retry",
      rawMessage: "I will follow up",
      via: "click" as const,
      composing: false,
    };

    const first = shouldCapture(event);
    expect(first.type).toBe("intent");
    if (first.type !== "intent") return;

    expect(shouldCapture({ ...event, via: "keyboard" })).toEqual({
      type: "ignore",
      reason: "duplicate",
    });
    expect(confirmSend(first.key, false)).toEqual({
      type: "ignore",
      reason: "failed_send",
    });

    const retry = shouldCapture({ ...event, via: "keyboard" });
    expect(retry.type).toBe("intent");
    if (retry.type === "intent") {
      expect(retry.key).not.toBe(first.key);
      confirmSend(retry.key, false);
    }
  });

  it("identifies slack channel, dm, thread, and workspace urls", () => {
    expect(siteFromUrl("https://app.slack.com/client/T123/C456")).toBe("slack");
    expect(siteFromUrl("https://mail.google.com/mail/u/0/#inbox")).toBe(
      "gmail",
    );
  });

  it("selector fallback reports missed probes without reading bodies", () => {
    const root = document.implementation.createHTMLDocument();
    root.body.innerHTML = `<div data-qa="message_input"></div>`;
    const probe = probeSelectors(root, {
      composeBody: ["div[data-qa='message_input']", "div[role='textbox']"],
      sendButton: ["button[data-qa='texty_send_button']"],
    });
    expect(probe.ok).toBe(false);
    expect(probe.missed).toEqual(["sendButton"]);
  });
});

describe("compose-scoped resolvers", () => {
  it("resolves the targeted Gmail composer and strips only quoted DOM", () => {
    document.body.innerHTML = `
      <section data-compose id="first-compose">
        <input data-to value="first@example.com">
        <div data-body>first draft</div>
        <button data-send>Send</button>
      </section>
      <section data-compose id="second-compose">
        <input data-to value="second@example.com">
        <div data-body id="second-body">&gt; second draft<blockquote>old quoted content</blockquote></div>
        <button data-send id="second-send">Send</button>
      </section>`;
    const selectors = {
      composeContainer: ["[data-compose]"],
      composeBody: ["[data-body]"],
      toField: ["[data-to]"],
      successProbe: ["[data-success]"],
    };
    const secondContainer = document.querySelector("#second-compose");
    const secondBody = document.querySelector("#second-body");
    const secondSend = document.querySelector("#second-send");
    expect(secondContainer).not.toBeNull();
    expect(secondBody).not.toBeNull();
    expect(secondSend).not.toBeNull();
    if (!secondContainer || !secondBody || !secondSend) return;

    for (const target of [secondSend, secondBody]) {
      const attempt = resolveGmailAttempt(target, selectors);
      expect(attempt?.container).toBe(secondContainer);
      expect(attempt?.body).toBe(secondBody);
      expect(attempt?.intent.recipient).toBe("second@example.com");
      expect(attempt?.intent.rawMessage).toBe("> second draft");
      expect(attempt?.intent.rawMessage).not.toContain("first draft");
      expect(attempt?.intent.rawMessage).not.toContain("old quoted content");
    }
  });

  it("resolves channel and thread Slack editors without crossing scopes", () => {
    window.history.replaceState({}, "", "/client/T123/C456");
    document.body.innerHTML = `
      <div data-channel>general</div>
      <section id="channel-editor">
        <div id="channel-body" data-editor>channel draft</div>
        <button id="channel-send" data-send>Send</button>
      </section>
      <div id="thread-pane" data-thread>
        <section id="thread-editor">
          <div id="thread-body" data-editor>&gt; thread draft<blockquote>old quote</blockquote></div>
          <button id="thread-send" data-send>Send</button>
        </section>
      </div>`;
    const selectors = {
      composeBody: ["[data-editor]"],
      sendButton: ["[data-send]"],
      channelContext: ["[data-channel]"],
      threadContext: ["[data-thread]"],
    };
    const channelBody = document.querySelector("#channel-body");
    const channelSend = document.querySelector("#channel-send");
    const threadBody = document.querySelector("#thread-body");
    const threadSend = document.querySelector("#thread-send");
    expect(channelBody).not.toBeNull();
    expect(channelSend).not.toBeNull();
    expect(threadBody).not.toBeNull();
    expect(threadSend).not.toBeNull();
    if (!channelBody || !channelSend || !threadBody || !threadSend) return;

    for (const target of [channelSend, channelBody]) {
      const attempt = resolveSlackAttempt(target, selectors);
      expect(attempt?.body).toBe(channelBody);
      expect(attempt?.intent.rawMessage).toBe("channel draft");
      expect(attempt?.intent.recipient).toBe("general");
      expect(attempt?.intent.sourceCtx).toBe("T123:C456");
    }

    for (const target of [threadSend, threadBody]) {
      const attempt = resolveSlackAttempt(target, selectors);
      expect(attempt?.body).toBe(threadBody);
      expect(attempt?.intent.rawMessage).toBe("> thread draft");
      expect(attempt?.intent.sourceCtx).toBe("T123:C456:thread:thread-pane");
    }
  });
});
