# Privacy

Callback's runtime:

- Opens no TCP or UDP listener
- Sends no captured message content over TCP/UDP or to a remote service
- Moves confirmed content locally through Chrome native messaging and a current-user, owner-only named pipe
- Stores desktop data in a local SQLite file and the bounded pending extension outbox in `chrome.storage.local`
- Never uses `chrome.storage.sync`
- Does not fetch selector packs at runtime
- Has no telemetry dependency
- Uses fixed generic copy for extracted/actionable Windows toasts; Phase 0 shows only the reminder text the user configured
- Logs capture IDs, kinds, scores, and content-free health data—never raw message bodies

Chrome/Gmail/Slack traffic and package-manager or Store downloads are outside this Callback-runtime guarantee. Callback does not add a separate remote content path.

The browser extension captures only after a Gmail or Slack web send is confirmed. The service worker keeps retryable items in its local outbox, Chrome starts `callback-native-host.exe`, and the host validates the pinned extension origin before forwarding a bounded frame over the named pipe. Extracted reminders use generic Windows toast copy, so captured clauses do not enter notification history; user-authored Phase 0 reminder text can appear in Action Center or on the lock screen according to Windows settings. Windows Focus Assist / DND remains controlled by the OS; Callback never bypasses it with an always-on-top card.

Purge from Health launches a helper and closes the app so SQLite is no longer open. After the desktop process exits, the helper clears Callback notifications from Windows history, deletes the desktop database and journal files, removes the native-host manifest and current-user registry key, and disables Callback autostart. `callback-app.exe --purge` performs the same desktop cleanup. Pending captures in the extension's browser-managed outbox are separate; removing the extension clears that queue.

Retention is enforced at startup, immediately after its setting changes, and once per day while the tray process remains resident. Expired resolved and review records are deleted. Open and snoozed promises remain actionable, but duplicated original-message context is redacted after the retention period. Retry receipts contain a SHA-256 digest derived from the original local payload plus content-free metadata; an expired receipt is removed on the same horizon once no retained capture depends on it. Exact retries remain idempotent while a receipt or retained capture exists. After an orphan receipt expires, a very late browser retry can be treated as a new capture.

Autostart, when enabled, writes only a quoted current-user Run command. The global shortcut only opens the local `?window=quick` window; Callback does not scrape selected text from other applications.

CI runs a static audit over first-party source, built extension JavaScript, storage APIs, raw-message log calls, and known telemetry dependencies. That audit is not a substitute for the required installed-process Windows TCP/UDP endpoint check. CI artifacts are unsigned; `SHA256SUMS.txt` checks integrity but is not an Authenticode signature, and SmartScreen warnings are expected until signing exists.
