---
meta:
  title: Develop and validate Callback
  navLabel: Development
  category: Project
  contentType: How-to
  plan: ./documentation-plan.md
---

# Develop and validate Callback

This guide covers Windows setup, local development, validation, private-corpus evaluation, and unsigned candidate packaging. It distinguishes frontend builds, Rust builds, installer generation, and artifact collection.

## Install the required tools

Use a Windows development environment with:

- Node.js 20.19.0 or newer within the 20.x line, or another version allowed by the locked package engine ranges
- Rust 1.85 or newer with `rustfmt` and `clippy`
- Microsoft C++ build tools required by Tauri and Rust Windows targets
- Microsoft Edge WebView2
- Nullsoft Scriptable Install System (NSIS) when building that installer locally
- Google Chrome when exercising the extension

Use the checked-in `package-lock.json` and `Cargo.lock`. Do not update dependencies as part of an unrelated feature or documentation change.

## Understand the repository layout

The top-level directories divide responsibilities:

| Path                  | Contents                                                                       |
| --------------------- | ------------------------------------------------------------------------------ |
| `src/`                | React desktop and Quick capture interfaces                                     |
| `src-tauri/`          | Tauri runtime, Windows adapters, SQLite, commands, and integration tests       |
| `extension/`          | Chrome manifest, selectors, content scripts, service worker, and Vitest suites |
| `crates/protocol/`    | Shared native-message protocol and schema                                      |
| `crates/native-host/` | Chrome-launched standard-input/output bridge                                   |
| `scripts/`            | Staging, validation, audit, metadata, and Windows artifact scripts             |
| `.github/workflows/`  | Windows continuous integration and release-candidate automation                |
| `docs/`               | Current references, roadmap, evidence rules, and release process               |
| `dist/`               | Generated React application bundle                                             |
| `extension/dist/`     | Generated Chrome extension bundle                                              |
| `target/`             | Generated Rust and Tauri build output                                          |
| `artifacts/`          | Generated candidate payloads and checksum manifest                             |

Git ignores the generated directories. Their presence does not prove a clean build, release approval, or installed qualification.

## Install locked dependencies

Install JavaScript dependencies from the lockfile:

```powershell
npm ci
```

Cargo commands use `--locked` so Rust dependency resolution also stays fixed.

## Build and load the extension

Build the Manifest V3 extension before loading it into Chrome:

```powershell
npm run build:extension
npm run validate:extension -- extension/dist/manifest.json
```

Then open Chrome's extension manager, enable developer mode, choose **Load unpacked**, and select `extension/dist`.

The build emits exactly these runtime files at the ZIP root:

- `manifest.json`
- `background.js`
- `content.js`
- `selectors.json`

The checked-in manifest key determines the development extension identifier. Keep that identity synchronized with both Rust `ALLOWED_ORIGIN` constants.

## Stage the native host

Tauri bundles the native host as an external binary. Build and stage it before starting the desktop app:

```powershell
npm run stage:native-host
```

This script performs a locked release build for `callback-native-host`, detects the Rust host target, and copies the executable to Tauri's ignored `src-tauri/binaries` path with its target suffix.

## Run the desktop in development

Start the Tauri development process after staging the host:

```powershell
npm run tauri dev
```

The app attempts to register the staged or sibling native host under the current Windows account. Open **Health** and use **Reconnect extension** if registration must be rewritten.

Vite listens on `127.0.0.1:1420` during development. This local development listener sits outside the installed production runtime privacy guarantee.

The application database normally lives under the Tauri application-data directory. On Windows, the expected path is:

```text
%APPDATA%\com.callback.desktop\callback.db
```

Do not edit a live database by hand. Use the UI, commands, or a disposable copy.

## Run the qualification checks

Run checks from the repository root. The following sequence matches the most recent strict local qualification profile.

### Validate Rust

Set incremental compilation off for reproducible release-oriented checks:

```powershell
$env:CARGO_INCREMENTAL = '0'
cargo fmt --all -- --check
cargo check --workspace --release --locked
cargo test --workspace --release --locked -j 1
cargo clippy --workspace --all-targets --release --locked -j 1 -- -D warnings
cargo check --release --no-default-features --locked -p callback-app
Remove-Item Env:CARGO_INCREMENTAL -ErrorAction SilentlyContinue
```

The no-default-features check confirms that compile-oriented platform boundaries remain intact. It does not certify a functional non-Windows runtime.

### Validate TypeScript and generated bundles

Run the frontend, extension, formatting, metadata, and privacy checks:

```powershell
npm run typecheck
npm run lint
npm run format:check
npm run test
npm run validate:extension
npm run build
npm run validate:extension -- extension/dist/manifest.json
npm run verify:release-metadata
npm run audit:local-only
git diff --check
```

`npm run test` already executes Vitest once. Do not add a watch flag to qualification runs.

The static local-only audit scans selected installed-runtime roots and built extension JavaScript. It checks selected network APIs, prohibited synchronized-storage references, runtime selector-fetch patterns, direct log statements containing `rawMessage` or `raw_message`, and known telemetry dependency names. It does not cover every storage alias, logging flow, test, build script, repository script, manifest, migration, or configuration file. It does not replace installed-process endpoint inspection.

### Interpret test scope

The Rust suites cover data, protocol, extraction, focus logic, policy, actions, and composition. The Vitest suites cover extension capture helpers and the outbox.

No current automated suite validates:

- React behavior through a browser renderer
- Live Gmail or Slack markup
- Chrome service-worker suspension
- Real Windows toast, lock, sleep, or Focus Assist behavior
- Installed native-host registry and path handling
- Installer upgrade and uninstall behavior
- Installed-runtime Transmission Control Protocol (TCP) or User Datagram Protocol (UDP) endpoints

Use the manual matrix in [`release.md`](release.md) for those surfaces.

## Evaluate the private extraction corpus

The private evaluator is intentionally ignored during normal tests. Prepare a local JSON Lines corpus that follows `src-tauri/tests/fixtures/messages.jsonl.example`.

Run it explicitly:

```powershell
$env:CALLBACK_PRIVATE_CORPUS = `
  (Resolve-Path '.\src-tauri\tests\fixtures\messages.jsonl').Path
npm run evaluate:private-corpus
Remove-Item Env:CALLBACK_PRIVATE_CORPUS
```

The evaluator requires at least 300 valid unique rows. It fails below 70 percent automatic-capture precision and reports the separate 80 percent release target.

Never add the corpus, environment variable, raw examples, or per-message output to Git, continuous integration, artifacts, caches, or repository secrets. Read [`kill-gates.md`](kill-gates.md) before recording a result.

## Build unsigned Windows candidates

Generate application bundles and collect artifacts in this order:

```powershell
npm run stage:native-host
npm run build
npx --no-install tauri build --bundles nsis,msi
npm run package:windows
```

The commands have distinct roles:

- `stage:native-host`: builds and stages the sidecar
- `build`: type-checks and builds the React app and extension
- `tauri build`: compiles the desktop and creates NSIS and Windows Installer (MSI) bundles
- `package:windows`: collects existing bundles, creates the extension ZIP, and verifies checksums

`package:windows` is not the installer compiler. It expects successful Tauri and extension outputs.

## Verify the artifact contract

A successful `0.1.0` collection creates:

```text
artifacts\Callback_0.1.0_x64-setup.exe
artifacts\Callback_0.1.0_x64_en-US.msi
artifacts\callback-extension-0.1.0.zip
artifacts\SHA256SUMS.txt
```

The packaging script enforces:

- Exactly one NSIS installer
- Exactly one MSI installer
- An extension ZIP containing the four required root entries
- No duplicate, absolute, or parent-traversal ZIP paths
- One lowercase SHA-256 entry for each payload
- Immediate checksum reverification

The script does not reject every additional safe or nested ZIP entry. Release qualification must inspect the archive and require exactly the four documented root files.

The outputs are unsigned. A checksum proves byte integrity after download. It does not establish publisher identity.

Treat `artifacts/` as disposable output. Rebuild candidates from a clean revision instead of carrying files forward.

## Use the package scripts correctly

The root scripts have these meanings:

| Script                            | Purpose                                                    |
| --------------------------------- | ---------------------------------------------------------- |
| `npm run dev`                     | Start the Vite frontend only                               |
| `npm run build`                   | Type-check and build app plus extension; no Rust installer |
| `npm run build:app`               | Build the React application bundle                         |
| `npm run build:extension`         | Build the Chrome extension bundle                          |
| `npm run stage:native-host`       | Build and stage the Rust sidecar                           |
| `npm run typecheck`               | Check desktop and extension TypeScript                     |
| `npm run lint`                    | Run ESLint with zero warnings                              |
| `npm run test`                    | Run extension Vitest suites once                           |
| `npm run format:check`            | Check Prettier formatting                                  |
| `npm run validate:extension`      | Validate source or built extension permissions and files   |
| `npm run verify:release-metadata` | Compare versions and extension identity                    |
| `npm run audit:local-only`        | Run the static privacy audit                               |
| `npm run evaluate:private-corpus` | Run the ignored local extraction evaluator                 |
| `npm run package:windows`         | Collect and verify existing Windows outputs                |
| `npm run tauri`                   | Invoke the Tauri command-line interface                    |

## Keep release metadata synchronized

A release version appears in:

- `package.json`
- `package-lock.json`
- `src-tauri/tauri.conf.json`
- `extension/manifest.json`
- Root Cargo workspace packages

`npm run verify:release-metadata` rejects divergence. It also derives the extension identifier from the manifest key and compares both native-host origin constants.

When a real Chrome Web Store identifier exists, set `CALLBACK_EXTENSION_ID` as a GitHub Actions repository or organization variable. The `release-candidate` job reads that variable directly and does not declare a GitHub Actions environment. Absence of the variable means Store identity remains unproven.

## Purge disposable local state

The GUI's **Purge local data** action closes the app and launches a helper. A source-run alternative is:

```powershell
cargo run -p callback-app -- --purge
```

A release executable can target an explicit database:

```powershell
.\target\release\callback-app.exe `
  --purge `
  --db "$env:APPDATA\com.callback.desktop\callback.db"
```

Purge removes desktop state and registrations. It does not clear the extension outbox; remove the extension to clear its browser-managed storage.

## Use Windows automation as the baseline

`.github/workflows/ci-windows.yml` runs locked setup, Rust checks, TypeScript checks, extension validation, production builds, metadata verification, the privacy audit, NSIS/MSI generation, artifact collection, and upload.

`.github/workflows/release-windows.yml` repeats candidate checks for manual dispatch or matching version tags. Only a matching tag, such as `v0.1.0`, passes verified files into the `windows-release` GitHub Actions environment and creates a draft GitHub release. The environment enforces reviewer or deployment protection only when repository settings configure those rules.

Neither workflow proves a human gate, signs binaries, publishes the draft, submits package-manager manifests, or certifies installed behavior. Follow [`release.md`](release.md) before publication.

## Add migrations without rewriting history

The current database schema version is `4`. Add a new numbered file under `src-tauri/migrations` and append it to `MIGRATIONS` in `src-tauri/src/db/mod.rs`.

Never edit a migration that existing databases may have applied. A migration must:

1. Run under the existing `BEGIN IMMEDIATE` wrapper
2. Preserve valid existing data or fail without advancing `user_version`
3. Add database-level checks for new state invariants
4. Include upgrade and rollback-path validation when tests are explicitly requested
5. Update [`current-state.md`](current-state.md) and [`architecture.md`](architecture.md)

## Preserve local-only changes

Before changing capture, transport, logging, storage, or dependencies:

1. Read [`privacy.md`](privacy.md)
2. Keep captured content out of logs and notification copy
3. Avoid remote APIs, telemetry, runtime selector fetches, and synchronized storage
4. Retain bounded local framing and outbox limits
5. Run `npm run audit:local-only`
6. Perform installed endpoint inspection before a qualified release
