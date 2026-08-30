use callback_lib::db::Database;
use callback_lib::domain::SourceApp;
use callback_lib::health::silence_remaining;
use callback_lib::ipc::commit::commit_envelope;
use callback_lib::native_host::autostart::{AUTOSTART_VALUE, autostart_reg_args};
use callback_lib::platform::notifications::RecordingSink;
use callback_lib::purge::{purge_from_args, purge_local_data_path};
use callback_lib::review::{ingest_manual, ingest_message};
use callback_lib::surfacing::phase0::{Phase0Rule, notify_matched};
use callback_protocol::{Envelope, MessageKind, PROTOCOL_VERSION};
use chrono::{Duration, TimeZone, Utc};
use std::path::Path;
use tempfile::tempdir;

fn open_temp_db() -> (tempfile::TempDir, Database) {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let database = Database::open(&path).expect("open database");
    (dir, database)
}

#[test]
fn capture_envelope_is_committed_before_ack() {
    let (_dir, database) = open_temp_db();
    let envelope = Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Capture,
        id: "cap-1".into(),
        payload: serde_json::json!({
            "captureId": "cap-1",
            "sourceApp": "slack",
            "sourceCtx": "D0123",
            "recipient": "Priya",
            "rawMessage": "I will send the invoice tomorrow",
            "sentAt": 1_700_000_000_000_i64
        }),
    };
    let ack = commit_envelope(&database, envelope).expect("commit");
    assert_eq!(ack.kind, MessageKind::Ack);
    assert_eq!(ack.id, "cap-1");
    assert_eq!(ack.payload["committed"], true);
    assert!(database.capture_count().expect("count") >= 1);
}

#[test]
fn duplicate_capture_envelope_is_idempotent_and_still_acked() {
    let (_dir, database) = open_temp_db();
    let envelope = Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Capture,
        id: "cap-dup".into(),
        payload: serde_json::json!({
            "captureId": "cap-dup",
            "sourceApp": "gmail",
            "rawMessage": "I will send the invoice tomorrow",
            "sentAt": 1_700_000_000
        }),
    };
    let first = commit_envelope(&database, envelope.clone()).expect("first");
    let second = commit_envelope(&database, envelope).expect("second");
    assert_eq!(first.payload["committed"], true);
    assert_eq!(second.payload["committed"], true);
    assert_eq!(database.capture_count().expect("count"), 1);
}

#[test]
fn handshake_envelope_acks_without_storing_a_capture() {
    let (_dir, database) = open_temp_db();
    let ack = commit_envelope(
        &database,
        Envelope {
            protocol_version: PROTOCOL_VERSION,
            kind: MessageKind::Handshake,
            id: "hs-1".into(),
            payload: serde_json::json!({}),
        },
    )
    .expect("handshake");
    assert_eq!(ack.kind, MessageKind::Ack);
    assert_eq!(ack.payload["committed"], true);
    assert_eq!(database.capture_count().expect("count"), 0);
}

#[test]
fn phase0_match_delivers_to_notification_sink() {
    let sink = RecordingSink::default();
    let rules = [Phase0Rule {
        id: 1,
        app_match: "Slack.exe".into(),
        reminder_text: "Follow up with Priya".into(),
        enabled: true,
    }];
    assert!(notify_matched(r"C:\Program Files\Slack\slack.exe", &rules, &sink).expect("notify"));
    let shown = sink.shown();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].body, "Follow up with Priya");
    assert_eq!(shown[0].title, "Callback");
}

#[test]
fn disabled_phase0_rule_does_not_notify() {
    let sink = RecordingSink::default();
    let rules = [Phase0Rule {
        id: 1,
        app_match: "slack.exe".into(),
        reminder_text: "Hidden".into(),
        enabled: false,
    }];
    assert!(!notify_matched("slack.exe", &rules, &sink).expect("notify"));
    assert!(sink.shown().is_empty());
}

#[test]
fn ingest_auto_links_context_and_app_triggers() {
    let (_dir, database) = open_temp_db();
    let stored = ingest_message(
        &database,
        "cap-link",
        SourceApp::Slack,
        Some("D0123"),
        Some("Priya"),
        "I will send the invoice tomorrow",
        1_700_000_000,
        Utc::now(),
        0,
        "UTC",
    )
    .expect("ingest");
    assert!(stored >= 1);
    let promise_id = database
        .list_by_status(callback_lib::domain::PromiseStatus::Open)
        .expect("list")
        .into_iter()
        .chain(
            database
                .list_by_status(callback_lib::domain::PromiseStatus::Review)
                .expect("review"),
        )
        .next()
        .expect("promise")
        .0;
    assert!(
        database.trigger_count(promise_id).expect("triggers") >= 2,
        "expected app + context triggers"
    );
}

#[test]
fn manual_capture_persists_low_scoring_text_exactly_once() {
    let (_dir, database) = open_temp_db();
    let first = ingest_manual(&database, "manual-stable", "  Buy milk  ", 1_700_000_000)
        .expect("manual capture");
    let retry = ingest_manual(&database, "manual-stable", "Buy milk", 1_700_000_000)
        .expect("idempotent retry");

    assert_eq!(first, retry);
    assert_eq!(database.capture_count().expect("capture count"), 1);
    let rows = database
        .list_by_status(callback_lib::domain::PromiseStatus::Open)
        .expect("open promises");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, first);
    assert_eq!(rows[0].1, "Buy milk");
    assert_eq!(rows[0].2, "manual");

    let reader = database.read_connection().expect("reader");
    let (manual_links, invalid_app_links): (i64, i64) = reader
        .query_row(
            "SELECT
               SUM(CASE WHEN kind = 'manual' THEN 1 ELSE 0 END),
               SUM(CASE WHEN kind = 'app_focus' AND match_value = 'manual' THEN 1 ELSE 0 END)
             FROM triggers WHERE promise_id = ?1",
            [first],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("trigger kinds");
    assert_eq!(manual_links, 1);
    assert_eq!(invalid_app_links, 0);
}

#[test]
fn manual_capture_rejects_empty_text_without_storage() {
    let (_dir, database) = open_temp_db();
    assert!(ingest_manual(&database, "manual-empty", "   ", 1_700_000_000).is_err());
    assert_eq!(database.capture_count().expect("capture count"), 0);
}

#[test]
fn purge_deletes_sqlite_and_sidecar_files() {
    let dir = tempdir().expect("temp");
    let path = dir.path().join("callback.db");
    drop(Database::open(&path).expect("open"));
    std::fs::write(format!("{}-wal", path.display()), b"wal").expect("wal");
    std::fs::write(format!("{}-shm", path.display()), b"shm").expect("shm");
    std::fs::write(format!("{}-journal", path.display()), b"journal").expect("journal");
    let report = purge_local_data_path(&path).expect("purge");
    assert!(report.deleted_db);
    assert!(!path.exists());
    assert!(!Path::new(&format!("{}-wal", path.display())).exists());
    assert!(!Path::new(&format!("{}-shm", path.display())).exists());
    assert!(!Path::new(&format!("{}-journal", path.display())).exists());
}

#[test]
fn purge_cli_accepts_db_flag() {
    let dir = tempdir().expect("temp");
    let path = dir.path().join("callback.db");
    let manifest = dir.path().join("callback-native-host.json");
    drop(Database::open(&path).expect("open"));
    std::fs::write(&manifest, b"{}").expect("manifest");
    let report = purge_from_args(&[
        "callback".into(),
        "--purge".into(),
        "--db".into(),
        path.display().to_string(),
        "--manifest".into(),
        manifest.display().to_string(),
        "--skip-registration".into(),
    ])
    .expect("cli");
    assert!(report.deleted_db);
    assert!(report.deleted_manifest);
    assert!(!path.exists());
    assert!(!manifest.exists());
}

#[test]
fn onboarding_silence_remaining_counts_down() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .expect("now");
    let until = now + Duration::minutes(10);
    assert_eq!(
        silence_remaining(now, Some(&until.timestamp().to_string())),
        600
    );
    assert_eq!(
        silence_remaining(
            now + Duration::minutes(11),
            Some(&until.timestamp().to_string())
        ),
        0
    );
    assert_eq!(silence_remaining(now, None), 0);
}

#[test]
fn autostart_registry_args_enable_and_disable() {
    let exe = Path::new(r"C:\Program Files\Callback\Callback.exe");
    let enable = autostart_reg_args(exe, true);
    assert!(enable.iter().any(|arg| arg == "add"));
    assert!(enable.join(" ").contains(AUTOSTART_VALUE));
    assert_eq!(
        enable
            .windows(2)
            .find(|pair| pair[0] == "/d")
            .map(|pair| pair[1].as_str()),
        Some(r#""C:\Program Files\Callback\Callback.exe""#)
    );
    let disable = autostart_reg_args(exe, false);
    assert!(disable.iter().any(|arg| arg == "delete"));
}

#[test]
fn malformed_capture_is_rejected_without_ack_or_storage() {
    let (_dir, database) = open_temp_db();
    let error = commit_envelope(
        &database,
        Envelope {
            protocol_version: PROTOCOL_VERSION,
            kind: MessageKind::Capture,
            id: "cap-invalid".into(),
            payload: serde_json::json!({
                "captureId": "different-id",
                "sourceApp": "slack",
                "rawMessage": "I will send the invoice tomorrow",
                "sentAt": 1_700_000_000
            }),
        },
    )
    .expect_err("mismatched capture id must fail");

    assert!(error.contains("capture id"));
    assert_eq!(database.capture_count().expect("count"), 0);
}

#[test]
fn kill_gates_enforce_plan_order_and_record_evidence() {
    let (_dir, database) = open_temp_db();
    assert!(
        database
            .update_kill_gate(
                "extraction_precision_300",
                "passed",
                "Precision was above the required threshold.",
            )
            .is_err()
    );
    database
        .update_kill_gate(
            "phase0_five_day",
            "passed",
            "Five days of local use showed context reminders were materially better.",
        )
        .expect("phase 0 gate");
    database
        .update_kill_gate(
            "extraction_precision_300",
            "passed",
            "The labeled 300-message corpus exceeded seventy percent precision.",
        )
        .expect("precision gate");
    assert!(
        database
            .kill_gate_passed("extraction_precision_300")
            .expect("gate status")
    );
}

#[test]
fn review_items_receive_triggers_only_after_promotion() {
    let (_dir, database) = open_temp_db();
    ingest_message(
        &database,
        "cap-review-only",
        SourceApp::Slack,
        Some("D0123"),
        Some("Priya"),
        "I will review",
        1_700_000_000,
        Utc::now(),
        0,
        "UTC",
    )
    .expect("ingest");
    let promise_id = database
        .list_by_status(callback_lib::domain::PromiseStatus::Review)
        .expect("review rows")
        .into_iter()
        .next()
        .expect("review promise")
        .0;
    assert_eq!(database.trigger_count(promise_id).expect("before"), 0);

    callback_lib::review::apply_review(
        &database,
        promise_id,
        callback_lib::domain::PromiseStatus::Review,
        callback_lib::review::ReviewAction::Edit,
        "I will review the invoice tomorrow",
        1_700_000_100,
    )
    .expect("promote");

    assert!(database.trigger_count(promise_id).expect("after") >= 2);
    assert_eq!(
        database.promise_text(promise_id).expect("text").as_deref(),
        Some("I will review the invoice tomorrow")
    );
}

#[test]
fn selector_probes_and_captures_update_durable_content_free_health() {
    let (_dir, database) = open_temp_db();
    let start = 1_700_000_000;
    for offset in 0..3 {
        database
            .record_selector_probe("gmail", false, start + offset)
            .expect("failed probe");
    }
    let broken = database
        .selector_health()
        .expect("health")
        .into_iter()
        .find(|record| record.site == "gmail")
        .expect("gmail");
    assert_eq!(broken.state, "broken");
    assert_eq!(broken.consecutive_failures, 3);
    assert_eq!(broken.first_observed_at, Some(start));

    database
        .record_selector_capture("gmail", start + 60)
        .expect("capture");
    let recovered = database
        .selector_health()
        .expect("health")
        .into_iter()
        .find(|record| record.site == "gmail")
        .expect("gmail");
    assert_eq!(recovered.state, "healthy");
    assert_eq!(recovered.consecutive_failures, 0);
    assert_eq!(recovered.last_capture_at, Some(start + 60));

    let parsed = callback_lib::health::parse_selector_probe(&serde_json::json!({
        "site": "slack",
        "ok": true,
        "observed_at": 1_700_000_000_000_i64,
        "missed_count": 0
    }))
    .expect("probe payload");
    assert_eq!(parsed.observed_at, 1_700_000_000);
}
