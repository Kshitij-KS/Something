---
meta:
  title: Understand Callback's privacy boundary
  navLabel: Privacy
  category: Project
  contentType: Reference
  plan: ./documentation-plan.md
---

# Understand Callback's privacy boundary

Callback's installed-runtime privacy contract keeps captured content on the local Windows account. Source and static checks identify no Callback-owned Transmission Control Protocol (TCP) or User Datagram Protocol (UDP) path for captured content. Installed-process endpoint inspection remains pending for `0.1.0`, so the evidence record does not yet certify this contract on a packaged build.

## Scope the guarantee to the installed runtime

The guarantee covers Callback's installed desktop process, native host, Chrome extension, local inter-process communication, and local persistence.

It does not cover:

- Gmail, Slack, Chrome, extension Store, package-manager, or operating-system traffic
- Source downloads, dependency installation, continuous integration, or release uploads
- The Vite development server on `127.0.0.1:1420`
- A browser or operating system compromised outside Callback's trust boundary

Callback does not add a second remote content path to the normal site traffic.

## Keep capture transport local

Confirmed content follows this path:

1. A Gmail or Slack content script confirms a completed send
2. The extension service worker writes a bounded record to `chrome.storage.local`
3. Chrome starts the origin-pinned native-messaging host
4. The host forwards a bounded frame over an owner-only named pipe
5. The Tauri core validates and commits the capture to local SQLite
6. The acknowledgement removes the browser outbox record

Callback never uses `chrome.storage.sync`. It does not fetch selector packs at runtime and declares no telemetry dependency.

The extension grants host access only to Gmail web and Slack web. The service worker validates the sender tab hostname before accepting capture, context, or probe data.

## Understand local trust limits

The native host validates the pinned Chrome extension origin. The Windows pipe uses a protected owner-only access-control list.

These controls isolate Windows accounts. They do not authenticate another process running under the same Windows account. A malicious same-user process could attempt to connect directly to the pipe.

The Tauri content security policy limits content and network sources to self and local Tauri inter-process communication. Some commands check the `main` webview label. Other custom commands trust any Callback webview with command access.

## Know what the desktop stores

Callback uses bundled SQLite without at-rest encryption. Operating-system account and disk protections provide local confidentiality.

The desktop can store:

| Data                     | Purpose                                              |
| ------------------------ | ---------------------------------------------------- |
| Original sent message    | Local extraction context, review, and retry evidence |
| Extracted clause         | Promise display and local surfacing                  |
| Source and route context | Context-specific trigger matching                    |
| Recipient display value  | Promise context in the local inbox                   |
| Deadlines and timezone   | Local scheduling and display                         |
| Capture fingerprint      | Exact-retry conflict detection                       |
| Surface and action state | Rate limits, recovery, and exactly-once actions      |
| Selector health          | Content-free capture diagnostics                     |
| Settings and gate notes  | Local configuration and human evidence               |

The Promise Inbox command projections omit the full original message. They expose the extracted promise text and required metadata to the trusted local React webview.

Visible-app discovery exposes executable basenames only. It does not return full paths, process identifiers, window handles, or window titles to React.

## Bound extension storage

The service worker stores pending capture records in `chrome.storage.local`. The outbox keeps at most:

- 500 records
- 5 MiB of encoded message-body bytes

Oldest records are evicted when needed. A site-disable setting takes effect immediately in the core. The extension removes pending records for that source after its next successful policy-bearing native acknowledgement. Browser cleanup can therefore lag while native messaging is unavailable.

Desktop purge cannot remove browser-managed extension storage. Remove the extension to clear its outbox and other local extension keys.

## Describe current logging instrumentation accurately

Accepted native envelopes log kind, identifier, and direction. Extraction instrumentation records clause ordinal, score, and classification reason. Selector diagnostics retain content-free status, counts, and timestamps. Native-host fatal errors log the protocol or transport error string to standard error and can include a rejected extension origin.

One dormant Rust tracing site includes the native-host executable path when registration fails. The current binary configures no tracing subscriber, so that event is not emitted by default. This path field remains a privacy-hardening item before tracing is enabled.

The privacy target excludes original message bodies, extracted clauses, recipients, source contexts, action tokens, window titles, and handles from logs. The static audit checks selected direct raw-message patterns, not every possible logging flow. Code review must inspect new logging and tracing sites.

## Minimize Windows notification content

Extracted actionable notifications use fixed generic copy:

- Title: `Callback`
- Body: `A reminder is ready in Callback.`

Captured clauses therefore do not enter Windows notification history through this path. Notification activation carries a random action token, not promise text.

Phase 0 focus rules are the explicit exception. They show the reminder text entered by the operator. Windows can display that text in Action Center or on the lock screen according to system settings.

Windows Focus Assist or Do Not Disturb controls operating-system presentation. Callback does not bypass it with an always-on-top card.

## Apply retention locally

Retention runs:

- At startup
- Immediately after the retention setting changes
- Every 24 hours while the tray process remains resident

After the retention horizon, Callback:

- Redacts original message bodies for unresolved Open and Snoozed promises
- Deletes eligible old captures for Review and terminal promises
- Removes orphan retry receipts when no retained capture depends on them

The extracted actionable clause remains for unresolved promises. An extension retry that arrives after both the receipt and retained capture expire can become a new capture.

## Purge desktop state after exit

**Purge local data** starts a helper and exits the desktop so SQLite is closed. The helper then attempts to remove:

- Callback notification history
- SQLite database, write-ahead log, shared-memory, and journal files
- Native-host manifest and temporary manifest
- Chrome current-user native-host registration
- Callback current-user autostart registration

Purge is sequential across operating-system resources. A reported failure can occur after earlier cleanup already succeeded.

The purge command does not clear Chrome's outbox. Extension removal clears browser-managed state.

## Keep Quick capture explicit

The global shortcut opens the local `?window=quick` webview. Callback never reads arbitrary selected text or clipboard content from another application.

Autostart, when enabled, stores only a quoted current-user Run command for the installed executable.

## Verify the boundary in two layers

Continuous integration runs `npm run audit:local-only`. The static audit checks:

- Source under `src`, `src-tauri/src`, `crates/native-host/src`, `crates/protocol/src`, and `extension/src`, plus built extension JavaScript
- Network application programming interfaces
- Synchronized extension storage
- Runtime selector fetching
- Raw-message log calls
- Known telemetry dependencies

The audit does not scan every test, script, migration, manifest, build file, or configuration. Static analysis cannot prove installed process behavior. Every qualified Windows release also requires endpoint inspection while exercising capture, focus, notifications, health, idle residency, and purge.

Read [`release.md`](release.md) for the installed matrix. Record only content-free results.

## Treat checksums and signing separately

`SHA256SUMS.txt` verifies candidate bytes. It does not establish publisher identity.

Current candidates are unsigned, so Windows SmartScreen and Mark-of-the-Web warnings are expected. Authenticode policy and key custody must be resolved before general availability under the current roadmap.
