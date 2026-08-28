use callback_lib::shortcut::{ShortcutOutcome, ShortcutPlan, choose_registration, open_quick_spec};

#[test]
fn quick_capture_window_uses_existing_query_and_does_not_read_selection() {
    let spec = open_quick_spec();
    assert_eq!(spec.label, "quick");
    assert!(
        spec.url.contains("window=quick"),
        "must reuse the existing ?window=quick surface, got {}",
        spec.url
    );
    assert!(
        !spec.reads_selected_text,
        "must not silently read selected text from other apps"
    );
    assert!(
        !spec.auto_prefill_clipboard,
        "clipboard must not prefill without an explicit preview/confirm"
    );
}

#[test]
fn default_primary_shortcut_is_ctrl_shift_k() {
    let plan = ShortcutPlan::from_settings(None, None);
    assert_eq!(plan.primary, "Ctrl+Shift+K");
    assert_eq!(plan.fallback, "Ctrl+Alt+K");
}

#[test]
fn settings_override_primary_and_fallback() {
    let plan = ShortcutPlan::from_settings(Some("Ctrl+Shift+Q"), Some("Ctrl+Alt+Q"));
    assert_eq!(plan.primary, "Ctrl+Shift+Q");
    assert_eq!(plan.fallback, "Ctrl+Alt+Q");
}

#[test]
fn registration_uses_fallback_when_primary_collides() {
    let plan = ShortcutPlan::from_settings(None, None);
    let outcome = choose_registration(&plan, |accel| {
        if accel == "Ctrl+Shift+K" {
            Err("already registered".into())
        } else {
            Ok(())
        }
    });
    assert_eq!(
        outcome,
        ShortcutOutcome::RegisteredFallback {
            accelerator: "Ctrl+Alt+K".into(),
            reason: "already registered".into(),
        }
    );
}

#[test]
fn registration_failure_is_reported_not_swallowed() {
    let plan = ShortcutPlan::from_settings(None, None);
    let outcome = choose_registration(&plan, |_| Err("blocked by OS".into()));
    assert_eq!(
        outcome,
        ShortcutOutcome::Failed {
            primary: "Ctrl+Shift+K".into(),
            fallback: "Ctrl+Alt+K".into(),
            reason: "blocked by OS".into(),
        }
    );
    assert!(!outcome.is_registered());
}

#[test]
fn successful_primary_registration_does_not_touch_fallback() {
    let plan = ShortcutPlan::from_settings(None, None);
    let mut seen = Vec::new();
    let outcome = choose_registration(&plan, |accel| {
        seen.push(accel.to_owned());
        Ok(())
    });
    assert_eq!(
        outcome,
        ShortcutOutcome::RegisteredPrimary {
            accelerator: "Ctrl+Shift+K".into(),
        }
    );
    assert_eq!(seen, ["Ctrl+Shift+K"]);
    assert!(outcome.is_registered());
}
