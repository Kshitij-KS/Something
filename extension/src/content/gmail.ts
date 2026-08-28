import type { SelectorPack } from "../capture";
import { firstMatch, shouldCapture } from "../capture";

export function snapshotGmail(
  root: ParentNode,
  selectors: SelectorPack["gmail"],
) {
  const body = firstMatch(root, selectors.composeBody ?? []);
  const to = firstMatch(root, selectors.toField ?? []);
  return {
    sourceApp: "gmail" as const,
    sourceCtx: document.location.pathname,
    recipient: to?.textContent?.trim() || undefined,
    rawMessage: body?.textContent?.trim() ?? "",
  };
}

export function gmailSendFromKeyboard(event: KeyboardEvent): boolean {
  return (
    event.key === "Enter" &&
    (event.ctrlKey || event.metaKey) &&
    !event.isComposing
  );
}

export { shouldCapture };
