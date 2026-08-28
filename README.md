# Callback

Local-first reminders that fire on **context**, not clock time. Windows-first Tauri app, Chrome MV3 capture extension, separate native-messaging host, SQLite on disk.

The runtime opens **no TCP/UDP listener** and does **not transmit message content**. Chrome Store and package-manager traffic is outside that guarantee.

## Run locally

```bash
npm install
cargo test
npm run test
npm run build
npm run tauri dev
```

Load `extension/dist` as an unpacked Chrome extension after `npm run build:extension`.

`Ctrl+Shift+K` (configurable in Settings, fallback `Ctrl+Alt+K`) opens the existing `?window=quick` window. Callback never reads selected text from other apps. If Windows or another program already owns the shortcut, Health shows the failure or the fallback that was registered. Health also has an **Open quick capture** button.

Purge local SQLite and unregister the native host (also available from Health):

```bash
cargo run -p callback-app -- --purge
# or, after a build:
# Callback.exe --purge --db "%APPDATA%\com.callback.desktop\callback.db"
```

## Architecture

Chrome content script → service worker outbox → `callback-native-host.exe` (stdio) → current-user named pipe → Tauri core → SQLite → Windows toast.

## Kill gates (pending you)

These cannot be closed in CI:

1. Phase 0: use hardcoded focus reminders for five days. Add a rule on the Phase 0 screen, focus that app for five seconds, and confirm a Windows toast. Cold-start toast actions, live lock/sleep on a physical session, and DND remain installed-build checks. Unit tests cover lock/sleep cancelling the five-second dwell generation.
2. Extraction: label 300 real sent messages; precision ≥ 70% before Phase 2, ≥ 80% before release.
3. Acceptance: use the closed loop daily for two weeks; ≥ 40% actionable.

Autostart is disclosed and toggled in Settings (current-user Run key). Confirming that Windows actually launches the signed/unsigned installed build at logon still needs a human session.

See `docs/kill-gates.md`.

## Unsigned binaries

CI (`ci-windows.yml`) runs `npx tauri build --bundles nsis,msi` **without a code-signing certificate**. That can produce NSIS and/or MSI packages plus the raw `callback.exe` / `callback-native-host.exe` artifacts. Those files are **unsigned**.

Windows SmartScreen and Mark-of-the-Web will warn. winget/Scoop do not eliminate that risk. A signed release needs a purchased/org Authenticode certificate that this workflow does not have. Do not treat CI installer artifacts as a signed distribution.

## License

MIT
