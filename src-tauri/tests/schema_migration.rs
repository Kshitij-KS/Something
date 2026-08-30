use std::fs;
use std::sync::Arc;
use std::thread;

use callback_lib::db::{self, DbError, Migration, PreparedCapture};
use callback_lib::domain::{CaptureRecord, LeaseState, PromiseStatus, SourceApp, SurfaceLease};
use callback_lib::review::ingest_manual;
use rusqlite::Connection;
use tempfile::tempdir;

fn open_temp_db() -> (tempfile::TempDir, db::Database) {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let database = db::Database::open(&path).expect("open database");
    (dir, database)
}

#[test]
fn fresh_migration_enables_wal_foreign_keys_and_schema_version() {
    let (_dir, database) = open_temp_db();
    let info = database.schema_info().expect("schema info");

    assert_eq!(info.user_version, db::CURRENT_SCHEMA_VERSION);
    assert_eq!(info.journal_mode.to_lowercase(), "wal");
    assert!(info.foreign_keys);
    assert!(info.busy_timeout_ms >= 5_000);
}

#[test]
fn repeated_migration_is_idempotent() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let first = db::Database::open(&path).expect("first open");
    drop(first);
    let second = db::Database::open(&path).expect("second open");
    assert_eq!(
        second.schema_info().expect("schema info").user_version,
        db::CURRENT_SCHEMA_VERSION
    );
}

#[test]
fn upgrade_applies_pending_sql_and_preserves_rows() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let conn = Connection::open(&path).expect("open raw");
    db::apply_pragmas(&conn).expect("pragmas");
    db::migrate_with(
        &conn,
        &[Migration {
            version: 1,
            sql: "CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT NOT NULL);",
        }],
    )
    .expect("v1");
    conn.execute("INSERT INTO items (label) VALUES ('kept')", [])
        .expect("insert");
    drop(conn);

    let conn = Connection::open(&path).expect("reopen");
    db::apply_pragmas(&conn).expect("pragmas");
    db::migrate_with(
        &conn,
        &[
            Migration {
                version: 1,
                sql: "CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT NOT NULL);",
            },
            Migration {
                version: 2,
                sql: "ALTER TABLE items ADD COLUMN extra TEXT NOT NULL DEFAULT 'ok';",
            },
        ],
    )
    .expect("v2");
    let extra: String = conn
        .query_row("SELECT extra FROM items WHERE label = 'kept'", [], |row| {
            row.get(0)
        })
        .expect("extra");
    let version = db::user_version(&conn).expect("version");
    assert_eq!(extra, "ok");
    assert_eq!(version, 2);
}

#[test]
fn interrupted_migration_rolls_back_ddl_and_version() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let conn = Connection::open(&path).expect("open");
    db::apply_pragmas(&conn).expect("pragmas");
    let error = db::migrate_with(
        &conn,
        &[Migration {
            version: 1,
            sql: "CREATE TABLE doomed (id INTEGER PRIMARY KEY); SELECT RAISE(ABORT, 'boom');",
        }],
    )
    .expect_err("migration should abort");
    assert!(matches!(error, DbError::Migration(_)));
    assert_eq!(db::user_version(&conn).expect("version"), 0);
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'doomed'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(exists, 0);
}

#[test]
fn newer_schema_is_rejected() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let conn = Connection::open(&path).expect("open");
    conn.pragma_update(None, "user_version", 9_999)
        .expect("stamp future schema");
    drop(conn);

    let error = db::Database::open(&path).expect_err("newer schema");
    assert!(matches!(error, DbError::NewerSchema { found: 9_999, .. }));
}

#[test]
fn deleting_a_promise_cascades_triggers_and_surface_rows() {
    let (_dir, database) = open_temp_db();
    database
        .insert_capture(&CaptureRecord::fixture("cap-1", 0))
        .expect("capture");
    let promise_id = database
        .insert_promise_from_capture("cap-1", 0, "I will send the invoice", 6, 0.8)
        .expect("promise");
    database
        .insert_trigger(promise_id, "app_focus", "slack.exe", 0)
        .expect("trigger");
    database
        .insert_lease(SurfaceLease::new(promise_id, "lease-1", "action-1"))
        .expect("lease");

    database.delete_promise(promise_id).expect("delete");

    assert_eq!(database.trigger_count(promise_id).expect("triggers"), 0);
    assert_eq!(database.lease_count(promise_id).expect("leases"), 0);
}

#[test]
fn duplicate_capture_id_and_clause_is_idempotent() {
    let (_dir, database) = open_temp_db();
    let capture = CaptureRecord::fixture("cap-dup", 1);
    let first = database.insert_capture(&capture).expect("first");
    let second = database.insert_capture(&capture).expect("second");
    assert_eq!(first, second);
    assert_eq!(database.capture_count().expect("count"), 1);
}

#[test]
fn concurrent_reader_sees_committed_wal_rows() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let database = Arc::new(db::Database::open(&path).expect("open"));
    database
        .insert_capture(&CaptureRecord::fixture("cap-wal", 0))
        .expect("capture");

    let reader = Arc::clone(&database);
    let handle = thread::spawn(move || reader.capture_count().expect("read"));
    let count = handle.join().expect("join");
    assert_eq!(count, 1);
}

#[test]
fn disk_full_is_mapped_from_sqlite_full() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let conn = Connection::open(&path).expect("open");
    db::apply_pragmas(&conn).expect("pragmas");
    conn.pragma_update(None, "max_page_count", 1)
        .expect("limit pages");
    let result = conn.execute_batch(
        "CREATE TABLE fat (payload BLOB); INSERT INTO fat (payload) VALUES (zeroblob(100000));",
    );
    drop(conn);
    let mapped = DbError::from(result.expect_err("should exhaust pages"));
    assert!(matches!(mapped, DbError::DiskFull), "mapped {mapped:?}");
}

#[test]
fn lease_recovery_releases_expired_unacted_leases() {
    let (_dir, database) = open_temp_db();
    database
        .insert_capture(&CaptureRecord::fixture("cap-lease", 0))
        .expect("capture");
    let promise_id = database
        .insert_promise_from_capture("cap-lease", 0, "I will follow up", 6, 0.9)
        .expect("promise");
    let mut lease = SurfaceLease::new(promise_id, "lease-stale", "action-stale");
    lease.state = LeaseState::Leased;
    lease.expires_at = 1;
    database.insert_lease(lease).expect("lease");

    let recovered = database.recover_leases(1_700_000_000).expect("recover");
    assert_eq!(recovered, 1);
    let stored = database
        .lease_by_token("lease-stale")
        .expect("lookup")
        .expect("present");
    assert_eq!(stored.state, LeaseState::Expired);
}

#[test]
fn corrupted_database_header_is_reported() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    fs::write(&path, b"this is not sqlite").expect("write garbage");
    let error = db::Database::open(&path).expect_err("corrupt");
    assert!(matches!(error, DbError::Corrupt(_)));
}

#[test]
fn selector_activity_migration_adds_first_observed_timestamp() {
    let dir = tempdir().expect("temp");
    let path = dir.path().join("callback.db");
    let database = db::Database::open(&path).expect("open");
    let connection = database.read_connection().expect("reader");
    let value: Option<i64> = connection
        .query_row(
            "SELECT first_observed_at FROM selector_health WHERE site = 'gmail'",
            [],
            |row| row.get(0),
        )
        .expect("column");
    assert_eq!(value, None);
    assert_eq!(
        database.schema_info().expect("schema").user_version,
        db::CURRENT_SCHEMA_VERSION
    );
}

#[test]
fn retention_deletes_expired_terminal_data_and_redacts_unresolved_context() {
    let (_dir, database) = open_temp_db();
    let now = 1_800_000_000;
    let old = now - (2 * 86_400);
    let cutoff = now - 86_400;

    let open_id = ingest_manual(&database, "retention-open", "Keep this reminder", old)
        .expect("open manual promise");
    let done_id = ingest_manual(&database, "retention-done", "Finished reminder", old)
        .expect("done manual promise");
    database
        .set_promise_status(done_id, PromiseStatus::Done, now)
        .expect("mark done");
    let review_id = ingest_manual(&database, "retention-review", "Review reminder", old)
        .expect("review manual promise");
    database
        .set_promise_status(review_id, PromiseStatus::Review, now)
        .expect("mark review");
    ingest_manual(
        &database,
        "retention-boundary",
        "Keep boundary context",
        cutoff,
    )
    .expect("boundary promise");
    database
        .commit_prepared_capture(&PreparedCapture {
            capture_id: "retention-discard-only".into(),
            payload_sha256: "a".repeat(64),
            source_app: SourceApp::Gmail,
            source_ctx: None,
            recipient: None,
            raw_message: "Can you send the invoice?".into(),
            sent_at: old,
            created_at: old,
            timezone: "UTC".into(),
            clauses: Vec::new(),
        })
        .expect("discard-only receipt");

    database
        .upsert_setting("retention_days", "1")
        .expect("retention setting");
    let report = database.enforce_retention(now).expect("retention pass");
    assert_eq!(report.cutoff_at, cutoff);
    assert_eq!(report.deleted_captures, 2);
    assert_eq!(report.deleted_receipts, 3);
    assert_eq!(report.redacted_captures, 1);
    assert_eq!(report.redacted_promises, 1);
    assert_eq!(database.capture_count().expect("capture count"), 2);

    let reader = database.read_connection().expect("reader");
    let (capture_raw, promise_raw, promise_text, status): (String, String, String, String) = reader
        .query_row(
            "SELECT captures.raw_message, promises.raw_message, promises.text, promises.status
                 FROM captures JOIN promises USING (capture_id, clause_ordinal)
                 WHERE promises.id = ?1",
            [open_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("preserved open promise");
    assert_eq!(capture_raw, "");
    assert_eq!(promise_raw, "");
    assert_eq!(promise_text, "Keep this reminder");
    assert_eq!(status, "open");

    let boundary_raw: String = reader
        .query_row(
            "SELECT raw_message FROM captures WHERE capture_id = 'retention-boundary'",
            [],
            |row| row.get(0),
        )
        .expect("boundary capture");
    assert_eq!(boundary_raw, "Keep boundary context");

    let removed: i64 = reader
        .query_row(
            "SELECT COUNT(*) FROM captures
             WHERE capture_id IN ('retention-done', 'retention-review')",
            [],
            |row| row.get(0),
        )
        .expect("removed captures");
    assert_eq!(removed, 0);
    let removed_receipts: i64 = reader
        .query_row(
            "SELECT COUNT(*) FROM capture_receipts
             WHERE capture_id IN (
               'retention-done', 'retention-review', 'retention-discard-only'
             )",
            [],
            |row| row.get(0),
        )
        .expect("removed receipts");
    assert_eq!(removed_receipts, 0);
    let retained_receipts: i64 = reader
        .query_row(
            "SELECT COUNT(*) FROM capture_receipts
             WHERE capture_id IN ('retention-open', 'retention-boundary')",
            [],
            |row| row.get(0),
        )
        .expect("retained receipts");
    assert_eq!(retained_receipts, 2);

    let repeated = database.enforce_retention(now).expect("repeat pass");
    assert_eq!(repeated.deleted_captures, 0);
    assert_eq!(repeated.deleted_receipts, 0);
    assert_eq!(repeated.redacted_captures, 0);
    assert_eq!(repeated.redacted_promises, 0);
}

#[test]
fn retention_setting_rejects_invalid_ranges() {
    let (_dir, database) = open_temp_db();
    for value in ["", "0", "3651", "forever", "-1"] {
        assert!(
            matches!(
                database.upsert_setting("retention_days", value),
                Err(DbError::InvalidSetting { .. })
            ),
            "{value} should be rejected"
        );
    }
    database
        .upsert_setting("retention_days", "3650")
        .expect("upper bound");
}

#[test]
fn schema_v3_adds_receipts_actions_and_unique_trigger_links() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let connection = Connection::open(&path).expect("open v2 database");
    db::apply_pragmas(&connection).expect("pragmas");
    db::migrate_with(&connection, &db::MIGRATIONS[..2]).expect("migrate to v2");
    connection
        .execute_batch(
            "INSERT INTO captures (
                id, capture_id, clause_ordinal, source_app, source_ctx, recipient,
                raw_message, sent_at, created_at
             ) VALUES (
                10, 'legacy-cap', 0, 'slack', 'T123:C456', 'Priya',
                'I will send the invoice tomorrow', 1700000000, 1700000100
             );
             INSERT INTO promises (
                id, capture_id, clause_ordinal, text, raw_message, source_app,
                source_ctx, recipient, confidence, score, status, ignore_count, created_at
             ) VALUES (
                20, 'legacy-cap', 0, 'I will send the invoice tomorrow',
                'I will send the invoice tomorrow', 'slack', 'T123:C456', 'Priya',
                0.8, 8, 'open', 0, 1700000100
             );
             INSERT INTO triggers (id, promise_id, kind, match_value, priority)
               VALUES (30, 20, 'app_focus', 'slack.exe', 1),
                      (31, 20, 'app_focus', 'slack.exe', 9);",
        )
        .expect("seed v2 rows");
    drop(connection);

    let database = db::Database::open(&path).expect("upgrade to v3");
    assert_eq!(
        database.schema_info().expect("schema").user_version,
        db::CURRENT_SCHEMA_VERSION
    );
    let reader = database.read_connection().expect("reader");
    let receipt_columns = {
        let mut statement = reader
            .prepare("PRAGMA table_info('capture_receipts')")
            .expect("receipt table info");
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("receipt columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect receipt columns")
    };
    assert_eq!(
        receipt_columns,
        [
            "capture_id",
            "payload_sha256",
            "source_app",
            "sent_at",
            "timezone",
            "stored_clauses",
            "committed_at",
        ]
    );
    assert!(
        !receipt_columns
            .iter()
            .any(|column| { matches!(column.as_str(), "raw_message" | "text" | "recipient") })
    );
    assert_eq!(
        reader
            .query_row(
                "SELECT COUNT(*) FROM triggers
                 WHERE promise_id = 20 AND kind = 'app_focus' AND match_value = 'slack.exe'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("deduplicated trigger count"),
        1
    );
    assert_eq!(
        reader
            .query_row("SELECT id FROM triggers WHERE promise_id = 20", [], |row| {
                row.get::<_, i64>(0)
            },)
            .expect("retained trigger"),
        30
    );
    let trigger_index_columns = {
        let mut statement = reader
            .prepare("PRAGMA index_info('idx_triggers_unique_link')")
            .expect("trigger index info");
        statement
            .query_map([], |row| row.get::<_, String>(2))
            .expect("trigger index columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect trigger index columns")
    };
    assert_eq!(trigger_index_columns, ["promise_id", "kind", "match_value"]);
    assert_eq!(
        reader
            .query_row(
                "SELECT COUNT(*) FROM pragma_index_list('triggers')
                 WHERE name = 'idx_triggers_unique_link' AND [unique] = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("unique trigger index"),
        1
    );
    let promise_columns = {
        let mut statement = reader
            .prepare("PRAGMA table_info('promises')")
            .expect("promise table info");
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("promise columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect promise columns")
    };
    assert!(
        promise_columns
            .iter()
            .any(|column| column == "deadline_escalated_at")
    );
    let deadline_index_columns = {
        let mut statement = reader
            .prepare("PRAGMA index_info('idx_promises_deadline')")
            .expect("deadline index info");
        statement
            .query_map([], |row| row.get::<_, String>(2))
            .expect("deadline index columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect deadline index columns")
    };
    assert_eq!(
        deadline_index_columns,
        ["status", "deadline", "deadline_escalated_at"]
    );
    assert!(
        reader
            .execute(
                "INSERT INTO capture_receipts (
                capture_id, payload_sha256, source_app, sent_at, timezone,
                stored_clauses, committed_at
             ) VALUES ('bad-hash', ?1, 'slack', 1, 'UTC', 0, 1)",
                ["G".repeat(64)],
            )
            .is_err()
    );
    assert!(
        reader
            .execute(
                "INSERT INTO triggers (promise_id, kind, match_value, priority)
             VALUES (20, 'app_focus', 'slack.exe', 5)",
                [],
            )
            .is_err()
    );
    drop(reader);

    let changed_legacy = callback_lib::db::PreparedCapture {
        capture_id: "legacy-cap".into(),
        payload_sha256: "b".repeat(64),
        source_app: callback_lib::domain::SourceApp::Slack,
        source_ctx: Some("T123:C456".into()),
        recipient: Some("Priya".into()),
        raw_message: "I will call Priya tomorrow".into(),
        sent_at: 1_700_000_000,
        created_at: 1_700_000_200,
        timezone: "UTC".into(),
        clauses: Vec::new(),
    };
    assert!(matches!(
        database.commit_prepared_capture(&changed_legacy),
        Err(DbError::CaptureConflict { .. })
    ));
    let exact_legacy = callback_lib::db::PreparedCapture {
        raw_message: "I will send the invoice tomorrow".into(),
        payload_sha256: "a".repeat(64),
        ..changed_legacy
    };
    let outcome = database
        .commit_prepared_capture(&exact_legacy)
        .expect("reconcile exact legacy retry");
    assert!(outcome.duplicate);
    assert_eq!(outcome.stored_clauses, 1);
    assert_eq!(
        database
            .read_connection()
            .expect("reader")
            .query_row(
                "SELECT COUNT(*) FROM capture_receipts WHERE capture_id = 'legacy-cap'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("legacy receipt"),
        1
    );
}

#[test]
fn schema_v4_deduplicates_notification_rows_and_enforces_one_per_surface() {
    let dir = tempdir().expect("temp dir");
    let path = dir.path().join("callback.db");
    let connection = Connection::open(&path).expect("open v3 database");
    db::apply_pragmas(&connection).expect("pragmas");
    db::migrate_with(&connection, &db::MIGRATIONS[..3]).expect("migrate to v3");
    connection
        .execute_batch(
            "INSERT INTO captures (
                capture_id, clause_ordinal, source_app, raw_message, sent_at, created_at
             ) VALUES ('notify-migration', 0, 'manual', 'Follow up', 1, 1);
             INSERT INTO promises (
                id, capture_id, clause_ordinal, text, raw_message, source_app,
                confidence, score, status, ignore_count, created_at
             ) VALUES (
                1, 'notify-migration', 0, 'Follow up', 'Follow up', 'manual',
                1.0, 10, 'open', 0, 1
             );
             INSERT INTO surface_attempts (
                id, promise_id, lease_token, action_token, state, expires_at, created_at
             ) VALUES (1, 1, 'lease-v3', 'action-v3', 'leased', 100, 1);
             INSERT INTO notification_attempts (
                id, surface_attempt_id, delivered, error, created_at
             ) VALUES (1, 1, 0, NULL, 1), (2, 1, 0, 'duplicate', 2);",
        )
        .expect("seed duplicate notification rows");
    drop(connection);

    let database = db::Database::open(&path).expect("upgrade to v4");
    let reader = database.read_connection().expect("reader");
    assert_eq!(
        reader
            .query_row(
                "SELECT COUNT(*) FROM notification_attempts WHERE surface_attempt_id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("deduplicated notifications"),
        1
    );
    assert_eq!(
        reader
            .query_row(
                "SELECT id FROM notification_attempts WHERE surface_attempt_id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("retained notification"),
        1
    );
    assert_eq!(
        reader
            .query_row(
                "SELECT COUNT(*) FROM pragma_index_list('notification_attempts')
                 WHERE name = 'idx_notification_attempt_surface' AND [unique] = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("notification unique index"),
        1
    );
    assert_eq!(
        reader
            .query_row(
                "SELECT COUNT(*) FROM pragma_index_list('promises')
                 WHERE name = 'idx_promises_snooze'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("snooze index"),
        1
    );
    assert!(
        reader
            .execute(
                "INSERT INTO notification_attempts (
                surface_attempt_id, delivered, error, created_at
             ) VALUES (1, 0, NULL, 3)",
                [],
            )
            .is_err()
    );
    drop(reader);

    assert_eq!(database.recover_leases(100).expect("recover"), 1);
    let recovered: (String, String) = database
        .read_connection()
        .expect("reader")
        .query_row(
            "SELECT s.state, n.error
             FROM surface_attempts s
             JOIN notification_attempts n ON n.surface_attempt_id = s.id
             WHERE s.id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("recovered pending notification");
    assert_eq!(
        recovered,
        (
            "expired".into(),
            "delivery result not recorded before lease expiry".into(),
        )
    );
}
