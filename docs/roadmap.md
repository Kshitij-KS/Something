---
meta:
  title: Plan Callback's future releases
  navLabel: Roadmap
  category: Project
  contentType: Reference
  plan: ./documentation-plan.md
---

# Plan Callback's future releases

This roadmap converts Callback's phase plan into versioned, evidence-gated release targets. It assigns no dates and makes no completion claim for work that lacks human or installed-build proof.

## Read version targets as gates, not dates

Callback already contains code from several planned phases. A later feature appearing in `0.1.0` does not mean its release milestone passed.

Each planned version has four independent dimensions:

- **Engineering**: required code exists and has reviewable behavior
- **Automated validation**: named checks pass for the candidate revision
- **Human evidence**: the required private trial passes
- **Distribution**: artifacts, signing, installation, and publication meet that version's policy

A version advances only when every required dimension passes. If a kill gate fails, stop the release train and fix or abandon the premise before broadening scope.

## Use the planned release train

The current release sequence is:

| Version          | Intended audience               | Primary outcome                             | Required human gate                               |
| ---------------- | ------------------------------- | ------------------------------------------- | ------------------------------------------------- |
| `0.1.0`          | Developers and internal testers | Current Windows engineering preview         | None passed by repository default                 |
| `0.2.0`          | Phase 0 tester                  | Qualified focus-rule preview                | `phase0_five_day`                                 |
| `0.3.0`          | Private capture tester          | Qualified capture and extraction alpha      | `extraction_precision_300` at 70 percent          |
| `0.4.0`          | Closed-loop beta tester         | Qualified context and deadline beta         | `acceptance_two_week` at 40 percent               |
| `0.5.0`          | Release owner and final testers | Installed Windows release candidate         | All gates plus installed matrix                   |
| `1.0.0`          | Supported Windows audience      | General-availability Windows release        | Release approval and 80 percent extraction target |
| `1.1.0`          | Existing users                  | Reliability and diagnostics hardening       | No new product gate planned                       |
| `1.2.0`          | Existing users                  | Workflow completeness and rule management   | No new product gate planned                       |
| `2.0.0` research | Uncommitted                     | New sources, browsers, or desktop platforms | New evidence plan required                        |

These version numbers are planning targets. Change them only through a documented release decision and synchronized metadata update.

## Treat `0.1.0` as the current engineering preview

`0.1.0` contains the complete implementation described in [`current-state.md`](current-state.md). Its purpose is to expose the system for focused testing, not to claim public readiness.

### Current engineering scope

The preview includes:

- Windows focus detection and five-second dwell
- Persistent focus-rule add, list, pause, and resume
- Gmail and Slack confirmed-send capture
- Durable extension outbox and native-messaging bridge
- Deterministic extraction and deadline parsing
- Promise Inbox, review, editing, and lifecycle actions
- Context and deadline surfacing behind evidence controls
- Settings, health, Quick capture, retention, and purge
- Unsigned Nullsoft Scriptable Install System (NSIS), Windows Installer (MSI), and extension candidate packaging

### Current evidence state

The repository seeds all three gates as `pending_user`. Existing build artifacts, automated tests, and reviews do not close those gates.

### Current distribution state

Generated artifacts are unsigned. No Chrome Web Store, winget, Scoop, or public GitHub release claim exists. SmartScreen warnings are expected.

## Qualify Phase 0 in `0.2.0`

`0.2.0` should prove the core premise. Pass only when the tester states that context timing improved actionability enough to keep using Callback and supports that decision with opportunity, useful-surface, and incorrect-surface counts. The gate has no numeric usefulness threshold.

### Scope the candidate narrowly

The candidate should preserve current downstream code while testing only the Phase 0 path:

- Create a focus rule from a visible executable or manual entry
- Require five seconds of continuous eligible focus
- Show the configured reminder through the Windows notification adapter
- Persist Active or Paused state across restart
- Prevent a paused rule from firing after a fresh dwell
- Resume later eligible firing without duplicating the rule
- Cancel stale dwell across focus change, lock, sleep, unlock, and resume
- Retain manual executable entry when process enumeration fails
- Keep process titles, paths, identifiers, and handles out of webview data

Do not enable extracted or deadline notifications as part of this gate.

### Collect required evidence

Use one installed format at a time. Record:

- Five consecutive days of actual focus-rule use
- Rule usefulness compared with a normal time reminder
- False or mistimed surfaces
- Pause and resume behavior across restart
- Rapid-toggle and multi-row behavior
- Physical or virtual machine lock, sleep, and resume results
- Windows Focus Assist behavior
- Installed-process endpoint inspection

Evidence must remain content-free. Do not record reminder text, window titles, process paths, or unrelated application details.

### Exit `0.2.0`

Advance only when:

1. `phase0_five_day` is recorded as passed with trial duration, opportunity count, useful and incorrect surface counts, blocking defects, and the final continue-or-stop decision
2. The Phase 0 subset of the installed matrix passes
3. No privacy or race regression remains open
4. The premise still warrants capture and extraction investment

If the gate fails, stop. Do not reinterpret the result as a partial pass.

### Keep unrelated improvements out

Rule edit, delete, reordering, and bulk controls are useful but do not prove the premise. Schedule them only when trial feedback makes them necessary or defer them to `1.2.0`.

## Qualify capture and extraction in `0.3.0`

`0.3.0` should prove that Callback can capture sent commitments without duplicates and classify them with acceptable precision.

### Require the Phase 0 prerequisite

Do not pass `extraction_precision_300` until `phase0_five_day` has passed. The database enforces this order.

### Qualify the browser pipeline

Run live Gmail and Slack canaries against the current web interfaces:

- Gmail new compose, reply, multiple drafts, button send, and keyboard send
- Slack channel, direct message, thread, button send, keyboard send, and multiline input
- Failed send, Input Method Editor composition, quoted text, and rapid duplicate gestures
- Extension-first, app-first, desktop restart, browser restart, and reconnect sequences
- Site disablement with pending outbox records
- Outbox retry after native host or desktop unavailability
- Selector health after successful capture and repeated misses

A selector change requires the full site canary because Gmail and Slack markup is not a stable application programming interface.

### Measure the private corpus

Use at least 300 unique, real sent messages. Evaluate the shipped deterministic baseline with an empty learned blocklist.

The required metrics are:

- Automatic-capture precision of at least 70 percent before closed-loop beta
- Automatic-capture precision target of at least 80 percent before `1.0.0`
- Candidate precision and automatic recall as secondary diagnostics

Keep raw corpus rows outside the repository, continuous integration, caches, artifacts, and secrets. Record aggregate counts and precision only.

### Validate review behavior

Confirm that:

- Scores 6 and above become Open
- Scores 4 and 5 enter Review
- Lower scores do not create promises
- Review promotion creates triggers
- Not a promise learns a local skeleton
- Exact retries do not duplicate capture, promise, trigger, or selector state
- Changed content under one capture identifier produces a conflict
- Disabled sources receive terminal discard without retaining browser retries

### Exit `0.3.0`

Advance only when:

1. `phase0_five_day` remains passed
2. At least 300 valid private rows were evaluated
3. Automatic-capture precision reaches 70 percent
4. `extraction_precision_300` is recorded as passed
5. Live Gmail and Slack canaries pass
6. No message body appears in logs, health data, or actionable notification text

A result between 70 and 80 percent can unlock beta work. It cannot satisfy the `1.0.0` release target.

## Qualify the closed loop in `0.4.0`

`0.4.0` should prove that captured promises return at useful moments without becoming a notification burden.

### Enable the implemented loop

Once extraction evidence passes, exercise:

- Exact web-context matching
- Application fallback matching
- Keyword-to-executable matching
- Deterministic candidate selection
- Daily cap and minimum gap
- Quiet hours and onboarding silence
- One active notification globally
- One surface per promise per local day
- One deadline escalation for an unseen promise
- Done, Snooze, Not a promise, and Ignore notification actions
- Fresh dwell after snooze expiry
- Archival after the third committed ignore
- Warm and cold routing into Promise Detail

### Freeze the acceptance metric first

The existing requirement is at least 40 percent actionable acceptance over two weeks. The repository does not yet freeze the exact denominator or which actions count positively.

Before the trial begins, document:

- Which delivered surfaces enter the denominator
- Whether Windows-suppressed notifications enter the denominator
- Which of Done, Snooze, Not a promise, Ignore, and Open count as accepted
- How duplicate, late, failed, and expired actions are excluded
- How days without a surface affect the trial

Do not change the formula after observing results.

### Run the two-week trial

Use the closed loop daily for 14 days. Record content-free aggregates:

- Delivered surfaces
- Accepted actions under the frozen formula
- Suppressed candidates by policy reason
- Duplicate or late actions
- Deadline escalations
- Unactioned expiries
- False-positive rejections

The code does not currently export these aggregates. Until a privacy-reviewed local report exists, collect only the minimum manual evidence needed.

### Exit `0.4.0`

Advance only when:

1. Extraction evidence remains valid for the candidate
2. The acceptance formula was frozen before the trial
3. Two weeks of daily use completed
4. Actionable acceptance reaches at least 40 percent
5. `acceptance_two_week` is recorded as passed
6. Notification fatigue, stale routing, and deadline behavior have no release-blocking defect

If acceptance fails, adjust trigger or surfacing behavior and repeat the full trial.

## Harden the installed candidate in `0.5.0`

`0.5.0` should convert a successful beta into a reproducible Windows release candidate. It adds no new product premise.

### Resolve current hardening items

Prioritize these implementation risks:

- Age or clear stale native-host connection health
- Confirm or initialize the supported protocol-activation path
- Apply explicit caller authorization to every sensitive Tauri command
- Resolve quiet-hours timezone consistency
- Decide whether the notification route queue must survive a crash
- Remove or isolate the legacy standalone review command and component
- Add a service-worker wake strategy that preserves local-only behavior
- Confirm current-user pipe risk is acceptable or add process authentication
- Freeze extension identity and distribution method
- Freeze signing ownership and certificate handling

Each change must preserve current capture, routing, focus-rule, and privacy behavior.

### Run the complete installed matrix

Qualify NSIS and MSI separately on clean Windows environments. Cover:

- Install, first run, app-first and extension-first order
- Native-host manifest, registry key, path move, and reconnect
- Gmail and Slack canaries
- Phase 0 and extracted focus dwell
- Warm and stopped-app notification actions
- Action Center cleanup
- Focus Assist, lock, sleep, resume, and clock changes
- Tray, single instance, shortcut fallback, and logon autostart
- Retention, purge, reinstall, update, and uninstall
- Installed-runtime Transmission Control Protocol (TCP) and User Datagram Protocol (UDP) endpoint inspection

[`release.md`](release.md) contains the authoritative matrix.

### Produce immutable candidate artifacts

The candidate must contain exactly:

- One NSIS installer
- One MSI installer
- One extension ZIP
- One checksum manifest

Verify each checksum independently. Verify both installers contain the native host and preserve quoted protocol commands.

### Exit `0.5.0`

Advance only when:

1. All three human gates pass
2. The 80 percent extraction release target passes on the frozen private corpus
3. Automated validation passes from a clean revision
4. Both installer paths pass the complete matrix
5. Endpoint inspection confirms the installed runtime contract
6. Signing and extension-distribution decisions are documented
7. A version-specific, content-free evidence record is approved

A draft release or uploaded artifact is not an exit criterion by itself.

## Publish Windows `1.0.0` deliberately

`1.0.0` should be the first supported general-availability release for the documented Windows scope.

### Support only proven targets

The support statement should include:

- Supported Windows versions that passed the installed matrix
- Windows x64 installer architecture
- Google Chrome version floor used by the extension build
- Gmail web and Slack web capture
- Local SQLite and extension-outbox storage
- Native Windows notifications and focus detection

Do not imply support for Edge, Firefox, Slack desktop, macOS, Linux, Android, or iOS.

### Require a trustworthy distribution decision

The preferred general-availability path uses Authenticode for relevant binaries and installers. If signing remains unavailable, keep the release in the `0.x` preview line and retain explicit unsigned warnings rather than labeling it `1.0.0`.

Define one extension installation path:

- Chrome Web Store publication with a verified stable identifier, or
- A clearly supported managed or unpacked deployment process

Do not treat the manifest public key as code signing or Store approval.

### Publish only approved artifacts

The tag must match synchronized version metadata. The release workflow must rebuild and verify candidates, pass through the `windows-release` environment, and create a draft. Treat that environment as protected only when repository settings configure required reviewers or deployment rules.

A release owner then verifies:

- Artifact names and count
- SHA-256 values
- Authenticode state
- Extension identifier
- Evidence record
- Support matrix
- Privacy wording
- Known limitations
- Release and rollback notes

Only a human approval publishes the draft.

## Improve reliability in `1.1.0`

`1.1.0` should address operational blind spots without expanding content collection.

### Planned reliability scope

Candidate work includes:

- Connection liveness with explicit disconnect and age state
- Alarm-backed extension reconnect and policy refresh
- Durable or recoverable notification-route handoff
- Consistent configured-timezone evaluation
- Clear native-host and selector remediation guidance
- Content-free local diagnostics export
- Better migration backup and recovery guidance
- Installed update and rollback qualification
- Removal of unused connection-state scaffolding

Any diagnostics export must remain local and exclude message text, recipient, source context, process path, and window data.

### Exit `1.1.0`

Require regression validation, the affected installed matrix, and privacy review. No new human product gate is planned unless behavior changes the capture or surfacing premise.

## Complete workflows in `1.2.0`

`1.2.0` should fill user-facing management gaps after reliability stabilizes.

### Planned workflow scope

Candidate work includes:

- Edit and delete for focus rules
- Explicit confirmation and undo policy for destructive rule changes
- Resolved-promise activity history
- A defined reopen flow for terminal promises
- Better onboarding failure and retry feedback
- Serialized settings writes with field-specific status
- Search and filters for a larger local inbox
- Accessibility verification across keyboard, scaling, contrast, and screen readers

Rule deletion and promise reopening require durable semantics before UI work starts. Do not add optimistic state that can disagree with SQLite.

### Exit `1.2.0`

Require migration safety where applicable, stale-write protection, accessibility review, and regression coverage for focus, notification routing, capture, and privacy.

## Keep `2.0.0` ideas exploratory

Post-`1.x` scope has no release commitment. Every expansion needs a fresh feasibility and privacy decision.

### Consider additional web sources

Possible selector-based sources include GitHub, Linear, Discord, or Microsoft Teams web. A source can enter planning only when it has:

- Compose-scoped authored-text extraction
- Reliable send confirmation
- A stable local context identifier
- Minimal host permissions
- Content-free selector health
- A complete live canary matrix
- A maintenance owner

Do not broaden host permissions before accepting the source.

### Consider additional Chromium browsers

Edge and other Chromium browsers need separate native-host registration, extension identity, executable matching, packaging, and installed tests. Chrome support does not imply browser portability.

### Consider macOS or Linux as separate products

A supported desktop port requires real implementations for:

- Foreground application observation
- Secure local host transport
- Native notifications and action activation
- Autostart and lifecycle behavior
- Packaging, signing, and continuous integration
- Installed endpoint and privacy validation

Compile-safe no-op adapters are not a starting release.

### Consider a local model only with evidence

A local language model remains default-off research. Start only if private-corpus results prove deterministic extraction has reached a measured ceiling that blocks useful recall or precision.

Any model path must remain local, bounded, inspectable, optional, and compatible with retention and purge. It cannot introduce telemetry, remote inference, silent downloads, or weaker fallback behavior.

## Preserve explicit non-goals

The planned release train excludes:

- Accounts, login, cloud sync, or a Callback server
- Telemetry or remote content processing
- Calendar writes, email sending, or task assignment
- Team project management
- Meeting transcription or voice capture
- Reading arbitrary clipboard or selected text
- Slack desktop capture in the Chrome extension
- Mobile foreground tracking
- Runtime selector downloads
- Automatic completion of human evidence gates

Changing a non-goal requires a product decision, privacy update, architecture review, and revised release plan.

## Prioritize future work by risk

When two planned items compete, use this order:

1. Data-loss, privacy, or authorization defects
2. Capture correctness and duplicate prevention
3. Notification action durability and stale-state safety
4. Selector breakage and health remediation
5. Installed lifecycle and distribution failures
6. Accessibility barriers
7. Workflow completeness
8. New sources or platforms
9. Optional extraction technology

This ordering protects trust before expanding surface area.
