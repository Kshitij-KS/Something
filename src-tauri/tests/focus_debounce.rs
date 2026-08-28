use std::time::{Duration, Instant};

use callback_lib::platform::focus::{
    BrowserContext, DebounceOutcome, FocusDebouncer, FocusEvent, FocusTarget, OsFocus,
    combine_focus,
};
use callback_lib::surfacing::phase0::{Phase0Rule, match_phase0};

fn target(app: &str) -> FocusTarget {
    FocusTarget {
        app_id: app.to_owned(),
        context: None,
    }
}

#[test]
fn rapid_alt_tab_cancels_the_first_dwell() {
    let mut debounce = FocusDebouncer::new(Duration::from_secs(5));
    let t0 = Instant::now();
    assert!(
        debounce
            .on_os_focus(Some(target("slack.exe")), t0)
            .is_pending()
    );
    let t1 = t0 + Duration::from_millis(400);
    assert!(
        debounce
            .on_os_focus(Some(target("code.exe")), t1)
            .is_pending()
    );
    let early = debounce.on_tick(t0 + Duration::from_secs(5));
    assert!(early.fired_app().is_none());
    assert!(early.is_pending());
    let fired = debounce.on_tick(t1 + Duration::from_secs(5));
    assert_eq!(fired.fired_app(), Some("code.exe"));
}

#[test]
fn same_app_refocus_does_not_restart_dwell() {
    let mut debounce = FocusDebouncer::new(Duration::from_secs(5));
    let t0 = Instant::now();
    debounce.on_os_focus(Some(target("slack.exe")), t0);
    debounce.on_os_focus(Some(target("slack.exe")), t0 + Duration::from_secs(1));
    let fired = debounce.on_tick(t0 + Duration::from_secs(5));
    assert_eq!(fired.fired_app(), Some("slack.exe"));
}

#[test]
fn protected_process_lookup_failure_skips_without_firing() {
    let mut debounce = FocusDebouncer::new(Duration::from_secs(5));
    let t0 = Instant::now();
    debounce.on_os_focus(Some(target("slack.exe")), t0);
    assert!(
        debounce
            .on_os_focus(None, t0 + Duration::from_millis(10))
            .is_cancelled()
    );
    assert!(
        debounce
            .on_tick(t0 + Duration::from_secs(5))
            .was_cancelled_or_idle()
    );
}

#[test]
fn lock_or_sleep_cancels_stale_timer() {
    let mut debounce = FocusDebouncer::new(Duration::from_secs(5));
    let t0 = Instant::now();
    debounce.on_os_focus(Some(target("slack.exe")), t0);
    assert!(debounce.on_lock_or_sleep().is_cancelled());
    assert!(
        debounce
            .on_tick(t0 + Duration::from_secs(5))
            .was_cancelled_or_idle()
    );
}

#[test]
fn lock_bumps_generation_so_stale_scheduled_fire_is_cancelled() {
    let mut debounce = FocusDebouncer::new(Duration::from_secs(5));
    let t0 = Instant::now();
    debounce.on_os_focus(Some(target("slack.exe")), t0);
    let generation = debounce.generation();
    assert_eq!(
        debounce.fire_scheduled(generation, t0 + Duration::from_secs(5)),
        DebounceOutcome::Fired(target("slack.exe"))
    );

    let mut debounce = FocusDebouncer::new(Duration::from_secs(5));
    debounce.on_os_focus(Some(target("slack.exe")), t0);
    let stale = debounce.generation();
    debounce.apply_focus_event(&FocusEvent::SessionLock, t0 + Duration::from_secs(1));
    assert_ne!(debounce.generation(), stale);
    assert_eq!(
        debounce.fire_scheduled(stale, t0 + Duration::from_secs(5)),
        DebounceOutcome::Cancelled
    );
    assert!(
        debounce
            .on_tick(t0 + Duration::from_secs(5))
            .was_cancelled_or_idle()
    );
}

#[test]
fn unlock_and_resume_restart_dwell_instead_of_firing_immediately() {
    let mut debounce = FocusDebouncer::new(Duration::from_secs(5));
    let t0 = Instant::now();
    debounce.on_os_focus(Some(target("slack.exe")), t0);
    debounce.apply_focus_event(&FocusEvent::SessionLock, t0 + Duration::from_secs(1));
    debounce.apply_focus_event(&FocusEvent::SessionUnlock, t0 + Duration::from_secs(2));
    debounce.on_os_focus(Some(target("slack.exe")), t0 + Duration::from_secs(2));
    assert!(
        debounce.on_tick(t0 + Duration::from_secs(5)).is_pending(),
        "dwell must restart after unlock, not inherit the pre-lock clock"
    );
    assert_eq!(
        debounce.on_tick(t0 + Duration::from_secs(7)).fired_app(),
        Some("slack.exe")
    );

    let mut sleep = FocusDebouncer::new(Duration::from_secs(5));
    sleep.on_os_focus(Some(target("code.exe")), t0);
    sleep.apply_focus_event(&FocusEvent::Sleep, t0 + Duration::from_millis(10));
    sleep.apply_focus_event(&FocusEvent::Resume, t0 + Duration::from_secs(1));
    sleep.on_os_focus(Some(target("code.exe")), t0 + Duration::from_secs(1));
    assert!(sleep.on_tick(t0 + Duration::from_secs(5)).is_pending());
    assert_eq!(
        sleep.on_tick(t0 + Duration::from_secs(6)).fired_app(),
        Some("code.exe")
    );
}

#[test]
fn chrome_without_matching_tab_is_not_web_context() {
    let os = OsFocus {
        exe_name: "chrome.exe".into(),
    };
    let none = combine_focus(Some(&os), None);
    assert_eq!(none.app_id, "chrome.exe");
    assert_eq!(none.context, None);

    let browser = BrowserContext {
        source_app: "slack".into(),
        source_ctx: Some("D0123".into()),
        visible: true,
        active: true,
    };
    let combined = combine_focus(Some(&os), Some(&browser));
    assert_eq!(combined.app_id, "chrome.exe");
    assert_eq!(combined.context.as_deref(), Some("slack:D0123"));
}

#[test]
fn stale_context_heartbeat_is_ignored() {
    let os = OsFocus {
        exe_name: "chrome.exe".into(),
    };
    let stale = BrowserContext {
        source_app: "gmail".into(),
        source_ctx: Some("thread-1".into()),
        visible: false,
        active: false,
    };
    let combined = combine_focus(Some(&os), Some(&stale));
    assert_eq!(combined.context, None);
}

#[test]
fn extension_context_payload_parses_snake_and_camel_case() {
    let snake = serde_json::json!({
        "source_app": "gmail",
        "source_ctx": "thread-1",
        "visible": true,
        "active": true
    });
    let parsed = callback_lib::platform::focus::parse_browser_context(&snake).expect("snake");
    assert_eq!(parsed.source_app, "gmail");
    assert_eq!(parsed.source_ctx.as_deref(), Some("thread-1"));
    let camel = serde_json::json!({
        "sourceApp": "slack",
        "sourceCtx": "D0123",
        "visible": true,
        "active": true
    });
    let parsed = callback_lib::platform::focus::parse_browser_context(&camel).expect("camel");
    assert_eq!(parsed.source_app, "slack");
    assert_eq!(parsed.source_ctx.as_deref(), Some("D0123"));
}

#[test]
fn phase0_rule_matches_exe_basename_case_insensitively() {
    let rules = [Phase0Rule {
        id: 1,
        app_match: "Slack.exe".into(),
        reminder_text: "Follow up with Priya".into(),
        enabled: true,
    }];
    let matched = match_phase0(r"C:\Program Files\Slack\slack.exe", &rules).expect("match");
    assert_eq!(matched.reminder_text, "Follow up with Priya");
}
