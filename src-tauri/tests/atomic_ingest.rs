use callback_lib::db::{Database, DbError, PreparedCapture, PreparedClause, PreparedTrigger};
use callback_lib::domain::{PromiseStatus, SourceApp};
use callback_lib::ipc::commit::commit_envelope;
use callback_lib::review::ingest_manual;
use callback_protocol::{Envelope, MessageKind, PROTOCOL_VERSION};
use chrono::{TimeZone, Utc};
use tempfile::tempdir;

fn open_temp_db() -> (tempfile::TempDir, Database) {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let database = Database::open(&path).expect("open database");
    (dir, database)
}

fn capture_envelope(id: &str, source_app: &str, raw_message: &str, sent_at: i64) -> Envelope {
    Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Capture,
        id: id.into(),
        payload: serde_json::json!({
            "captureId": id,
            "sourceApp": source_app,
            "sourceCtx": "T123:C456",
            "recipient": "Priya",
            "rawMessage": raw_message,
            "sentAt": sent_at,
        }),
    }
}

fn table_count(database: &Database, table: &str) -> i64 {
    let connection = database.read_connection().expect("reader");
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("table count")
}

#[test]
fn exact_retry_keeps_one_immutable_receipt_across_timezone_changes() {
    let (_dir, database) = open_temp_db();
    database
        .upsert_setting("timezone", "UTC")
        .expect("timezone");
    let envelope = capture_envelope(
        "cap-exact-retry",
        "gmail",
        "I will send the invoice tomorrow",
        1_700_000_000,
    );
    let before = Utc::now().timestamp();
    let first = commit_envelope(&database, envelope.clone()).expect("first commit");
    let first_receipt: (String, i64, i64, String) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT payload_sha256, stored_clauses, committed_at, timezone
             FROM capture_receipts WHERE capture_id = 'cap-exact-retry'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("receipt");
    let first_counts = (
        table_count(&database, "capture_receipts"),
        table_count(&database, "captures"),
        table_count(&database, "promises"),
        table_count(&database, "triggers"),
    );

    database
        .upsert_setting("timezone", "America/New_York")
        .expect("changed timezone");
    let second = commit_envelope(&database, envelope).expect("exact retry");
    let after = Utc::now().timestamp();
    let second_receipt: (String, i64, i64, String) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT payload_sha256, stored_clauses, committed_at, timezone
             FROM capture_receipts WHERE capture_id = 'cap-exact-retry'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("receipt after retry");

    assert_eq!(first.payload["committed"], true);
    assert_eq!(second.payload["committed"], true);
    assert_eq!(first_receipt, second_receipt);
    assert_eq!(first_receipt.0.len(), 64);
    assert_eq!(first_receipt.1, first_counts.2);
    assert_eq!(first_receipt.3, "UTC");
    assert!((before..=after).contains(&first_receipt.2));
    assert_eq!(first_counts.0, 1);
    assert_eq!(
        first_counts,
        (
            table_count(&database, "capture_receipts"),
            table_count(&database, "captures"),
            table_count(&database, "promises"),
            table_count(&database, "triggers"),
        )
    );
}

#[test]
fn changed_payload_reusing_capture_id_conflicts_without_mutation() {
    let (_dir, database) = open_temp_db();
    database
        .upsert_setting("timezone", "UTC")
        .expect("timezone");
    commit_envelope(
        &database,
        capture_envelope(
            "cap-conflict",
            "slack",
            "I will send the invoice tomorrow",
            1_700_000_000,
        ),
    )
    .expect("first commit");
    let before = (
        table_count(&database, "capture_receipts"),
        table_count(&database, "captures"),
        table_count(&database, "promises"),
        table_count(&database, "triggers"),
    );
    let original: (String, String) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT c.raw_message, r.payload_sha256
             FROM captures c JOIN capture_receipts r USING (capture_id)
             WHERE c.capture_id = 'cap-conflict'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("original");

    let error = commit_envelope(
        &database,
        capture_envelope(
            "cap-conflict",
            "slack",
            "I will call Priya tomorrow",
            1_700_000_000,
        ),
    )
    .expect_err("changed payload must conflict");
    let stored: (String, String) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT c.raw_message, r.payload_sha256
             FROM captures c JOIN capture_receipts r USING (capture_id)
             WHERE c.capture_id = 'cap-conflict'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("stored original");

    assert!(error.contains("conflicts with previously committed content"));
    assert_eq!(stored, original);
    assert_eq!(
        before,
        (
            table_count(&database, "capture_receipts"),
            table_count(&database, "captures"),
            table_count(&database, "promises"),
            table_count(&database, "triggers"),
        )
    );
}

#[test]
fn discard_only_capture_commits_a_content_free_receipt() {
    let (_dir, database) = open_temp_db();
    database
        .upsert_setting("timezone", "UTC")
        .expect("timezone");
    let envelope = capture_envelope(
        "cap-discard-only",
        "gmail",
        "Can you send the invoice?",
        1_700_000_000_000,
    );
    let first = commit_envelope(&database, envelope.clone()).expect("discard receipt");
    let receipt: (i64, i64, String, String) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT stored_clauses, sent_at, source_app, timezone
             FROM capture_receipts WHERE capture_id = 'cap-discard-only'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("receipt");
    let last_capture: Option<i64> = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT last_capture_at FROM selector_health WHERE site = 'gmail'",
            [],
            |row| row.get(0),
        )
        .expect("selector health");

    assert_eq!(first.payload["committed"], true);
    assert_eq!(receipt, (0, 1_700_000_000, "gmail".into(), "UTC".into()));
    assert_eq!(table_count(&database, "capture_receipts"), 1);
    assert_eq!(table_count(&database, "captures"), 0);
    assert_eq!(table_count(&database, "promises"), 0);
    assert_eq!(table_count(&database, "triggers"), 0);
    assert_eq!(last_capture, Some(1_700_000_000));

    commit_envelope(&database, envelope).expect("exact discard retry");
    assert_eq!(table_count(&database, "capture_receipts"), 1);
    assert!(
        commit_envelope(
            &database,
            capture_envelope(
                "cap-discard-only",
                "gmail",
                "I will send the invoice tomorrow",
                1_700_000_000_000,
            ),
        )
        .is_err()
    );
}

#[test]
fn trigger_failure_rolls_back_the_entire_capture_transaction() {
    let (_dir, database) = open_temp_db();
    let mut prepared = PreparedCapture {
        capture_id: "cap-rollback".into(),
        payload_sha256: "a".repeat(64),
        source_app: SourceApp::Slack,
        source_ctx: Some("T123:C456".into()),
        recipient: Some("Priya".into()),
        raw_message: "I will follow up".into(),
        sent_at: 1_700_000_000,
        created_at: 1_700_000_100,
        timezone: "UTC".into(),
        clauses: vec![PreparedClause {
            ordinal: 0,
            text: "I will follow up".into(),
            score: 8,
            confidence: 0.8,
            status: PromiseStatus::Open,
            deadline: None,
            triggers: vec![PreparedTrigger {
                kind: "invalid_kind".into(),
                match_value: "slack.exe".into(),
                priority: 10,
            }],
        }],
    };

    let error = database
        .commit_prepared_capture(&prepared)
        .expect_err("trigger constraint must fail");
    assert!(matches!(error, DbError::Sqlite(_)));
    for table in ["capture_receipts", "captures", "promises", "triggers"] {
        assert_eq!(table_count(&database, table), 0, "{table} rolled back");
    }
    let last_capture: Option<i64> = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT last_capture_at FROM selector_health WHERE site = 'slack'",
            [],
            |row| row.get(0),
        )
        .expect("selector health");
    assert_eq!(last_capture, None);

    prepared.clauses[0].triggers[0].kind = "app_focus".into();
    let outcome = database
        .commit_prepared_capture(&prepared)
        .expect("retry after rollback");
    assert!(!outcome.duplicate);
    assert_eq!(outcome.stored_clauses, 1);
}

#[test]
fn disabled_site_is_terminally_discarded_without_storage() {
    let (_dir, database) = open_temp_db();
    database
        .upsert_setting("slack_enabled", "false")
        .expect("disable Slack");
    let ack = commit_envelope(
        &database,
        capture_envelope(
            "cap-disabled",
            "slack",
            "I will send the invoice tomorrow",
            1_700_000_000,
        ),
    )
    .expect("terminal discard ack");

    assert_eq!(ack.kind, MessageKind::Ack);
    assert_eq!(ack.id, "cap-disabled");
    assert_eq!(ack.payload["committed"], false);
    assert_eq!(ack.payload["discard"], true);
    assert_eq!(ack.payload["reason"], "site_disabled");
    assert_eq!(ack.payload["site_policy"]["slack"], false);
    for table in ["capture_receipts", "captures", "promises", "triggers"] {
        assert_eq!(table_count(&database, table), 0, "{table} remains empty");
    }
    let last_capture: Option<i64> = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT last_capture_at FROM selector_health WHERE site = 'slack'",
            [],
            |row| row.get(0),
        )
        .expect("selector health");
    assert_eq!(last_capture, None);
}

#[test]
fn envelope_deadline_uses_sent_at_and_persists_the_configured_timezone() {
    let (_dir, database) = open_temp_db();
    database
        .upsert_setting("timezone", "UTC")
        .expect("timezone");
    let sent_at = Utc
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("sent timestamp")
        .timestamp();
    let expected_deadline = Utc
        .with_ymd_and_hms(2026, 8, 27, 17, 0, 0)
        .single()
        .expect("deadline")
        .timestamp();
    let before = Utc::now().timestamp();
    commit_envelope(
        &database,
        capture_envelope(
            "cap-sent-deadline",
            "gmail",
            "I will send the invoice tomorrow",
            sent_at * 1_000,
        ),
    )
    .expect("commit");
    let after = Utc::now().timestamp();

    let stored: (i64, i64, i64, String, String, String, i64, i64) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT c.sent_at, r.sent_at, p.deadline, p.deadline_tz,
                    p.deadline_precision, r.timezone, c.created_at, r.committed_at
             FROM captures c
             JOIN promises p USING (capture_id, clause_ordinal)
             JOIN capture_receipts r USING (capture_id)
             WHERE c.capture_id = 'cap-sent-deadline'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .expect("stored deadline");

    assert_eq!(stored.0, sent_at);
    assert_eq!(stored.1, sent_at);
    assert_eq!(stored.2, expected_deadline);
    assert_eq!(stored.3, "UTC");
    assert_eq!(stored.4, "eod");
    assert_eq!(stored.5, "UTC");
    assert!((before..=after).contains(&stored.6));
    assert!((before..=after).contains(&stored.7));
}

#[test]
fn manual_deadline_obeys_new_york_dst_and_invalid_timezones_fail_closed() {
    let (_dir, database) = open_temp_db();
    database
        .upsert_setting("timezone", "America/New_York")
        .expect("timezone");
    let captured_at = Utc
        .with_ymd_and_hms(2026, 3, 7, 17, 0, 0)
        .single()
        .expect("capture timestamp")
        .timestamp();
    let expected_deadline = Utc
        .with_ymd_and_hms(2026, 3, 8, 21, 0, 0)
        .single()
        .expect("DST deadline")
        .timestamp();
    let promise_id = ingest_manual(
        &database,
        "manual-dst",
        "I will send the invoice tomorrow",
        captured_at,
    )
    .expect("manual capture");
    let stored: (i64, String, String, String) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT p.deadline, p.deadline_tz, p.deadline_precision, r.timezone
             FROM promises p JOIN capture_receipts r USING (capture_id)
             WHERE p.id = ?1",
            [promise_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("manual deadline");
    assert_eq!(
        stored,
        (
            expected_deadline,
            "America/New_York".into(),
            "eod".into(),
            "America/New_York".into(),
        )
    );

    database
        .upsert_setting("timezone", "UTC")
        .expect("changed timezone");
    assert_eq!(
        ingest_manual(
            &database,
            "manual-dst",
            "I will send the invoice tomorrow",
            captured_at,
        )
        .expect("idempotent manual retry"),
        promise_id
    );
    assert!(matches!(
        database.upsert_setting("timezone", "Mars/Olympus"),
        Err(DbError::InvalidSetting { .. })
    ));

    let connection = database
        .read_connection()
        .expect("writer for corruption test");
    connection
        .execute(
            "UPDATE settings SET v = 'Mars/Olympus' WHERE k = 'timezone'",
            [],
        )
        .expect("corrupt timezone setting");
    drop(connection);
    let counts_before = (
        table_count(&database, "capture_receipts"),
        table_count(&database, "captures"),
        table_count(&database, "promises"),
        table_count(&database, "triggers"),
    );
    assert!(matches!(
        ingest_manual(
            &database,
            "manual-invalid-timezone",
            "I will send the report tomorrow",
            captured_at,
        ),
        Err(DbError::InvalidSetting { key, .. }) if key == "timezone"
    ));
    assert_eq!(
        counts_before,
        (
            table_count(&database, "capture_receipts"),
            table_count(&database, "captures"),
            table_count(&database, "promises"),
            table_count(&database, "triggers"),
        )
    );
}
