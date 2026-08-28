use callback_lib::db::Database;
use callback_lib::domain::SourceApp;
use callback_lib::health::silence_remaining;
use callback_lib::ipc::commit::commit_envelope;
use callback_lib::native_host::autostart::{AUTOSTART_VALUE, autostart_reg_args};
use callback_lib::platform::notifications::RecordingSink;
use callback_lib::purge::{purge_from_args, purge_local_data_path};
use callback_lib::review::ingest_message;
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
fn purge_deletes_sqlite_and_sidecar_files() {
    let dir = tempdir().expect("temp");
    let path = dir.path().join("callback.db");
    drop(Database::open(&path).expect("open"));
    std::fs::write(format!("{}-wal", path.display()), b"wal").expect("wal");
    std::fs::write(format!("{}-shm", path.display()), b"shm").expect("shm");
    let report = purge_local_data_path(&path).expect("purge");
    assert!(report.deleted_db);
    assert!(!path.exists());
    assert!(!Path::new(&format!("{}-wal", path.display())).exists());
    assert!(!Path::new(&format!("{}-shm", path.display())).exists());
}

#[test]
fn purge_cli_accepts_db_flag() {
    let dir = tempdir().expect("temp");
    let path = dir.path().join("callback.db");
    drop(Database::open(&path).expect("open"));
    let report = purge_from_args(&[
        "callback".into(),
        "--purge".into(),
        "--db".into(),
        path.display().to_string(),
    ])
    .expect("cli");
    assert!(report.deleted_db);
    assert!(!path.exists());
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
    assert!(
        enable
            .iter()
            .any(|arg: &String| arg.contains("Callback.exe"))
    );
    let disable = autostart_reg_args(exe, false);
    assert!(disable.iter().any(|arg| arg == "delete"));
}
