# Kill gates

Human-time gates. Status is also stored in SQLite table `kill_gates`.

| ID                         | Requirement                                                                                                          | Status       |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------- | ------------ |
| `phase0_five_day`          | Use Phase 0 for five days. Stop if context reminders are not materially better than time reminders.                  | pending_user |
| `extraction_precision_300` | Label 300 real sent messages. ≥70% precision before Phase 2, ≥80% before release. Keep the corpus private and local. | pending_user |
| `acceptance_two_week`      | Use the closed loop daily for two weeks. ≥40% actionable acceptance before launch work.                              | pending_user |

Installed-build checks that also need a human session: toast actions when the app is stopped, live lock/sleep on a physical session, DND/Focus Assist, autostart, and package-manager install/uninstall. CI can emit **unsigned** NSIS/MSI; SmartScreen warnings are expected until Authenticode signing exists.
