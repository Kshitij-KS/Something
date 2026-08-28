use std::fs;
use std::sync::Arc;
use std::thread;

use callback_lib::db::{self, DbError, Migration};
use callback_lib::domain::{CaptureRecord, LeaseState, SurfaceLease};
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
