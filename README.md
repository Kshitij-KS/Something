# Callback

Local-first reminders that fire on **context**, not clock time. Callback is a Windows-first Tauri app with a Chrome MV3 capture extension, a separate native-messaging host, and SQLite on disk.

Callback opens **no TCP/UDP listener** and sends **no captured message content over TCP/UDP or to a remote service**. Confirmed content moves locally from Chrome through native messaging and an owner-only named pipe into the desktop database. Chrome/site traffic and package-manager or Store downloads are outside this Callback-runtime guarantee.

## Run locally

```powershell
npm ci
npm run stage:native-host
cargo test --workspace --locked -j 1
npm run test -- --run
npm run build
npm run tauri dev
```

Load `extension/dist` as an unpacked Chrome extension after `npm run build:extension`.

`Ctrl+Shift+K` (configurable in Settings, fallback `Ctrl+Alt+K`) opens the existing `?window=quick` window. Callback never reads selected text from other apps. If Windows or another program already owns the shortcut, Health shows the failure or the fallback that was registered. Health also has an **Open quick capture** button.

Purge local SQLite and unregister the native host (also available from Health):

```powershell
cargo run -p callback-app -- --purge
# After a release build:
# .\target\release\callback-app.exe --purge --db "$env:APPDATA\com.callback.desktop\callback.db"
```

## Architecture

Chrome content script → service worker outbox → `callback-native-host.exe` (stdio) → current-user named pipe → Tauri core → SQLite → Windows toast.

The desktop process is single-instance. Installed `callback-action://` notification actions are forwarded to that process, and closing the main window keeps Callback available through its tray icon. Extracted/actionable toasts use generic copy rather than captured text; purge also clears Callback's Windows notification history. Actual cold/warm protocol activation and Action Center cleanup still require an installed-build Windows smoke test.

## Kill gates (pending you)

These cannot be closed in CI:

1. Phase 0: use hardcoded focus reminders for five days. Add a rule on the Phase 0 screen, focus that app for five seconds, and confirm a Windows toast. Cold-start toast actions, live lock/sleep on a physical session, and DND remain installed-build checks. Unit tests cover lock/sleep cancelling the five-second dwell generation.
2. Extraction: label 300 real sent messages; precision ≥70% before Phase 2 and ≥80% before release. Keep the corpus private and run `npm run evaluate:private-corpus` only with `CALLBACK_PRIVATE_CORPUS` set locally.
3. Acceptance: use the closed loop daily for two weeks; require ≥40% actionable acceptance.

Autostart is disclosed and toggled in Settings through the current-user Run key. Confirming that Windows launches the installed build at logon still needs a human session. See `docs/kill-gates.md` for evidence rules.

## Windows release candidates

Windows CI and `.github/workflows/release-windows.yml` request both NSIS and MSI. Packaging fails unless exactly one installer of each format exists, validates a root-correct extension ZIP, and emits an immediately re-verified `artifacts/SHA256SUMS.txt`. The manifest key-derived extension ID must match both native-host origins; a real Chrome Web Store ID can also be enforced through `CALLBACK_EXTENSION_ID` once one exists.

The generated NSIS/MSI installers and extension ZIP are **unsigned**. SHA-256 checksums verify downloaded bytes but are not publisher signatures. Windows SmartScreen and Mark-of-the-Web warnings are expected until Authenticode signing is configured. The tag workflow hands verified artifacts to a separate `windows-release` publication environment that runs no project code and creates a draft GitHub release only; configure that environment with required reviewers. It does not claim Chrome Web Store, winget, or Scoop publication. No package-manager manifests are generated before immutable release URLs and real hashes exist.

Separate NSIS/MSI install/uninstall tests and the interactive toast, focus, DND, lock/sleep, autostart, native-host, and runtime endpoint matrix remain physical/VM checks. Do not treat a successful source build as installed-device certification.

## License

MIT
