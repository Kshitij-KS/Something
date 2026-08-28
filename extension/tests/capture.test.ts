import { describe, expect, it } from "vitest";
import {
  confirmSend,
  probeSelectors,
  shouldCapture,
  siteFromUrl,
} from "../src/capture";

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

  it("ignores IME, quoted, failed, and duplicate handlers", () => {
    expect(
      shouldCapture({
        sourceApp: "slack",
        rawMessage: "I will send it",
        via: "keyboard",
        composing: true,
      }).type,
    ).toBe("ignore");
    expect(
      shouldCapture({
        sourceApp: "gmail",
        rawMessage: "> quoted reply",
        via: "click",
        composing: false,
      }).type,
    ).toBe("ignore");
    const first = shouldCapture({
      sourceApp: "slack",
      sourceCtx: "C123",
      rawMessage: "I will follow up",
      via: "click",
      composing: false,
    });
    const second = shouldCapture({
      sourceApp: "slack",
      sourceCtx: "C123",
      rawMessage: "I will follow up",
      via: "keyboard",
      composing: false,
    });
    expect(second.type).toBe("ignore");
    if (first.type === "intent") {
      expect(confirmSend(first.key, false)).toEqual({
        type: "ignore",
        reason: "failed_send",
      });
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
