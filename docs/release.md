---
meta:
  title: Qualify and release Callback for Windows
  navLabel: Release Process
  category: Project
  contentType: How-to
  plan: ./documentation-plan.md
---

# Qualify and release Callback for Windows

This guide defines how a source revision becomes an unsigned candidate, an installed-build-qualified candidate, a draft, and a supported release. A successful build is only the first step.

## Distinguish release states

Use these states consistently:

| State                     | Meaning                                                                                                  |
| ------------------------- | -------------------------------------------------------------------------------------------------------- |
| Source build              | The code compiles or tests pass on one machine                                                           |
| Candidate artifacts       | Reproducible payloads and checksums exist                                                                |
| Human-gate qualified      | Required private trials passed with evidence                                                             |
| Installed-build qualified | Nullsoft Scriptable Install System (NSIS) and Windows Installer (MSI) packages passed the Windows matrix |
| Draft release             | Verified assets exist in a nonpublic GitHub draft                                                        |
| Supported release         | A release owner approved and published the qualified draft                                               |

`0.1.0` is currently an engineering preview. Repository-default gates remain pending, artifacts are unsigned, and installed-device certification is incomplete.

## Require evidence before release work

A public candidate must satisfy four categories:

1. Automated code and artifact checks
2. Human product evidence
3. Installed Windows behavior
4. Distribution and approval policy

Do not substitute one category for another. Test counts do not prove five days of use. A checksum does not prove publisher identity. A draft does not prove installation.

## Start from a clean release revision

Before building:

1. Select one reviewed revision
2. Confirm the working tree contains no unintended changes
3. Confirm `Cargo.lock` and `package-lock.json` belong to that revision
4. Confirm the version appears consistently across every manifest
5. Confirm release notes match implemented behavior
6. Confirm no human gate or matrix item is represented as passed without evidence

Run:

```powershell
git status --short
npm ci
npm run verify:release-metadata
```

The tagged draft-publication path requires a tag named exactly `v` plus the package version, such as `v0.1.0`. Manual workflow dispatch builds a test candidate without a tag or draft.

## Synchronize version and identity

Update one semantic version across:

- `package.json`
- Root metadata in `package-lock.json`
- `src-tauri/tauri.conf.json`
- `extension/manifest.json`
- `src-tauri/Cargo.toml`
- `crates/native-host/Cargo.toml`
- `crates/protocol/Cargo.toml`

`npm run verify:release-metadata` also checks that the manifest key-derived extension identifier matches both Rust native-host allowlists.

The `release-candidate` job currently reads `CALLBACK_EXTENSION_ID` from a repository or organization Actions variable. Set that variable only after a real published extension identity exists. The job does not attach to `windows-release`, so an environment-scoped variable is unavailable unless the workflow changes. Leaving the variable unset makes no Store claim.

## Pass the automated qualification

Run the strict local sequence in [`development.md`](development.md) before qualification. The current Windows workflows provide a reduced candidate profile: they run formatting, non-release Clippy, non-release tests, frontend checks, builds, metadata validation, privacy audit, and packaging. They do not run the documented workspace release check, release-profile tests, no-default-features check, or `git diff --check`.

The release policy still requires every category below. Do not treat the current workflow as a substitute for omitted strict checks until the workflow is aligned.

### Rust checks

The candidate must pass:

- `cargo fmt --all -- --check`
- Workspace release check with the lockfile
- Workspace release tests serialized on Windows
- Workspace Clippy across all targets with warnings denied
- `callback-app` release check without default features

### TypeScript and extension checks

The candidate must pass:

- Desktop and extension type checking
- ESLint with zero warnings
- Prettier check
- Vitest run
- Source and built extension validation
- Production app and extension bundles

### Contract and privacy checks

The candidate must pass:

- Version and extension-origin synchronization
- Static local-only audit
- Git diff whitespace validation
- Protocol and native-host integration suites
- Database migration and durable-state suites

### Candidate build checks

The candidate must produce:

- Locked native-host release binary
- Tauri release executable
- NSIS installer
- MSI installer
- Extension ZIP
- Checksum manifest

The private extraction corpus remains outside continuous integration. Its aggregate evidence is a separate release requirement.

## Build the Windows payloads

Run the canonical local sequence:

```powershell
npm run stage:native-host
npm run build
npx --no-install tauri build --bundles nsis,msi
npm run package:windows
```

The Windows Tauri configuration bundles the native host as a sidecar. The packaging script then collects and verifies existing outputs.

## Enforce the artifact contract

For version `0.1.0`, the current naming contract is:

| Payload            | Current filename               |
| ------------------ | ------------------------------ |
| NSIS installer     | `Callback_0.1.0_x64-setup.exe` |
| MSI installer      | `Callback_0.1.0_x64_en-US.msi` |
| Chrome extension   | `callback-extension-0.1.0.zip` |
| Integrity manifest | `SHA256SUMS.txt`               |

A future release substitutes its synchronized semantic version without changing the roles.

### Verify candidate contents

Confirm:

- `artifacts/` contains exactly three payloads and one checksum file
- `SHA256SUMS.txt` contains exactly one lowercase SHA-256 line per payload
- Independent hashes match every listed value
- The extension ZIP has no parent paths, duplicate entries, or extra files
- The extension ZIP root contains `manifest.json`, `background.js`, `content.js`, and `selectors.json`
- NSIS and MSI both contain `callback-native-host.exe`
- Protocol command registration quotes the executable and `%1`
- Signature state matches release notes

Do not modify an artifact after hashing. Rebuild and regenerate the manifest instead.

## Complete every human gate

[`kill-gates.md`](kill-gates.md) defines three ordered gates:

1. Five-day Phase 0 use
2. Private 300-message extraction evaluation
3. Two-week closed-loop acceptance trial

For a supported `1.0.0` release, require both extraction thresholds:

- At least 70 percent to unlock closed-loop testing
- At least 80 percent for release

The database enforces prerequisite order but cannot verify that evidence is truthful. A release owner must review the content-free record.

## Freeze the acceptance formula

Before starting the two-week acceptance trial, define:

- Delivered-surface denominator
- Treatment of operating-system-suppressed notifications
- Positive actions
- Neutral actions
- Rejection and ignore treatment
- Duplicate, late, and expired action exclusions
- Trial start and end boundaries

Store the definition with the version evidence. Do not tune it after viewing results.

## Qualify NSIS and MSI separately

Use clean Windows physical or virtual machine sessions. Install either NSIS or MSI in one session, never both at once.

Record for each run:

- Windows edition, version, build, and architecture
- Chrome version
- Installer type and SHA-256
- Signature state
- Existing Callback or extension state
- Start and end result
- Content-free defect references

Reset the environment before testing the other installer.

## Run the installed Windows matrix

Every item remains pending until exercised on the packaged build.

### Install and first-run behavior

Verify:

- NSIS clean install, launch, close-to-tray, explicit Quit, and uninstall
- MSI clean install, launch, close-to-tray, explicit Quit, and uninstall
- First-run application-data creation
- Database migration from every supported prior schema
- Native-host sidecar placement
- Native-host manifest path and content
- Current-user registry registration
- No administrator requirement for normal runtime
- Unsigned SmartScreen disclosure when applicable

### App and extension ordering

Verify:

- Desktop installed before extension
- Extension installed before first desktop launch
- Browser running during desktop install and first launch
- Desktop restart while browser remains open
- Browser restart while desktop remains resident
- **Reconnect extension** after a path or registration change
- Multiple Chrome profiles without cross-profile capture confusion
- Extension disable, enable, remove, and reinstall

### Focus rules

Verify:

- Visible-app discovery returns executable basenames only
- Manual executable entry remains usable after discovery failure
- Create exactly one rule for a test application
- A fresh five-second focus dwell shows the reminder
- Rapid focus changes do not show stale reminders
- Same-target events do not restart or duplicate dwell
- Pause persists after app restart
- Paused rules do not fire after a fresh dwell
- Resume restores later eligible firing
- Rapid same-row clicks produce one authoritative transition
- Different rows report independent state
- An exact paused duplicate is not silently resumed

### Gmail capture

Verify:

- New compose and reply
- Multiple open drafts
- Button send
- Control or Command plus Enter send
- Failed send
- Empty body
- Quoted history exclusion
- Correct recipient and compose scope
- Site disablement and re-enable
- Selector health after success and forced miss

Do not record message bodies in test notes.

### Slack capture

Verify:

- Channel, direct message, and thread composer
- Button send
- Enter send
- Shift plus Enter multiline input
- Input Method Editor composition
- Multiple visible composers
- Single-page route and workspace changes
- Editor replacement during send
- Failed confirmation and immediate retry
- Site disablement and re-enable
- Selector health and sanitized capture-stage data

Use synthetic nonpersonal text for installed smoke tests.

### Outbox and transport

Verify:

- Capture while the desktop is stopped
- Capture while the host registration is broken
- Ordered flush after reconnect
- Exact retry without duplicate promises
- Terminal discard after source disablement
- Native-host restart after core loss
- Host standard output contains framed protocol bytes only
- Named pipe rejects another Windows account
- Oversized or malformed envelopes fail safely

The current-user pipe does not reject a malicious same-user process. Record this as an accepted limitation or block the release pending hardening.

### Extraction and Promise Inbox

Verify:

- Open, Review, and discard routing at known score boundaries
- Deadline parsing in the configured timezone
- Daylight-saving gap and repeated-time rejection
- Review promotion and Not a promise
- Open, Snoozed, Review, and Resolved tabs
- Edit text and deadline
- Done, Snooze, Resume, Not a promise, and Ignore
- Archive after the third Ignore
- Dirty-draft protection during navigation and notification routing
- Stale-write rejection across two state changes
- Missing record after retention

### Extracted surfacing policy

Run this group only after extraction evidence passes. Verify:

- Exact web context
- Source-application fallback
- Keyword-to-executable fallback
- Earliest-deadline candidate ordering
- One active notification
- Daily cap
- Minimum gap
- Same-promise local-day suppression
- Quiet hours
- Onboarding silence
- Clock rollback protection
- One deadline escalation
- Fresh dwell after snooze expiry
- No Phase 0 fallback after an extracted candidate is policy-suppressed

### Notification delivery and action routing

Verify:

- Generic actionable title and body
- No captured clause in Action Center
- Open action with the app already running
- Done, Snooze, Not a promise, and Ignore while running
- Every action from a stopped app
- Single-instance forwarding
- First-in-first-out routing for several activations
- Dirty-draft defer, open, and dismiss choices
- Duplicate action token behavior
- Late action token behavior
- Action Center cleanup after purge
- Windows Focus Assist or Do Not Disturb behavior

### Session and lifecycle behavior

Verify:

- Workstation lock during pending dwell
- Unlock with the same foreground app
- Sleep during pending dwell
- Resume with the same foreground app
- Clock and timezone changes
- Main-window close hides to tray
- Tray open and explicit Quit
- Second-instance launch
- Primary shortcut
- Fallback shortcut after collision
- Shortcut reconfiguration
- Optional autostart after a real sign-out or reboot

### Retention, purge, and uninstall

Verify:

- Retention on startup
- Immediate retention after a setting change
- Redaction of old unresolved source bodies
- Deletion of eligible old resolved and review captures
- Retry behavior before and after receipt expiry
- Purge while the app is running
- Database, journal, manifest, registry, autostart, and notification cleanup
- Extension outbox remains until extension removal
- NSIS uninstall cleanup
- MSI uninstall cleanup
- Reinstall after purge and after uninstall

Document any state intentionally retained by uninstall.

### Runtime endpoint inspection

Inspect installed Callback processes while exercising capture, focus, health, notifications, and idle residency. Include:

- `callback-app.exe`
- `callback-native-host.exe`
- Purge helper mode when used

Confirm no Callback-owned Transmission Control Protocol (TCP) or User Datagram Protocol (UDP) listener and no captured-content remote connection. Exclude normal Chrome, Gmail, Slack, Store, package-manager, development-server, and operating-system traffic from the product claim.

The static audit cannot replace this check.

### Accessibility and layout

Verify:

- Keyboard-only navigation
- Visible focus indicators
- Screen-reader names and live feedback
- 44 px action targets where specified
- 200 percent scaling
- Narrow window layout
- Long executable, reminder, recipient, and promise text
- High-contrast mode
- Reduced-motion behavior when motion exists

## Make an explicit signing decision

Authenticode signing is not configured in the repository.

For general availability:

1. Assign certificate ownership and renewal responsibility
2. Keep private keys outside the repository and build logs
3. Sign the intended binaries and installers
4. Verify signatures on clean Windows systems
5. Generate checksums after final signed bytes exist
6. Record timestamping and certificate chain details without secrets

If signing is deferred, keep the release in the preview line. State that SmartScreen and Mark-of-the-Web warnings are expected.

The extension manifest key stabilizes identity. It is not a Windows signature or Chrome Web Store approval.

## Use the GitHub release workflow safely

`.github/workflows/release-windows.yml` has two jobs.

### Build a release candidate

A manual dispatch or `v*` tag runs source checkout, locked validation, unsigned bundle creation, packaging, and artifact upload. A tag must match the package version.

Use manual dispatch for test candidates. Do not create a release tag while human gates or installed checks remain open.

### Create a draft after a tag

The publish job:

1. Downloads the verified candidate into a separate job
2. Rechecks checksums without running project code
3. Requires the `windows-release` environment
4. Creates a draft GitHub release
5. Uploads all four files

Configure required reviewers in repository settings. The workflow file cannot prove that environment protection exists.

### Publish by human decision

The workflow stops at a draft. Before publication, verify evidence, signatures, support scope, privacy text, hashes, extension identity, release notes, and rollback instructions.

Do not claim package-manager or Chrome Web Store publication unless those channels completed independently.

## Add downstream channels after immutable assets

Create winget or Scoop metadata only after final GitHub asset URLs and hashes cannot change. Test installation, upgrade, and uninstall through each channel.

Handle Chrome Web Store submission separately. Store review, identifier, policy compliance, and listing status are not implied by desktop release automation.

Do not add Homebrew until a supported macOS implementation, signing path, package, and continuous-integration matrix exist.

## Record version evidence without private content

Create one reviewed record under `docs/release-evidence/` for every candidate that reaches human or installed qualification. The record should contain:

- Version and source revision
- Candidate workflow or local build identity
- Artifact filenames, sizes, and SHA-256 values
- Signature state
- Automated validation summary
- Human-gate status and aggregate metrics
- Acceptance formula version
- NSIS and MSI matrix status
- Runtime endpoint inspection result
- Known limitations and accepted risks
- Approver and decision

Do not include corpus rows, message text, recipients, source contexts, window titles, process paths, identifiers, handles, tokens, or secrets.

A pending record is useful. It prevents generated artifacts from being mistaken for a qualified release.

## Prepare rollback and support notes

Before publication, document:

- How to disable Gmail or Slack capture
- How to pause focus rules
- How to reconnect the extension
- How to avoid exporting private data during support
- How to purge local state
- Whether downgrade across the current schema is supported
- How to uninstall each installer format
- Which local files or browser storage may remain

Do not recommend replacing a newer database with an older binary unless you tested schema compatibility.

## Validate after publication

After release:

1. Download each public asset through the published path
2. Reverify SHA-256 and Authenticode state
3. Install through every supported channel
4. Repeat Gmail and Slack canaries
5. Confirm notification actions and native-host registration
6. Confirm the documented privacy boundary through endpoint inspection
7. Record regressions against the released version

Ship selector or native-host fixes through the same candidate, evidence, draft, and approval process. Do not silently broaden permissions or runtime networking.
