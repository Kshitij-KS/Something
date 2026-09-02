# Callback

Callback captures commitments from confirmed Gmail and Slack messages, then resurfaces them when the relevant application or web context returns. The local Windows system combines a Tauri desktop app, Chrome Manifest V3 extension, native-messaging host, focus watcher, and SQLite database.

> **Current status:** version `0.1.0` is an unsigned Windows engineering preview. Focus rules, browser capture, extraction, the Promise Inbox, gated surfacing, diagnostics, and packaging are implemented. All repository-default human evidence gates remain pending, so existing installers are test candidates rather than proof of installed-device qualification or a public `1.0` release.

## Check the repository status

The current scope is narrow and explicit:

| Area             | Current state                                                                                                                                                                     |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Desktop          | Windows x64 target with Nullsoft Scriptable Install System (NSIS) and Windows Installer (MSI) packaging                                                                           |
| Browser          | Google Chrome with a Manifest V3 extension                                                                                                                                        |
| Capture sites    | Gmail web and Slack web                                                                                                                                                           |
| Storage          | Local SQLite plus a bounded `chrome.storage.local` outbox                                                                                                                         |
| Network boundary | Static source and audit checks identify no Callback-owned Transmission Control Protocol (TCP) or User Datagram Protocol (UDP) path; installed endpoint inspection remains pending |
| Distribution     | Unsigned test artifacts; no Store, winget, or Scoop publication claim                                                                                                             |
| Human evidence   | Phase 0, extraction precision, and acceptance gates default to `pending_user`                                                                                                     |
| Other platforms  | macOS and Linux compile-oriented no-op boundaries; no supported runtime                                                                                                           |

The [`0.1.0` evidence snapshot](docs/release-evidence/0.1.0.md) preserves an unattributed historical local report of 116 passing runnable Rust tests and 14 passing extension tests. It has no retained timestamp, transcript, or clean source revision and is not candidate qualification evidence. One private-corpus test remains intentionally ignored without a local 300-message corpus. Re-run every check for the revision you plan to test or release.

## Use the implemented features

The repository currently implements:

- Persistent focus rules with visible-app discovery, manual executable entry, five-second dwell, and per-rule pause or resume
- Confirmed-send capture from Gmail web and Slack web
- A bounded, retry-safe extension outbox and versioned native-messaging transport
- Deterministic promise extraction, deadline parsing, review routing, and local rejection learning
- A Promise Inbox with Open, Snoozed, Review, and Resolved views
- Promise editing, promotion, completion, snooze, resume, rejection, and three-skip archival
- Context-triggered and one-time deadline surfacing behind the extraction evidence gate
- Durable notification leases, one-time action tokens, warm and cold action routing, and generic actionable toast copy
- Daily limits, minimum spacing, quiet hours, onboarding silence, and fresh-dwell requirements
- Global Quick capture, tray residency, single-instance behavior, configurable shortcuts, and optional autostart
- Settings, selector health, native-host reconnect, retention, and local purge
- Windows continuous integration, unsigned NSIS/MSI packaging, extension ZIP validation, and SHA-256 manifests

Read [`docs/current-state.md`](docs/current-state.md) for the complete feature inventory, code map, gate behavior, and known limitations.

## Read the documentation

Use these references according to your task:

- [`docs/current-state.md`](docs/current-state.md): implemented behavior, code ownership, validation coverage, and limitations
- [`docs/architecture.md`](docs/architecture.md): runtime processes, data flows, state transitions, trust boundaries, and failure recovery
- [`docs/development.md`](docs/development.md): setup, commands, checks, generated outputs, and local packaging
- [`docs/privacy.md`](docs/privacy.md): installed-runtime privacy contract, retained data, and exclusions
- [`docs/kill-gates.md`](docs/kill-gates.md): human evidence rules and private-corpus procedure
- [`docs/roadmap.md`](docs/roadmap.md): future scope and gate-driven planned versions
- [`docs/release.md`](docs/release.md): candidate qualification, Windows matrix, artifact contract, and publication flow
- [`docs/documentation-plan.md`](docs/documentation-plan.md): documentation authority and maintenance rules

[`callback-spec.md`](callback-spec.md) preserves the original product and build proposal. It is historical and may describe superseded schema, dependencies, timelines, or distribution plans.

## Start local development

Install Node.js 20, Rust 1.85 or newer, and the Windows prerequisites required by Tauri. Then run:

```powershell
npm ci
npm run build:extension
npm run stage:native-host
npm run tauri dev
```

Load `extension/dist` as an unpacked Chrome extension after the extension build. The Vite development server uses localhost and sits outside the installed-runtime no-listener guarantee.

Run the complete local qualification and packaging sequences from [`docs/development.md`](docs/development.md). Do not infer installed behavior from a successful source build.

## Preserve the privacy boundary

Confirmed content stays on the local machine. It travels from the content script to `chrome.storage.local`, Chrome native messaging, an origin-pinned native host, an owner-only named pipe, and SQLite.

Extracted actionable notifications use generic text. User-authored focus-rule text can appear in Windows notification history or on the lock screen. Read [`docs/privacy.md`](docs/privacy.md) before changing capture, transport, storage, logging, or notification code.

## License

Callback is licensed under the MIT License. See [`LICENSE`](LICENSE).
