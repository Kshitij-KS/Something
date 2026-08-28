use callback_lib::db::Database;
use callback_lib::domain::{CaptureRecord, LeaseState, PromiseStatus, SurfaceLease};
use callback_lib::platform::focus::{BrowserContext, FocusTarget, OsFocus, combine_focus};
use callback_lib::platform::notifications::RecordingSink;
use callback_lib::surfacing::engine::{DwellAction, handle_dwell};
use callback_lib::surfacing::phase0::Phase0Rule;
use chrono::{Duration, TimeZone, Utc};
use tempfile::tempdir;

fn open_temp_db() -> (tempfile::TempDir, Database) {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let database = Database::open(&path).expect("open database");
    (dir, database)
}

fn seed_open_slack_dm(database: &Database, capture_id: &str, text: &str) -> i64 {
    let mut capture = CaptureRecord::fixture(capture_id, 0);
    text.clone_into(&mut capture.raw_message);
    database.insert_capture(&capture).expect("capture");
    let promise_id = database
        .insert_promise_from_capture(capture_id, 0, text, 8, 0.8)
        .expect("promise");
    database
        .set_promise_status(promise_id, PromiseStatus::Open, 1_700_000_000)
        .expect("open");
    database
        .insert_trigger(promise_id, "app_ctx_focus", "slack:D0123", 100)
        .expect("ctx trigger");
    database
        .insert_trigger(promise_id, "app_focus", "slack", 10)
        .expect("app trigger");
    promise_id
}

fn chrome_slack_dm() -> FocusTarget {
    let os = OsFocus {
        exe_name: "chrome.exe".into(),
    };
    let browser = BrowserContext {
        source_app: "slack".into(),
        source_ctx: Some("D0123".into()),
        visible: true,
        active: true,
    };
    combine_focus(Some(&os), Some(&browser))
}

#[test]
fn dwell_surfaces_one_extracted_promise_with_crash_safe_token() {
    let (_dir, database) = open_temp_db();
    let promise_id = seed_open_slack_dm(&database, "cap-surface", "I will send the invoice");
    let sink = RecordingSink::default();
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");

    let action = handle_dwell(&database, &sink, &chrome_slack_dm(), now, &[]).expect("dwell");
    match action {
        DwellAction::ExtractedShown {
            promise_id: shown_id,
            action_token,
        } => {
            assert_eq!(shown_id, promise_id);
            assert!(!action_token.is_empty());
            assert!(!action_token.starts_with("phase0:"));
        }
        other => panic!("expected extracted surface, got {other:?}"),
    }

    let shown = sink.shown();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title, "Callback");
    assert!(shown[0].body.contains("invoice"));
    let lease = database
        .lease_by_token_action(&shown[0].action_token)
        .expect("lookup")
        .expect("lease");
    assert_eq!(lease.promise_id, promise_id);
    assert_eq!(lease.action_token, shown[0].action_token);
    assert_eq!(lease.state, LeaseState::Shown);
}

#[test]
fn dwell_combines_os_and_extension_and_ignores_chrome_without_tab() {
    let (_dir, database) = open_temp_db();
    seed_open_slack_dm(&database, "cap-no-tab", "I will send the invoice");
    let sink = RecordingSink::default();
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    let chrome_only = combine_focus(
        Some(&OsFocus {
            exe_name: "chrome.exe".into(),
        }),
        None,
    );
    let action = handle_dwell(&database, &sink, &chrome_only, now, &[]).expect("dwell");
    assert_eq!(action, DwellAction::None);
    assert!(sink.shown().is_empty());
}

#[test]
fn dwell_does_not_burst_when_a_lease_is_already_active() {
    let (_dir, database) = open_temp_db();
    let promise_id = seed_open_slack_dm(&database, "cap-active", "I will send the invoice");
    let mut lease = SurfaceLease::new(promise_id, "lease-live", "action-live");
    lease.state = LeaseState::Shown;
    lease.expires_at = 2_000_000_000;
    database.insert_lease(lease).expect("lease");
    let sink = RecordingSink::default();
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    let action = handle_dwell(&database, &sink, &chrome_slack_dm(), now, &[]).expect("dwell");
    assert_eq!(action, DwellAction::Suppressed);
    assert!(sink.shown().is_empty());
}

#[test]
fn competing_matches_select_one_candidate() {
    let (_dir, database) = open_temp_db();
    seed_open_slack_dm(&database, "cap-a", "I will send the older invoice");
    let later = seed_open_slack_dm(
        &database,
        "cap-b",
        "I will send the urgent invoice tomorrow",
    );
    database
        .insert_extracted_promise(
            "cap-b",
            0,
            "I will send the urgent invoice tomorrow",
            8,
            0.8,
            Some((1_700_000_100, "UTC".into(), "day".into())),
        )
        .ok();
    let _ = later;
    let sink = RecordingSink::default();
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    let action = handle_dwell(&database, &sink, &chrome_slack_dm(), now, &[]).expect("dwell");
    assert!(matches!(action, DwellAction::ExtractedShown { .. }));
    assert_eq!(sink.shown().len(), 1);
}

#[test]
fn dwell_falls_back_to_phase0_when_no_extracted_match() {
    let (_dir, database) = open_temp_db();
    let sink = RecordingSink::default();
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    let rules = [Phase0Rule {
        id: 9,
        app_match: "slack.exe".into(),
        reminder_text: "Follow up with Priya".into(),
        enabled: true,
    }];
    let target = FocusTarget {
        app_id: r"C:\Program Files\Slack\slack.exe".into(),
        context: None,
    };
    let action = handle_dwell(&database, &sink, &target, now, &rules).expect("dwell");
    assert_eq!(action, DwellAction::Phase0Shown { rule_id: 9 });
    assert_eq!(sink.shown()[0].body, "Follow up with Priya");
    assert_eq!(sink.shown()[0].action_token, "phase0:9");
}

#[test]
fn extracted_surface_wins_over_phase0_and_stays_single() {
    let (_dir, database) = open_temp_db();
    seed_open_slack_dm(&database, "cap-win", "I will send the invoice");
    let sink = RecordingSink::default();
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    let rules = [Phase0Rule {
        id: 1,
        app_match: "chrome.exe".into(),
        reminder_text: "Phase 0 should not fire".into(),
        enabled: true,
    }];
    let action = handle_dwell(&database, &sink, &chrome_slack_dm(), now, &rules).expect("dwell");
    assert!(matches!(action, DwellAction::ExtractedShown { .. }));
    assert_eq!(sink.shown().len(), 1);
    assert_ne!(sink.shown()[0].body, "Phase 0 should not fire");
}

#[test]
fn rate_limit_gap_suppresses_a_second_dwell() {
    let (_dir, database) = open_temp_db();
    seed_open_slack_dm(&database, "cap-gap-1", "I will send the invoice");
    seed_open_slack_dm(&database, "cap-gap-2", "I will ping Priya");
    let sink = RecordingSink::default();
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    assert!(matches!(
        handle_dwell(&database, &sink, &chrome_slack_dm(), now, &[]).expect("first"),
        DwellAction::ExtractedShown { .. }
    ));
    let later = now + Duration::minutes(10);
    let second = handle_dwell(&database, &sink, &chrome_slack_dm(), later, &[]).expect("second");
    assert_eq!(second, DwellAction::Suppressed);
    assert_eq!(sink.shown().len(), 1);
}
