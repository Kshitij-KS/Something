import type { SelectorPack } from "../capture";
import { firstMatch } from "../capture";

export function snapshotSlack(
  root: ParentNode,
  selectors: SelectorPack["slack"],
) {
  const body = firstMatch(root, selectors.composeBody ?? []);
  const channel = firstMatch(root, selectors.channelContext ?? []);
  const thread = firstMatch(root, selectors.threadContext ?? []);
  const context = thread?.id || channel?.textContent?.trim();
  return {
    sourceApp: "slack" as const,
    sourceCtx: context,
    recipient: channel?.textContent?.trim() || undefined,
    rawMessage: body?.textContent?.trim() ?? "",
  };
}

export function slackSendFromKeyboard(event: KeyboardEvent): boolean {
  return event.key === "Enter" && !event.shiftKey && !event.isComposing;
}
