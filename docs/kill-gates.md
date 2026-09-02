---
meta:
  title: Qualify Callback with human evidence
  navLabel: Kill Gates
  category: Project
  contentType: How-to
  plan: ./documentation-plan.md
---

# Qualify Callback with human evidence

Callback uses three ordered human-evidence gates that continuous integration cannot complete. The repository-default status of every gate is `pending_user`.

## Separate local status from repository readiness

Each installation stores gate status in SQLite table `kill_gates`. That local record can differ from this repository document.

Use the repository status as the release baseline. Do not treat one developer's local database as global release approval.

| ID                         | Requirement                                                                                 | Repository status | Release effect                               |
| -------------------------- | ------------------------------------------------------------------------------------------- | ----------------- | -------------------------------------------- |
| `phase0_five_day`          | Use focus rules for five days, then make and explain a continue-or-stop decision            | `pending_user`    | Required before extraction evidence can pass |
| `extraction_precision_300` | Evaluate 300 real sent messages with at least 70 percent automatic-capture precision        | `pending_user`    | Unlocks extracted and deadline surfacing     |
| `acceptance_two_week`      | Use the closed loop daily for two weeks and reach at least 40 percent actionable acceptance | `pending_user`    | Required before launch qualification         |

General availability also requires at least 80 percent automatic-capture precision. The evaluator reports this release target separately from the 70 percent enablement threshold.

## Follow prerequisite order

The database enforces this order:

1. `phase0_five_day`
2. `extraction_precision_300`
3. `acceptance_two_week`

A downstream gate cannot pass before its prerequisite. A passed upstream gate cannot reset while its dependent remains passed.

For every pass or failure, record the trial duration, opportunity count, useful and incorrect surface counts, blocking defects, and final decision. The database validates note length and gate order, not the truth or completeness of those fields.

## Run the five-day Phase 0 gate

This gate tests the product premise before extraction influences notifications.

During five days of real use:

- Create a focus rule for a known application
- Confirm a fresh five-second dwell shows the expected notification
- Compare usefulness with a clock-based reminder
- Exercise pause and resume across restart
- Observe false, mistimed, duplicate, or missing surfaces
- Exercise lock, sleep, resume, and Windows Focus Assist
- Confirm no private process details leave the local boundary

Pass only when the tester chooses to keep using context-triggered reminders and supports that decision with the recorded opportunity, useful-surface, and incorrect-surface counts. Do not pass the gate because the implementation worked mechanically. The gate has no numeric usefulness threshold.

Record content-free evidence such as days completed, reminder opportunities, useful surfaces, incorrect surfaces, and the final premise decision. Do not include reminder text, window titles, process paths, identifiers, or unrelated application data.

## Prepare the private extraction corpus

Keep the real corpus outside version control. Each nonblank JSON Lines row must contain exactly four fields:

- Unique nonempty `id` with no leading or trailing whitespace
- `source` equal to `gmail` or `slack`
- Nonempty `text`
- `label` equal to `promise` or `not_promise`

Unknown fields are rejected. Follow `src-tauri/tests/fixtures/messages.jsonl.example`.

Classification is message-level:

1. Any Capture clause makes the message Capture
2. Otherwise, any Review clause makes the message Review
3. Otherwise, the message is Discard

The evaluator uses the shipped baseline, an empty learned blocklist, the `UTC` timezone, zero offset, and a fixed evaluation instant of `2026-01-15 12:00:00 UTC`.

## Calculate extraction metrics

The gate metric is automatic-capture precision:

`true promise labels among Capture results / all Capture results`

The evaluator also reports:

- Capture plus Review candidate precision
- Automatic recall
- Whether the 80 percent release target was reached

At least 70 percent automatic-capture precision is required to pass `extraction_precision_300`. At least 80 percent is required by the release plan before `1.0.0`.

## Run the local evaluator

Set the local corpus path and invoke the ignored test explicitly:

```powershell
$env:CALLBACK_PRIVATE_CORPUS = `
  (Resolve-Path '.\src-tauri\tests\fixtures\messages.jsonl').Path
npm run evaluate:private-corpus
Remove-Item Env:CALLBACK_PRIVATE_CORPUS
```

The command fails when it finds:

- Malformed, invalid, or duplicate rows
- Fewer than 300 valid unique messages
- Undefined automatic precision
- Automatic-capture precision below 70 percent

The evaluator emits aggregate counts and metrics only. It does not print corpus paths, identifiers, message text, clauses, or per-message results. It does not access the network, mutate the app database, or record a gate decision.

After `phase0_five_day` passes, record aggregate extraction evidence locally. Never add the corpus, environment variable, or raw output to continuous integration, artifacts, caches, repository secrets, screenshots, or support logs.

## Freeze actionable acceptance before the trial

The current product requirement is at least 40 percent actionable acceptance over two weeks. The historical plan does not define a complete formula.

Before starting the trial, freeze:

- Which delivered surfaces enter the denominator
- How operating-system-suppressed notifications are treated
- Which actions count positively
- How Open, Ignore, Not a promise, duplicate, late, expired, and failed actions count
- Trial start and end boundaries

Store that formula with the candidate evidence. Do not revise it after inspecting results.

## Run the two-week acceptance gate

After extraction passes, use the closed loop daily for 14 days. Exercise context surfaces, deadline escalation, Done, Snooze, Resume, Not a promise, Ignore, and notification routing.

Record content-free aggregates under the frozen formula. Pass only when actionable acceptance reaches at least 40 percent. Repeated surfaces beyond configured policy, stale-target notifications, or any defect that forces the tester to disable notifications invalidates the trial.

If the result fails, keep `acceptance_two_week` failed or pending, adjust behavior, and repeat the full trial. Do not average incompatible formulas or candidates.

## Keep installed checks separate

Human gates measure product usefulness. They do not replace the packaged Windows matrix.

Installed checks still include:

- Stopped-app and warm-process notification actions
- Physical or virtual machine lock, sleep, and resume
- Windows Focus Assist or Do Not Disturb
- Current-user autostart after real logon
- Native-host registration and reconnect
- Nullsoft Scriptable Install System (NSIS) and Windows Installer (MSI) install, upgrade, purge, and uninstall
- Gmail and Slack live canaries
- Installed-process Transmission Control Protocol (TCP) and User Datagram Protocol (UDP) inspection
- Authenticode and SmartScreen behavior

Read [`release.md`](release.md) for the complete matrix.

## Record only content-free evidence

A version evidence record may include:

- Candidate version and revision
- Trial duration
- Aggregate counts and percentages
- Pass, fail, or pending decision
- Content-free defect references
- Reviewer and approval

It must not include messages, clauses, recipients, contexts, reminder text, window titles, process paths, tokens, or private corpus rows.

Use `docs/release-evidence/` for repository-reviewed release records. A local SQLite note alone does not publish a result.
