# Kill gates

Human-time gates. Status is also stored in SQLite table `kill_gates`.

| ID                         | Requirement                                                                                                          | Status       |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------- | ------------ |
| `phase0_five_day`          | Use Phase 0 for five days. Stop if context reminders are not materially better than time reminders.                  | pending_user |
| `extraction_precision_300` | Label 300 real sent messages. ≥70% precision before Phase 2, ≥80% before release. Keep the corpus private and local. | pending_user |
| `acceptance_two_week`      | Use the closed loop daily for two weeks. ≥40% actionable acceptance before launch work.                              | pending_user |

## Private extraction corpus

Keep the real corpus outside version control. Each nonblank JSONL row must contain a unique nonempty `id`, `source` (`gmail` or `slack`), nonempty `text`, and `label` (`promise` or `not_promise`), matching `src-tauri/tests/fixtures/messages.jsonl.example`. Classification is message-level: any Capture clause wins, then Review, then Discard. The gate metric is automatic-capture precision, `Capture promise / all Capture`; Capture-plus-Review candidate precision and automatic recall are secondary diagnostics. Evaluation uses the shipped baseline with an empty learned blocklist.

On Windows PowerShell, run the local evaluator explicitly:

```powershell
$env:CALLBACK_PRIVATE_CORPUS = (Resolve-Path '.\src-tauri\tests\fixtures\messages.jsonl').Path
npm run evaluate:private-corpus
Remove-Item Env:CALLBACK_PRIVATE_CORPUS
```

The command emits aggregate counts and metrics only. It fails on malformed/invalid/duplicate rows, fewer than 300 valid unique messages, undefined automatic precision, or precision below the 70% Phase 2 gate; 80% is reported separately as the release target. It does not print corpus paths, IDs, message text, clauses, or per-message results, access the network, mutate the app database, or record a gate decision. After `phase0_five_day` has passed, a human can record the aggregate evidence for `extraction_precision_300` locally. Never add the corpus or evaluator environment variable to CI, artifacts, caches, or repository secrets.

Installed-build checks that also need a human session: toast actions when the app is stopped, live lock/sleep on a physical session, DND/Focus Assist, autostart, and package-manager install/uninstall. CI can emit **unsigned** NSIS/MSI; SmartScreen warnings are expected until Authenticode signing exists.
