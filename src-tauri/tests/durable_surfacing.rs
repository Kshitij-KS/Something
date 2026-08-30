use callback_lib::db::Database;
use callback_lib::domain::{CaptureRecord, PromiseStatus};
use callback_lib::platform::focus::FocusTarget;
use callback_lib::platform::notifications::{
    ACTIONABLE_REMINDER_BODY, NotificationRequest, NotificationSink, NotifyError, RecordingSink,
    toast_xml,
};
use callback_lib::surfacing::actions::{
    ActionActivation, ActionResult, ParseActionError, SurfaceAction, dispatch_activation,
    parse_action_argument, parse_cold_start_args,
};
use callback_lib::surfacing::engine::{
    DwellAction, SurfaceError, handle_dwell, handle_maintenance_tick,
};
use chrono::{TimeZone, Utc};
use tempfile::tempdir;

fn open_temp_db() -> (tempfile::TempDir, Database) {
    let dir = tempdir().expect("temp dir");
    let database = Database::open(&dir.path().join("callback.db")).expect("open database");
    database
        .update_kill_gate(
            "phase0_five_day",
            "passed",
            "Five-day local Phase 0 trial passed for this test fixture.",
        )
        .expect("phase 0 gate");
    database
        .update_kill_gate(
            "extraction_precision_300",
            "passed",
            "Three-hundred-message precision threshold passed for this test fixture.",
        )
        .expect("extraction gate");
    database
        .upsert_setting("timezone", "UTC")
        .expect("timezone");
    (dir, database)
}

fn seed_open_promise(database: &Database, capture_id: &str, deadline: Option<i64>) -> i64 {
    let mut capture = CaptureRecord::fixture(capture_id, 0);
    capture.raw_message = "I will send the invoice".into();
    database.insert_capture(&capture).expect("capture");
    let promise_id = database
        .insert_promise_from_capture(capture_id, 0, "I will send the invoice", 8, 0.8)
        .expect("promise");
    database
        .set_promise_status(promise_id, PromiseStatus::Open, 1_700_000_000)
        .expect("open");
    database
        .insert_trigger(promise_id, "app_ctx_focus", "slack:D0123", 100)
        .expect("trigger");
    if let Some(deadline) = deadline {
        database
            .insert_extracted_promise(
                capture_id,
                0,
                "I will send the invoice",
                8,
                0.8,
                Some((deadline, "UTC".into(), "minute".into())),
            )
            .expect("deadline");
    }
    promise_id
}

fn target() -> FocusTarget {
    FocusTarget {
        app_id: "chrome.exe".into(),
        context: Some("slack:D0123".into()),
    }
}

fn table_count(database: &Database, table: &str) -> i64 {
    database
        .read_connection()
        .expect("reader")
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count")
}

#[derive(Debug, Default)]
struct FailingSink;

impl NotificationSink for FailingSink {
    fn show(&self, _request: &NotificationRequest) -> Result<(), NotifyError> {
        Err(NotifyError::Delivery("simulated sink failure".into()))
    }
}

#[test]
fn notification_delivery_records_success_or_failure_and_releases_failed_lease() {
    let (_dir, database) = open_temp_db();
    seed_open_promise(&database, "cap-delivery-success", None);
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    let sink = RecordingSink::default();
    assert!(matches!(
        handle_dwell(&database, &sink, &target(), now, &[]).expect("surface"),
        DwellAction::ExtractedShown { .. }
    ));
    let success: (String, i64, Option<String>) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT s.state, n.delivered, n.error
             FROM surface_attempts s
             JOIN notification_attempts n ON n.surface_attempt_id = s.id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("successful attempt");
    assert_eq!(success, ("shown".into(), 1, None));

    let (_dir, failed_database) = open_temp_db();
    seed_open_promise(&failed_database, "cap-delivery-failure", None);
    let error = handle_dwell(&failed_database, &FailingSink, &target(), now, &[])
        .expect_err("sink failure");
    assert!(matches!(error, SurfaceError::Notification(_)));
    let failure: (String, i64, String, i64) = failed_database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT s.state, n.delivered, n.error, s.expires_at
             FROM surface_attempts s
             JOIN notification_attempts n ON n.surface_attempt_id = s.id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("failed attempt");
    assert_eq!(failure.0, "expired");
    assert_eq!(failure.1, 0);
    assert!(failure.2.contains("simulated sink failure"));
    assert_eq!(failure.3, now.timestamp());
    assert!(
        !failed_database
            .has_active_surface(now.timestamp())
            .expect("active surface")
    );
}

#[test]
fn done_and_reject_actions_are_atomic_and_duplicate_safe() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");

    let (_dir, done_database) = open_temp_db();
    let done_promise = seed_open_promise(&done_database, "cap-done", None);
    let done_sink = RecordingSink::default();
    handle_dwell(&done_database, &done_sink, &target(), now, &[]).expect("surface done");
    let done_token = done_sink.shown()[0]
        .action_token
        .clone()
        .expect("action token");
    let activation = ActionActivation {
        action: SurfaceAction::Done,
        action_token: done_token,
    };
    assert!(matches!(
        dispatch_activation(
            &done_database,
            &activation,
            now.timestamp() + 1,
            "2026-08-27"
        )
        .expect("done action"),
        ActionResult::Applied {
            next: PromiseStatus::Done,
            ..
        }
    ));
    assert_eq!(
        dispatch_activation(
            &done_database,
            &activation,
            now.timestamp() + 2,
            "2026-08-27"
        )
        .expect("duplicate done"),
        ActionResult::Duplicate
    );
    let done_state: (String, String, i64) = done_database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT p.status, s.action, COUNT(*) OVER ()
             FROM promises p JOIN surface_attempts s ON s.promise_id = p.id
             WHERE p.id = ?1",
            [done_promise],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("done state");
    assert_eq!(done_state, ("done".into(), "done".into(), 1));

    let (_dir, reject_database) = open_temp_db();
    let reject_promise = seed_open_promise(&reject_database, "cap-reject", None);
    let reject_sink = RecordingSink::default();
    handle_dwell(&reject_database, &reject_sink, &target(), now, &[]).expect("surface reject");
    let reject_activation = ActionActivation {
        action: SurfaceAction::Reject,
        action_token: reject_sink.shown()[0]
            .action_token
            .clone()
            .expect("action token"),
    };
    dispatch_activation(
        &reject_database,
        &reject_activation,
        now.timestamp() + 1,
        "2026-08-27",
    )
    .expect("reject");
    dispatch_activation(
        &reject_database,
        &reject_activation,
        now.timestamp() + 2,
        "2026-08-27",
    )
    .expect("duplicate reject");
    let rejected: (String, i64) = reject_database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT p.status, (SELECT SUM(hits) FROM blocklist)
             FROM promises p WHERE p.id = ?1",
            [reject_promise],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("rejected state");
    assert_eq!(rejected, ("dismissed".into(), 1));
}

#[test]
fn snooze_reopens_on_timer_but_waits_for_a_new_dwell() {
    let (_dir, database) = open_temp_db();
    let promise_id = seed_open_promise(&database, "cap-snooze", None);
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    let sink = RecordingSink::default();
    handle_dwell(&database, &sink, &target(), now, &[]).expect("surface");
    let activation = ActionActivation {
        action: SurfaceAction::Snooze,
        action_token: sink.shown()[0].action_token.clone().expect("action token"),
    };
    dispatch_activation(&database, &activation, now.timestamp() + 1, "2026-08-27").expect("snooze");
    let snooze_until: i64 = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT snooze_until FROM promises WHERE id = ?1 AND status = 'snoozed'",
            [promise_id],
            |row| row.get(0),
        )
        .expect("snooze timestamp");

    let timer_sink = RecordingSink::default();
    let maintenance = handle_maintenance_tick(
        &database,
        &timer_sink,
        Utc.timestamp_opt(snooze_until, 0)
            .single()
            .expect("due time"),
    )
    .expect("maintenance");
    assert_eq!(maintenance.reopened_snoozes, 1);
    assert_eq!(maintenance.deadline_surface, None);
    assert!(timer_sink.shown().is_empty());
    assert!(
        database
            .list_surfaceable_rows(snooze_until)
            .expect("surfaceable rows")
            .is_empty()
    );

    let next_day = Utc
        .with_ymd_and_hms(2026, 8, 28, 12, 0, 0)
        .single()
        .expect("next day");
    let fresh_sink = RecordingSink::default();
    assert!(matches!(
        handle_dwell(&database, &fresh_sink, &target(), next_day, &[]).expect("fresh dwell"),
        DwellAction::ExtractedShown { .. }
    ));
    assert_eq!(fresh_sink.shown().len(), 1);
}

#[test]
fn third_committed_ignore_archives_and_each_token_is_single_use() {
    let (_dir, database) = open_temp_db();
    let promise_id = seed_open_promise(&database, "cap-ignore", None);
    let base = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("base")
        .timestamp();

    for index in 0..3 {
        let shown_at = base + i64::from(index) * 86_400;
        let lease_token = uuid::Uuid::new_v4().to_string();
        let action_token = uuid::Uuid::new_v4().to_string();
        let attempt = database
            .begin_notification_attempt(
                promise_id,
                &lease_token,
                &action_token,
                shown_at,
                shown_at + 900,
            )
            .expect("begin attempt");
        database
            .finish_notification_delivered(
                attempt,
                shown_at,
                &format!("2026-08-{}", 27 + index),
                false,
            )
            .expect("finish attempt");
        let activation = ActionActivation {
            action: SurfaceAction::Ignore,
            action_token,
        };
        let result = dispatch_activation(
            &database,
            &activation,
            shown_at + 1,
            &format!("2026-08-{}", 27 + index),
        )
        .expect("ignore");
        assert!(matches!(result, ActionResult::Applied { .. }));
        assert_eq!(
            dispatch_activation(
                &database,
                &activation,
                shown_at + 2,
                &format!("2026-08-{}", 27 + index),
            )
            .expect("duplicate ignore"),
            ActionResult::Duplicate
        );
    }

    let stored: (String, i64, i64) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT status, ignore_count,
                    (SELECT COUNT(*) FROM surface_attempts WHERE promise_id = ?1 AND action = 'ignored')
             FROM promises WHERE id = ?1",
            [promise_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("archived promise");
    assert_eq!(stored, ("archived".into(), 3, 3));
}

#[test]
fn deadline_escalation_is_durable_once_and_failures_remain_retryable() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    let (_dir, database) = open_temp_db();
    let promise_id = seed_open_promise(&database, "cap-deadline", Some(now.timestamp() - 1));
    let sink = RecordingSink::default();
    let first = handle_maintenance_tick(&database, &sink, now).expect("deadline maintenance");
    assert!(matches!(
        first.deadline_surface,
        Some(DwellAction::ExtractedShown { .. })
    ));
    assert_eq!(sink.shown().len(), 1);
    assert_eq!(sink.shown()[0].body, ACTIONABLE_REMINDER_BODY);
    assert!(!toast_xml(&sink.shown()[0]).contains("invoice"));
    let escalated_at: Option<i64> = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT deadline_escalated_at FROM promises WHERE id = ?1",
            [promise_id],
            |row| row.get(0),
        )
        .expect("escalation stamp");
    assert_eq!(escalated_at, Some(now.timestamp()));
    let second = handle_maintenance_tick(&database, &sink, now).expect("second maintenance");
    assert_eq!(second.deadline_surface, None);
    assert_eq!(table_count(&database, "notification_attempts"), 1);

    let (_dir, failed_database) = open_temp_db();
    let failed_promise = seed_open_promise(
        &failed_database,
        "cap-deadline-failed",
        Some(now.timestamp() - 1),
    );
    assert!(matches!(
        handle_maintenance_tick(&failed_database, &FailingSink, now),
        Err(SurfaceError::Notification(_))
    ));
    let failed_escalation: Option<i64> = failed_database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT deadline_escalated_at FROM promises WHERE id = ?1",
            [failed_promise],
            |row| row.get(0),
        )
        .expect("failed escalation stamp");
    assert_eq!(failed_escalation, None);
    assert!(
        failed_database
            .list_due_deadline_candidates(now.timestamp())
            .expect("retryable deadline")
            .iter()
            .any(|candidate| candidate.promise_id == failed_promise)
    );
}

#[test]
fn action_parser_and_toast_xml_are_strict_and_content_safe() {
    let token = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
    for (verb, action) in [
        ("done", SurfaceAction::Done),
        ("snooze", SurfaceAction::Snooze),
        ("reject", SurfaceAction::Reject),
        ("ignore", SurfaceAction::Ignore),
    ] {
        assert_eq!(
            parse_action_argument(&format!("callback-action://{verb}/{token}"))
                .expect("parse action"),
            Some(ActionActivation {
                action,
                action_token: token.into(),
            })
        );
    }
    assert_eq!(parse_action_argument("--purge").expect("unrelated"), None);
    assert_eq!(
        parse_action_argument("callback-action:done:bad"),
        Err(ParseActionError::Malformed)
    );
    assert!(matches!(
        parse_action_argument("callback-action://done/not-a-uuid"),
        Err(ParseActionError::InvalidToken)
    ));
    assert_eq!(
        parse_cold_start_args([
            "callback.exe",
            &format!("callback-action://done/{token}"),
            &format!("callback-action://ignore/{token}"),
        ]),
        Err(ParseActionError::Ambiguous)
    );

    let escaped = toast_xml(&NotificationRequest::informational(
        "Callback <local>",
        "A&B",
    ));
    assert!(escaped.contains("Callback &lt;local&gt;"));
    assert!(escaped.contains("A&amp;B"));
    assert!(!escaped.contains("<actions>"));

    let actionable = NotificationRequest::actionable(token);
    let xml = toast_xml(&actionable);
    assert!(xml.contains(ACTIONABLE_REMINDER_BODY));
    assert_eq!(xml.matches("activationType=\"protocol\"").count(), 4);
    assert!(xml.contains("callback-action://ignore/"));
    let informational = toast_xml(&NotificationRequest::informational("Callback", "Phase 0"));
    assert!(!informational.contains("<actions>"));
    assert!(!informational.contains("callback-action://"));
}
