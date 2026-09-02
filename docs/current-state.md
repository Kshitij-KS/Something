---
meta:
  title: Understand Callback's current implementation
  navLabel: Current State
  category: Project
  contentType: Reference
  plan: ./documentation-plan.md
---

# Understand Callback's current implementation

This reference describes what the repository implements now, which behavior remains gated, where each subsystem lives, and which limitations still block a public release. It separates code presence from automated validation, human evidence, and installed-build qualification.

## Interpret the status correctly

Version `0.1.0` is a Windows engineering preview with unsigned candidate artifacts. The desktop, extension, native host, persistence, extraction, surfacing, and release infrastructure exist, but the repository does not prove that any human gate passed.

| Dimension                      | Repository state                                                                                                 |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------- |
| Product version                | `0.1.0` across npm, Tauri, extension, and Cargo packages                                                         |
| Engineering scope              | Phase 0, capture, extraction, closed loop, health, and packaging code present                                    |
| Repository-default evidence    | All three kill gates seeded as `pending_user`                                                                    |
| Runtime target                 | Windows with the default `windows-platform` Cargo feature                                                        |
| Browser target                 | Google Chrome Manifest V3                                                                                        |
| Site target                    | Gmail web and Slack web                                                                                          |
| Candidate artifacts            | Unsigned Nullsoft Scriptable Install System (NSIS), Windows Installer (MSI), extension ZIP, and `SHA256SUMS.txt` |
| Public-release claim           | None                                                                                                             |
| Installed-device certification | Pending human physical or virtual machine checks                                                                 |

The [`0.1.0` evidence snapshot](release-evidence/0.1.0.md) preserves an unattributed historical report of 116 passing runnable Rust tests and 14 passing extension tests. It has no retained timestamp, transcript, workflow identity, or clean source revision, so it is not candidate qualification evidence. One private-corpus test remains ignored until a private local corpus is supplied.

## Understand the gate-controlled rollout

Engineering has proceeded behind persistent evidence gates. Runtime behavior still respects the rollout boundary:

1. Phase 0 focus rules can surface before any gate passes
2. Extracted context reminders stay silent until `extraction_precision_300` passes
3. Deadline escalation also stays silent until `extraction_precision_300` passes
4. The database prevents passing extraction before `phase0_five_day`
5. The database prevents passing acceptance before extraction
6. `acceptance_two_week` governs release readiness rather than unlocking another runtime path

A developer can compile downstream code without claiming that its trial passed. Read [`kill-gates.md`](kill-gates.md) for evidence rules.

## Use the desktop application

The React application has two presentation modes and four main sections.

Closing the `main` window hides it instead of terminating the process, so background workers remain resident. Click the tray icon or choose **Open** to restore it; choose **Quit** to exit. Closing the auxiliary `quick` window closes that window normally.

### Complete onboarding

`src/components/Onboarding.tsx` explains extension installation, local storage, autostart defaults, the 30-minute silence window, and unsigned-build warnings. Completing onboarding persists a future timestamp that suppresses extracted notifications for 30 minutes.

The Quick capture window bypasses onboarding because it must remain available from the global shortcut.

### Capture a promise manually

`src/App.tsx` renders a dedicated `?window=quick` mode. It accepts typed text, never reads the clipboard or another application's selection, and invokes `quick_capture` with a retry-stable `manual-uuid_value` identifier.

The backend creates an Open promise with score `10`, confidence `1.0`, an optional parsed deadline, and local keyword triggers. A manual promise without a keyword mapping or deadline remains visible in the inbox but cannot match application focus.

### Work from the Promise Inbox

`src/components/PromiseInbox.tsx`, `src/components/PromiseDetail.tsx`, and `src/promises.ts` implement the main promise workflow:

- Open, Snoozed, Review, and Resolved tabs
- Deterministic list ordering for each status
- Content-minimized summaries and detail projections
- Editable promise text and optional timezone-aware deadline
- Daylight-saving gap and repeated-time rejection in the editor
- Promotion from Review to Open with trigger creation
- Done, one-hour snooze, resume, Not a promise, and skip actions
- Automatic archival on the third committed skip
- Local blocklist learning after Not a promise
- Dirty-draft protection during navigation, refresh, and notification routing
- Stale-write protection through expected status and ignore count
- Loading, empty, error, retry, and missing-record states
- Generation and ownership guards against late asynchronous responses

Resolved records are read-only. The UI does not yet provide a full activity timeline or a safe reopen flow.

### Route clicked notifications

Actionable Windows notifications use `callback-action://` activation URIs. `src-tauri/src/main.rs`, `src-tauri/src/lifecycle.rs`, and `src-tauri/src/commands.rs` parse activation arguments, apply valid actions, restore the main window, and enqueue a content-minimized route.

The React shell listens for `promise-route-ready`, peeks the first-in-first-out route, opens the correct status tab, and acknowledges it only after handling. Duplicate activation keys and recently acknowledged route IDs are bounded to 128 entries in memory.

The route queue is not durable across a crash. Installed cold-start and warm-process activation remain manual qualification items.

### Manage focus rules

The Focus rules screen implements the complete current Phase 0 rule lifecycle:

- List persisted rules
- Discover visible Windows applications
- Retain manual executable entry when discovery fails
- Add an executable and reminder pair
- Preserve exact paused duplicates without silently resuming them
- Display Active or Paused state
- Pause or resume one rule with backend-authoritative state
- Retry a failed full-list load
- Record human kill-gate evidence in prerequisite order

The UI prevents full-list reads, add or retry requests, and same-row toggles from racing. Separate rows can report independent feedback. There is no rule edit, delete, bulk action, reordering, or import flow.

`src-tauri/src/platform/focus/sys_windows.rs` enumerates executable basenames only. It does not expose titles, full paths, process identifiers, or window handles to the webview. Enumeration revalidates window ownership and process identity to resist handle and process identifier reuse.

### Configure runtime settings

`src/components/Settings.tsx` loads and saves:

- Daily surface cap from 1 to 3
- Minimum surface gap of at least 90 minutes
- Data retention from 1 to 3,650 days
- Internet Assigned Numbers Authority (IANA) timezone
- Quiet-hours enablement, start, and end
- Gmail and Slack capture enablement
- Current-user autostart
- Primary and fallback global shortcuts

The backend validates all persisted values. Retention changes trigger an immediate retention pass. A site-disable setting becomes authoritative in the core immediately and clears matching live browser context. The extension receives that policy and removes pending source items after its next successful policy-bearing native acknowledgement; `save_setting` does not wait for browser cleanup.

### Inspect health and remove local data

`src/components/HealthStatus.tsx` shows:

- Whether this process has observed a native handshake
- Whether the sidecar executable exists
- Gmail and Slack selector state
- Probe, failure, and last-capture timestamps
- Remaining onboarding silence
- Shortcut registration outcome
- The adapter's no-network-listener declaration

Health can reopen Quick capture, rewrite native-host registration, refresh diagnostics, or schedule a purge helper. It does not poll continuously.

The connection label means that a handshake occurred during the current process lifetime. It does not age out or detect a later disconnect.

## Capture confirmed Gmail and Slack sends

The Chrome extension has no popup or options page. The desktop controls the two-site policy.

### Restrict extension privileges

`extension/manifest.json` declares Manifest V3, `nativeMessaging`, and `storage`. Host permissions and content-script matches cover only:

- `https://mail.google.com/*`
- `https://app.slack.com/*`

The extension does not request tabs, scripting, wildcard hosts, remote code, or synchronized storage. The checked-in public key stabilizes the development extension identifier used by native-host origin allowlists.

### Resolve compose-scoped content

`extension/src/content/gmail.ts` and `extension/src/content/slack.ts` locate the relevant composer instead of reading an entire page.

Gmail handling:

- Supports send-button clicks and Control or Command plus Enter
- Resolves the nearest compose container and body
- Reads the local recipient field and path context
- Removes quoted and forwarded blocks from a clone
- Requires both compose completion and a success-probe mutation

Slack handling:

- Supports send-button clicks and Enter without Shift
- Excludes Input Method Editor composition
- Preserves the active editor across pointer and click handoff
- Resolves channel, direct-message, thread, and team route context
- Rejects ambiguous composer matches
- Requires the scoped body to remain empty for 250 ms
- Can adopt one unambiguous replacement editor after Slack rerenders

Both paths time out after 10 seconds. Failed confirmation sends no message body to the service worker and clears the local dedupe window for retry.

### Keep a durable local outbox

`extension/src/background.ts` and `extension/src/outbox.ts` own policy, native messaging, and retry behavior:

- Default both sites to disabled, hydrate the last stored policy before connecting, and refresh policy from native acknowledgements
- Keep a fresh browser profile disabled until its first policy-bearing acknowledgement
- Validate sender hostnames against the claimed source
- Accept capture only after content-script confirmation
- Create stable `cap-intent_key` identifiers
- Serialize storage mutations
- Keep at most 500 records and 5 MiB
- Drop the oldest items when limits are exceeded
- Flush in order after a successful handshake or new enqueue
- Remove an item only after committed or terminal-discard acknowledgement
- Remove source-specific pending items after a policy-bearing acknowledgement disables that site
- Retry native connection from 500 ms up to 10 seconds

Manifest V3 timers are best-effort while the service worker is alive. Durable recovery depends on the stored outbox and the next worker wake because no `chrome.alarms` wake strategy exists.

### Report content-free health and context

The content script sends:

- A visible and active route heartbeat every two seconds
- Context updates after focus, visibility, hash, and history changes
- Selector probes every 30 seconds when a composer exists
- Sanitized Slack capture-stage diagnostics

The core retains live browser context in memory for 15 seconds. It combines web context only when Chrome is the foreground executable. Edge and other Chromium browsers are not recognized by this path.

Selector packs come from `extension/selectors.json` at build time. The extension does not fetch runtime selector updates.

## Move captures through local native messaging

The production capture chain is:

1. The service worker calls `chrome.runtime.connectNative('com.callback.host')`
2. Chrome starts `callback-native-host.exe` with a pinned extension origin
3. The host uses binary standard input and output with native-messaging frames
4. The host connects to `\\.\pipe\callback-com.callback.desktop`
5. The Tauri process validates and commits the envelope
6. The acknowledgement returns only after the database transaction commits
7. The extension removes the outbox record after committed or discard acknowledgement

`crates/protocol` defines protocol version `1`, envelope kinds, JSON encoding, and frame limits. Chrome-to-host frames allow 64 MiB. Host-to-Chrome frames allow 1 MiB. Capture validation applies an additional 5 MiB body limit.

`crates/native-host` validates the pinned origin, logs kind, identifier, and direction for accepted envelopes, and makes up to 20 core connection attempts with 250 ms between failures. Fatal protocol or transport failures log their error string to standard error and can include a rejected origin. Standard output remains framed-only.

`src-tauri/src/ipc/named_pipe.rs` creates owner-only Windows pipe instances. This prevents cross-user access but does not authenticate another process running as the same Windows user.

## Extract and persist promises

`src-tauri/src/extraction`, `src-tauri/src/review`, and `src-tauri/src/db` implement deterministic local processing.

### Score clauses deterministically

The extractor:

- Segments a message into candidate clauses
- Normalizes contractions while retaining original display text
- Adds scores for commissive language, deliverable verbs, temporal anchors, and concrete objects
- Rejects questions, requests to another person, opinions, completed actions, quoted text, and learned blocklist shapes
- Penalizes conditional, attendance-only, and long clauses
- Routes score 6 or higher to Open
- Routes scores 4 and 5 to Review
- Discards lower scores

It uses no language model and makes no network request. Logs contain clause ordinal, score, and reason without body text.

### Parse a bounded deadline lexicon

`src-tauri/src/extraction/deadline.rs` resolves explicit local forms such as today, end of day, tonight, tomorrow, `by Friday`, end of week, this week, next week, `by the Nth`, and `in an hour`. It stores UTC instants with the parsing timezone and precision.

Ambiguous or nonexistent local wall-clock times fail closed. A promise remains valid without a deadline.

### Commit a retry-safe write set

`Database::commit_prepared_capture` stores one capture receipt, all retained clauses, promises, deadlines, triggers, and selector health in one transaction. A capture identifier plus lowercase SHA-256 payload fingerprint defines idempotency.

An exact retry returns the prior result. Reusing an identifier for changed canonical content returns a conflict. A transaction failure rolls back every related write.

Disabled sites receive a terminal discard acknowledgement. This prevents private content from remaining indefinitely in the extension outbox after a policy change.

## Match focus and surface reminders

The Windows runtime combines operating-system focus with fresh extension context.

### Detect a stable focus target

`src-tauri/src/platform/focus` uses a Windows event hook and a hidden message window for:

- Foreground changes
- Session lock and unlock
- Suspend and resume

Callbacks use a bounded channel and never block the Windows callback. The five-second `FocusDebouncer` uses monotonic time and generation invalidation. Lock, unlock, sleep, resume, protected-process lookup failure, and target changes cancel stale dwell.

### Match Phase 0 rules

`src-tauri/src/surfacing/phase0.rs` matches enabled executable basenames without case sensitivity. A successful dwell shows the exact reminder text configured by the operator.

Phase 0 does not use extracted-reminder daily caps, spacing, quiet hours, or action tokens. Windows Focus Assist still controls operating-system delivery. Pausing a rule is effective for dwell evaluations that begin after persistence succeeds; it cannot retract a toast already submitted to Windows.

### Match extracted promises

`src-tauri/src/triggers` creates and evaluates:

- Exact source-context triggers at priority 100
- Source-application fallbacks at priority 10
- Keyword-to-executable fallbacks at priority 5
- Manual markers at priority 0 that never match focus

The selector collapses multiple matching triggers for one promise. It then orders candidates by earliest deadline, highest confidence, oldest creation, and trigger priority.

### Apply surfacing policy

`src-tauri/src/surfacing/engine.rs` and `rate_limit.rs` enforce:

- One active extracted notification globally
- One surface for the same promise per local day
- A configurable daily cap from 1 to 3, default 3
- A configurable minimum gap of at least 90 minutes
- Configured quiet hours
- A 30-minute onboarding silence
- Suppression after a clock rollback greater than five minutes
- One deadline escalation for an otherwise unseen promise
- A fresh dwell after a snooze expires

A policy-suppressed extracted match does not fall through to Phase 0 during the same dwell.

### Deliver and redeem actions durably

Each actionable surface starts with a database lease and unique action token. The notification sink receives fixed generic copy, not captured text. A successful operating-system submission records delivery atomically. A failure records a bounded error and releases the lease.

Done, Snooze, Not a promise, and Ignore redeem a token exactly once. Tokens expire after 15 minutes. Startup recovers expired leased or shown attempts.

## Store and remove local data

The database uses bundled SQLite with write-ahead logging, foreign keys, a five-second busy timeout, and `synchronous=NORMAL`. It runs an integrity check before applying numbered migrations.

### Track the current schema

Schema version `4` includes these main surfaces:

| Table                   | Purpose                                                      |
| ----------------------- | ------------------------------------------------------------ |
| `captures`              | One retained source clause and its local context             |
| `capture_receipts`      | Content-free retry identity and committed-clause count       |
| `promises`              | Extracted text, state, deadline, snooze, and resolution data |
| `triggers`              | Context, application, keyword, and manual links              |
| `surface_attempts`      | Durable leases, tokens, shown state, and actions             |
| `notification_attempts` | Operating-system delivery outcome per surface                |
| `phase0_rules`          | Executable, reminder text, and persisted enabled state       |
| `selector_health`       | Content-free Gmail and Slack diagnostics                     |
| `blocklist`             | Learned false-positive skeletons                             |
| `settings`              | Validated local configuration                                |
| `kill_gates`            | Ordered local human evidence status                          |
| `connection_state`      | Persisted scaffold not used by current runtime health        |

SQLite is not encrypted at rest. Raw message content and source context remain local but can exist in the database until retention or purge removes them.

### Enforce retention

Retention runs at startup, after its setting changes, and every 24 hours while the app remains resident. It:

- Redacts old raw source bodies for unresolved Open and Snoozed promises
- Deletes old captures whose promises are Review or terminal
- Removes orphan retry receipts after their retention horizon

An extension retry that arrives after both the receipt and retained capture expire can become a new capture.

### Purge desktop state

Purge starts a helper, exits the desktop process, and then removes:

- SQLite database, write-ahead log, shared-memory, and journal files
- Callback notification history
- Native-host manifest and Chrome current-user registry key
- Current-user autostart registration

The extension outbox belongs to Chrome and is not removed by desktop purge. Removing the extension clears that browser-managed queue.

## Map the codebase

Use this map to find each implementation surface:

| Path                               | Responsibility                                                                                   |
| ---------------------------------- | ------------------------------------------------------------------------------------------------ |
| `src/main.tsx`                     | React entry point and Strict Mode root                                                           |
| `src/App.tsx`                      | App mode, onboarding gate, navigation, routes, Quick capture, focus rules, and evidence controls |
| `src/promises.ts`                  | Promise types, tab mappings, labels, date conversion, and formatting                             |
| `src/components/PromiseInbox.tsx`  | List, detail ownership, routing, mutation serialization, and dirty-state protection              |
| `src/components/PromiseDetail.tsx` | Promise editor, metadata, deadlines, and lifecycle actions                                       |
| `src/components/Settings.tsx`      | Settings form and persistence                                                                    |
| `src/components/HealthStatus.tsx`  | Diagnostics, reconnect, Quick capture fallback, and purge                                        |
| `src/components/Onboarding.tsx`    | First-run disclosure and extension setup                                                         |
| `src/components/ReviewQueue.tsx`   | Legacy standalone review UI; not mounted                                                         |
| `src/styles.css`                   | Desktop, inbox, rule, responsive, and accessibility styling                                      |
| `extension/src/content.ts`         | Policy-aware gesture handling, send confirmation, context, and probes                            |
| `extension/src/content/gmail.ts`   | Gmail compose resolution and authored-text extraction                                            |
| `extension/src/content/slack.ts`   | Slack composer, route, channel, direct-message, and thread resolution                            |
| `extension/src/capture.ts`         | Intent validation, canonicalization, dedupe, and confirmation state                              |
| `extension/src/background.ts`      | Native port, source policy, sender checks, context, probes, and flush control                    |
| `extension/src/outbox.ts`          | Serialized bounded local retry queue                                                             |
| `extension/selectors.json`         | Versioned build-time selector fallback chains                                                    |
| `crates/protocol`                  | Shared versioned envelopes and framing                                                           |
| `crates/native-host`               | Chrome origin check and standard-input/output bridge                                             |
| `src-tauri/src/main.rs`            | Purge mode, activation parsing, and runtime entry                                                |
| `src-tauri/src/lib.rs`             | Tauri composition, startup, workers, pipe callback, and command registration                     |
| `src-tauri/src/commands.rs`        | React-facing command contract and managed state                                                  |
| `src-tauri/src/lifecycle.rs`       | Single instance, tray, activation redemption, route queue, and window behavior                   |
| `src-tauri/src/db/mod.rs`          | Migrations, transactions, projections, settings, retention, gates, and durable actions           |
| `src-tauri/src/domain/mod.rs`      | Promise and supporting state transitions                                                         |
| `src-tauri/src/extraction`         | Clause scoring and deadline parsing                                                              |
| `src-tauri/src/review`             | Capture ingestion, review actions, fingerprints, and trigger preparation                         |
| `src-tauri/src/triggers`           | Trigger creation and matching                                                                    |
| `src-tauri/src/surfacing`          | Dwell, Phase 0, selection, rate limits, leases, and callback actions                             |
| `src-tauri/src/platform`           | Windows adapters and non-Windows no-op implementations                                           |
| `src-tauri/src/ipc`                | Envelope validation, acknowledgement, and named-pipe server                                      |
| `src-tauri/src/native_host`        | Manifest registration and current-user autostart                                                 |
| `src-tauri/src/health`             | Content-free selector and connection diagnostics                                                 |
| `src-tauri/src/shortcut.rs`        | Primary and fallback global shortcut policy                                                      |
| `src-tauri/src/purge.rs`           | Post-exit local cleanup helper                                                                   |
| `src-tauri/migrations`             | Authoritative schema versions 1 through 4                                                        |
| `scripts`                          | Staging, extension validation, privacy audit, metadata checks, and packaging                     |
| `.github/workflows`                | Windows continuous integration and draft release candidates                                      |

## Use the Tauri command surface

`src-tauri/src/lib.rs` registers 23 commands:

| Surface                | Commands                                                                   |
| ---------------------- | -------------------------------------------------------------------------- |
| Notification routes    | `peek_pending_promise_route`, `ack_pending_promise_route`                  |
| Promise Inbox          | `list_promises`, `get_promise`, `update_promise`, `act_on_promise`         |
| Legacy review          | `list_review`, `review_promise`                                            |
| Focus rules            | `list_focus_apps`, `list_phase0`, `set_phase0_rule_enabled`, `add_phase0`  |
| Capture and settings   | `quick_capture`, `save_setting`, `load_setting`                            |
| Diagnostics            | `health`, `health_banner`, `reconnect_extension`, `open_quick_capture`     |
| Lifecycle and evidence | `complete_onboarding`, `list_kill_gates`, `record_kill_gate`, `purge_data` |

Route access, visible-app enumeration, and focus-rule toggling verify the `main` window label. Several other custom commands rely on the trusted local webview instead of an explicit main-window check. Broad command authorization remains a hardening item.

## Understand validation coverage

The Rust workspace has 14 `callback-app` integration suites plus native-host framing tests. They cover:

- Schema migration, corruption, disk-full mapping, retention, and lease recovery
- Atomic capture, conflict detection, exact retry, and disabled-site discard
- Domain transitions, review promotion, blocklist learning, and settings
- Focus debounce, target matching, candidate ordering, and policy suppression
- Durable delivery, duplicate and late actions, snooze, and deadline behavior
- Native framing, origin rejection, partial input, oversized input, and one framed-only standard-output handshake smoke case
- Platform adapter selection, shortcut policy, and composition wiring

The static privacy audit also checks known raw-message logging patterns.

Vitest runs `extension/tests/capture.test.ts` and `extension/tests/outbox.test.ts`. Those tests cover compose isolation, quoted-content removal, Input Method Editor rejection, dedupe, retries, serialized storage, source removal, stable identifiers, and overflow eviction.

Automated checks do not certify:

- Live Gmail or Slack Document Object Model compatibility
- Browser service-worker suspension and restart timing
- React component behavior in a browser test harness
- Real Windows event hooks, pipe access-control lists, or notification delivery
- Warm and cold protocol activation
- Action Center cleanup, Focus Assist, lock, sleep, autostart, or uninstall
- Installed-process Transmission Control Protocol (TCP) and User Datagram Protocol (UDP) endpoints
- Authenticode identity or SmartScreen reputation

Read [`development.md`](development.md) for commands and [`release.md`](release.md) for manual qualification.

## Track current limitations and technical debt

The following constraints are part of the present state:

- Every repository-default human gate remains pending
- Phase 0 rules cannot be edited or deleted
- Extracted and deadline notifications stay gated until extraction evidence passes
- Runtime health does not age or clear a prior handshake
- `connection_state` and its reducer are scaffolding rather than live health state
- Action delivery currently depends on process arguments and single-instance forwarding; installed protocol behavior remains a manual test
- The in-memory notification route queue does not survive a crash
- The legacy standalone `ReviewQueue` is orphaned
- Legacy `review_promise` lacks the stronger stale-snapshot protection used by Promise Detail actions
- Not every custom Tauri command checks the calling window label
- SQLite has no at-rest encryption
- Another process running as the same Windows user can address the named pipe
- Only `chrome.exe` participates in web-context matching
- Manifest V3 reconnect timers have no explicit alarm-backed wake
- Selectors update only through a rebuilt extension
- Quiet-hour clock evaluation can use the machine-local zone while daily accounting uses the configured zone
- Phase 0 bypasses extracted-reminder cap, gap, and quiet-hour policy
- Desktop purge does not clear Chrome's extension outbox
- macOS and Linux adapters compile but do not provide functional focus, pipe, or notification behavior
- Release artifacts are unsigned and unqualified on installed devices

Future work and its proposed version boundaries live in [`roadmap.md`](roadmap.md).
