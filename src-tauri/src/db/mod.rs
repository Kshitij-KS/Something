use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::{CaptureRecord, LeaseState, PromiseStatus, SurfaceLease};

const INITIAL_SCHEMA: &str = include_str!("../../migrations/0001_initial.sql");

/// Schema version applied by the bundled production migrations.
pub const CURRENT_SCHEMA_VERSION: i32 = 1;

/// A numbered SQL migration.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// Monotonic schema version applied by this script.
    pub version: i32,
    /// SQL executed inside a single transaction.
    pub sql: &'static str,
}

/// Production migrations applied to a Callback store.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: INITIAL_SCHEMA,
}];

/// Snapshot of connection pragmas used by tests and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaInfo {
    /// SQLite `user_version`.
    pub user_version: i32,
    /// Journal mode string, typically `wal`.
    pub journal_mode: String,
    /// Whether `foreign_keys` is on.
    pub foreign_keys: bool,
    /// Busy timeout in milliseconds.
    pub busy_timeout_ms: i32,
}

/// Storage failures, including migration and integrity cases.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The file is a Callback database from a newer app.
    #[error("database schema {found} is newer than supported {supported}")]
    NewerSchema {
        /// Version found on disk.
        found: i32,
        /// Version this binary can open.
        supported: i32,
    },
    /// SQLite reported a corrupt or non-database file.
    #[error("database is corrupt: {0}")]
    Corrupt(String),
    /// Disk or quota is exhausted.
    #[error("database or disk is full")]
    DiskFull,
    /// A migration script failed and was rolled back.
    #[error("migration failed: {0}")]
    Migration(String),
    /// Settings JSON or key failed validation.
    #[error("invalid setting {key}: {reason}")]
    InvalidSetting {
        /// Settings key.
        key: String,
        /// Human-readable reason.
        reason: String,
    },
    /// Writer mutex was poisoned.
    #[error("database writer lock poisoned")]
    Poisoned,
    /// Underlying SQLite error.
    #[error(transparent)]
    Sqlite(rusqlite::Error),
}

impl From<rusqlite::Error> for DbError {
    fn from(error: rusqlite::Error) -> Self {
        map_sqlite_error(error)
    }
}

/// Maps rusqlite failures onto the Callback storage error taxonomy.
#[must_use]
pub fn map_sqlite_error(error: rusqlite::Error) -> DbError {
    if let rusqlite::Error::SqliteFailure(code, message) = &error {
        let text = message.clone().unwrap_or_default();
        match code.code {
            rusqlite::ErrorCode::DiskFull => return DbError::DiskFull,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase => {
                return DbError::Corrupt(text);
            }
            _ if text.contains("not a database") => return DbError::Corrupt(text),
            _ if text.contains("disk is full") => return DbError::DiskFull,
            _ => {}
        }
    }
    let text = error.to_string();
    if text.contains("not a database") {
        DbError::Corrupt(text)
    } else {
        DbError::Sqlite(error)
    }
}

/// Single-writer SQLite store with WAL readers.
pub struct Database {
    path: PathBuf,
    writer: Mutex<Connection>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Database")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Database {
    /// Opens or creates a Callback database, applying migrations.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the file cannot be opened, is newer than this
    /// binary, is corrupt, or a migration fails.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| DbError::Migration(format!("create parent directory: {error}")))?;
        }
        let conn = Connection::open(path)?;
        apply_pragmas(&conn)?;
        probe_integrity(&conn)?;
        let found = user_version(&conn)?;
        if found > CURRENT_SCHEMA_VERSION {
            return Err(DbError::NewerSchema {
                found,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        migrate_with(&conn, MIGRATIONS)?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: Mutex::new(conn),
        })
    }

    /// Returns pragma and schema version diagnostics.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] if the writer connection cannot be used.
    pub fn schema_info(&self) -> Result<SchemaInfo, DbError> {
        self.with_writer(|conn| {
            Ok(SchemaInfo {
                user_version: user_version(conn)?,
                journal_mode: conn.pragma_query_value(None, "journal_mode", |row| row.get(0))?,
                foreign_keys: conn.pragma_query_value(None, "foreign_keys", |row| {
                    let value: i64 = row.get(0)?;
                    Ok(value != 0)
                })?,
                busy_timeout_ms: conn.pragma_query_value(None, "busy_timeout", |row| row.get(0))?,
            })
        })
    }

    /// Inserts a capture or returns the existing row id for the same key.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn insert_capture(&self, capture: &CaptureRecord) -> Result<i64, DbError> {
        self.with_writer(|conn| insert_capture(conn, capture))
    }

    /// Counts stored captures.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn capture_count(&self) -> Result<i64, DbError> {
        self.with_writer(|conn| {
            conn.query_row("SELECT COUNT(*) FROM captures", [], |row| row.get(0))
                .map_err(DbError::from)
        })
    }

    /// Creates a promise linked to an existing capture.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn insert_promise_from_capture(
        &self,
        capture_id: &str,
        clause_ordinal: i64,
        text: &str,
        score: i32,
        confidence: f64,
    ) -> Result<i64, DbError> {
        self.with_writer(|conn| {
            insert_promise_from_capture(conn, capture_id, clause_ordinal, text, score, confidence)
        })
    }

    /// Inserts a trigger row.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn insert_trigger(
        &self,
        promise_id: i64,
        kind: &str,
        match_value: &str,
        priority: i32,
    ) -> Result<i64, DbError> {
        self.with_writer(|conn| {
            conn.execute(
                "INSERT INTO triggers (promise_id, kind, match_value, priority) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![promise_id, kind, match_value, priority],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Inserts a surface lease/attempt.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn insert_lease(&self, lease: SurfaceLease) -> Result<i64, DbError> {
        self.with_writer(|conn| insert_lease(conn, &lease))
    }

    /// Deletes a promise, cascading dependents.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn delete_promise(&self, promise_id: i64) -> Result<(), DbError> {
        self.with_writer(|conn| {
            conn.execute("DELETE FROM promises WHERE id = ?1", [promise_id])?;
            Ok(())
        })
    }

    /// Counts triggers for a promise.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn trigger_count(&self, promise_id: i64) -> Result<i64, DbError> {
        self.with_writer(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM triggers WHERE promise_id = ?1",
                [promise_id],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
    }

    /// Counts surface attempts for a promise.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn lease_count(&self, promise_id: i64) -> Result<i64, DbError> {
        self.with_writer(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM surface_attempts WHERE promise_id = ?1",
                [promise_id],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
    }

    /// Recovers expired unacted leases.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn recover_leases(&self, now_unix: i64) -> Result<u64, DbError> {
        self.with_writer(|conn| {
            let changed = conn.execute(
                "UPDATE surface_attempts
                 SET state = 'expired'
                 WHERE state IN ('leased', 'shown')
                   AND expires_at <= ?1
                   AND action IS NULL",
                [now_unix],
            )?;
            Ok(changed as u64)
        })
    }

    /// Loads a lease by token.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn lease_by_token(&self, token: &str) -> Result<Option<SurfaceLease>, DbError> {
        self.lease_by_column("lease_token", token)
    }

    /// Loads a lease by the crash-safe action token shown on the toast.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn lease_by_token_action(&self, token: &str) -> Result<Option<SurfaceLease>, DbError> {
        self.lease_by_column("action_token", token)
    }

    fn lease_by_column(&self, column: &str, token: &str) -> Result<Option<SurfaceLease>, DbError> {
        self.with_writer(|conn| {
            let sql = format!(
                "SELECT promise_id, lease_token, action_token, state, expires_at
                 FROM surface_attempts WHERE {column} = ?1"
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query([token])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            Ok(Some(SurfaceLease {
                promise_id: row.get(0)?,
                lease_token: row.get(1)?,
                action_token: row.get(2)?,
                state: LeaseState::from_db(&row.get::<_, String>(3)?).map_err(|reason| {
                    DbError::InvalidSetting {
                        key: "lease_state".into(),
                        reason,
                    }
                })?,
                expires_at: row.get(4)?,
            }))
        })
    }

    /// Open (or due snoozed) promises joined to their triggers.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn list_surfaceable_rows(&self, now_unix: i64) -> Result<Vec<SurfaceableRow>, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare(
                "SELECT p.id, t.kind, t.match_value, t.priority, p.deadline, p.confidence, p.created_at
                 FROM promises p
                 JOIN triggers t ON t.promise_id = p.id
                 WHERE p.status = 'open'
                    OR (p.status = 'snoozed' AND (p.snooze_until IS NULL OR p.snooze_until <= ?1))",
            )?;
            let rows = stmt.query_map([now_unix], |row| {
                Ok(SurfaceableRow {
                    promise_id: row.get(0)?,
                    kind: row.get(1)?,
                    match_value: row.get(2)?,
                    priority: row.get(3)?,
                    deadline_ts: row.get(4)?,
                    confidence: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
        })
    }

    /// Promise clause text used as the toast body.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn promise_text(&self, promise_id: i64) -> Result<Option<String>, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare("SELECT text FROM promises WHERE id = ?1")?;
            let mut rows = stmt.query([promise_id])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get(0)?)),
                None => Ok(None),
            }
        })
    }

    /// Whether this promise already had a surface attempt on the local calendar day.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn promise_shown_on_day(&self, promise_id: i64, local_day: &str) -> Result<bool, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare(
                "SELECT 1 FROM surface_attempts
                 WHERE promise_id = ?1 AND local_day = ?2
                   AND state IN ('leased', 'shown', 'acted')
                 LIMIT 1",
            )?;
            let mut rows = stmt.query(rusqlite::params![promise_id, local_day])?;
            Ok(rows.next()?.is_some())
        })
    }

    /// True when a leased/shown attempt is still outstanding.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn has_active_surface(&self, now_unix: i64) -> Result<bool, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare(
                "SELECT 1 FROM surface_attempts
                 WHERE state IN ('leased', 'shown')
                   AND expires_at > ?1
                   AND action IS NULL
                 LIMIT 1",
            )?;
            let mut rows = stmt.query([now_unix])?;
            Ok(rows.next()?.is_some())
        })
    }

    /// Surfaces credited to a local calendar day.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn count_surfaces_on_day(&self, local_day: &str) -> Result<u32, DbError> {
        self.with_writer(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM surface_attempts
                 WHERE local_day = ?1 AND state IN ('leased', 'shown', 'acted')",
                [local_day],
                |row| {
                    let count: i64 = row.get(0)?;
                    Ok(u32::try_from(count).unwrap_or(0))
                },
            )
            .map_err(DbError::from)
        })
    }

    /// Latest toast shown_at stamp.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn last_shown_at(&self) -> Result<Option<i64>, DbError> {
        self.with_writer(|conn| {
            conn.query_row("SELECT MAX(shown_at) FROM surface_attempts", [], |row| {
                row.get(0)
            })
            .map_err(DbError::from)
        })
    }

    /// Local day of the most recent surface attempt that recorded one.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn last_surface_day(&self) -> Result<Option<String>, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare(
                "SELECT local_day FROM surface_attempts
                 WHERE local_day IS NOT NULL
                 ORDER BY COALESCE(shown_at, created_at) DESC
                 LIMIT 1",
            )?;
            let mut rows = stmt.query([])?;
            match rows.next()? {
                Some(row) => Ok(row.get(0)?),
                None => Ok(None),
            }
        })
    }

    /// Marks a lease shown after the sink accepted the toast.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn mark_lease_shown(
        &self,
        lease_token: &str,
        now_unix: i64,
        local_day: &str,
    ) -> Result<(), DbError> {
        self.with_writer(|conn| {
            conn.execute(
                "UPDATE surface_attempts
                 SET state = 'shown', shown_at = ?2, local_day = ?3
                 WHERE lease_token = ?1",
                rusqlite::params![lease_token, now_unix, local_day],
            )?;
            Ok(())
        })
    }

    /// Validates and upserts a setting.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::InvalidSetting`] when validation fails.
    pub fn upsert_setting(&self, key: &str, value: &str) -> Result<(), DbError> {
        validate_setting(key, value)?;
        self.with_writer(|conn| {
            conn.execute(
                "INSERT INTO settings (k, v) VALUES (?1, ?2)
                 ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                rusqlite::params![key, value],
            )?;
            Ok(())
        })
    }

    /// Reads a setting value.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare("SELECT v FROM settings WHERE k = ?1")?;
            let mut rows = stmt.query([key])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get(0)?)),
                None => Ok(None),
            }
        })
    }

    /// Opens a short-lived reader against the WAL database.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] if the reader cannot be opened.
    pub fn read_connection(&self) -> Result<Connection, DbError> {
        let conn = Connection::open(&self.path)?;
        apply_pragmas(&conn)?;
        Ok(conn)
    }

    /// Filesystem path of this database.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Updates a promise status after a validated transition.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn set_promise_status(
        &self,
        promise_id: i64,
        status: PromiseStatus,
        now: i64,
    ) -> Result<(), DbError> {
        self.with_writer(|conn| {
            let resolved = matches!(
                status,
                PromiseStatus::Done | PromiseStatus::Dismissed | PromiseStatus::Archived
            );
            conn.execute(
                "UPDATE promises SET status = ?1, resolved_at = CASE WHEN ?2 THEN ?3 ELSE resolved_at END WHERE id = ?4",
                rusqlite::params![status.as_str(), resolved, now, promise_id],
            )?;
            Ok(())
        })
    }

    /// Upserts a learned blocklist skeleton.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn upsert_blocklist(&self, pattern: &str, now: i64) -> Result<(), DbError> {
        self.with_writer(|conn| {
            conn.execute(
                "INSERT INTO blocklist (pattern, hits, created_at) VALUES (?1, 1, ?2)
                 ON CONFLICT(pattern) DO UPDATE SET hits = hits + 1",
                rusqlite::params![pattern, now],
            )?;
            Ok(())
        })
    }

    /// Returns all blocklist skeletons.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn blocklist_patterns(&self) -> Result<Vec<String>, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare("SELECT pattern FROM blocklist")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
        })
    }

    /// Inserts a scored promise from extraction.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn insert_extracted_promise(
        &self,
        capture_id: &str,
        clause_ordinal: i64,
        text: &str,
        score: i32,
        confidence: f64,
        deadline: Option<(i64, String, String)>,
    ) -> Result<i64, DbError> {
        self.with_writer(|conn| {
            insert_promise_from_capture(conn, capture_id, clause_ordinal, text, score, confidence)?;
            if let Some((ts, tz, precision)) = deadline {
                conn.execute(
                    "UPDATE promises SET deadline = ?1, deadline_tz = ?2, deadline_precision = ?3
                     WHERE capture_id = ?4 AND clause_ordinal = ?5",
                    rusqlite::params![ts, tz, precision, capture_id, clause_ordinal],
                )?;
            }
            conn.query_row(
                "SELECT id FROM promises WHERE capture_id = ?1 AND clause_ordinal = ?2",
                rusqlite::params![capture_id, clause_ordinal],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
    }

    /// Lists review-status promises.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn list_by_status(
        &self,
        status: PromiseStatus,
    ) -> Result<Vec<(i64, String, String, Option<String>, i32)>, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, text, source_app, recipient, score FROM promises WHERE status = ?1 ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map([status.as_str()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
        })
    }

    /// Lists Phase 0 hardcoded rules.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn list_phase0_rules(&self) -> Result<Vec<(i64, String, String, bool)>, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, app_match, reminder_text, enabled FROM phase0_rules ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| {
                let enabled: i64 = row.get(3)?;
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, enabled != 0))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
        })
    }

    /// Inserts a Phase 0 rule.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn insert_phase0_rule(&self, app_match: &str, reminder_text: &str) -> Result<i64, DbError> {
        self.with_writer(|conn| {
            conn.execute(
                "INSERT INTO phase0_rules (app_match, reminder_text, enabled) VALUES (?1, ?2, 1)",
                rusqlite::params![app_match, reminder_text],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Returns recorded kill gates.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn kill_gates(&self) -> Result<Vec<KillGateRecord>, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare("SELECT id, status, notes FROM kill_gates ORDER BY id")?;
            let rows = stmt.query_map([], |row| {
                Ok(KillGateRecord {
                    id: row.get(0)?,
                    status: row.get(1)?,
                    notes: row.get(2)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
        })
    }

    fn with_writer<T>(
        &self,
        body: impl FnOnce(&Connection) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        let conn = self.writer.lock().map_err(|_| DbError::Poisoned)?;
        body(&conn)
    }
}

/// Applies WAL, busy timeout, and foreign keys on every connection.
///
/// # Errors
///
/// Returns [`DbError`] if a pragma cannot be set.
pub fn apply_pragmas(conn: &Connection) -> Result<(), DbError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5_000)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

/// Reads `PRAGMA user_version`.
///
/// # Errors
///
/// Returns [`DbError`] if the pragma cannot be queried.
pub fn user_version(conn: &Connection) -> Result<i32, DbError> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(DbError::from)
}

/// Applies numbered migrations that are newer than the current user version.
///
/// # Errors
///
/// Returns [`DbError::NewerSchema`] or [`DbError::Migration`] on failure.
pub fn migrate_with(conn: &Connection, migrations: &[Migration]) -> Result<(), DbError> {
    let mut current = user_version(conn)?;
    let Some(max) = migrations.iter().map(|item| item.version).max() else {
        return Ok(());
    };
    if current > max {
        return Err(DbError::NewerSchema {
            found: current,
            supported: max,
        });
    }
    for migration in migrations {
        if migration.version <= current {
            continue;
        }
        conn.execute("BEGIN IMMEDIATE", [])?;
        match conn.execute_batch(migration.sql) {
            Ok(()) => {
                conn.pragma_update(None, "user_version", migration.version)?;
                conn.execute("COMMIT", [])?;
                current = migration.version;
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK", []);
                return Err(DbError::Migration(error.to_string()));
            }
        }
    }
    Ok(())
}

fn probe_integrity(conn: &Connection) -> Result<(), DbError> {
    match conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)) {
        Ok(result) if result.eq_ignore_ascii_case("ok") => Ok(()),
        Ok(result) => Err(DbError::Corrupt(result)),
        Err(error) => Err(map_sqlite_error(error)),
    }
}

fn insert_capture(conn: &Connection, capture: &CaptureRecord) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO captures (
            capture_id, clause_ordinal, source_app, source_ctx, recipient, raw_message, sent_at, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(capture_id, clause_ordinal) DO NOTHING",
        rusqlite::params![
            capture.capture_id,
            capture.clause_ordinal,
            capture.source_app.as_str(),
            capture.source_ctx,
            capture.recipient,
            capture.raw_message,
            capture.sent_at,
            capture.created_at,
        ],
    )?;
    conn.query_row(
        "SELECT id FROM captures WHERE capture_id = ?1 AND clause_ordinal = ?2",
        rusqlite::params![capture.capture_id, capture.clause_ordinal],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

fn insert_promise_from_capture(
    conn: &Connection,
    capture_id: &str,
    clause_ordinal: i64,
    text: &str,
    score: i32,
    confidence: f64,
) -> Result<i64, DbError> {
    let status = PromiseStatus::from_score(score).as_str();
    let (source_app, source_ctx, recipient, raw_message, created_at): (
        String,
        Option<String>,
        Option<String>,
        String,
        i64,
    ) = conn.query_row(
        "SELECT source_app, source_ctx, recipient, raw_message, created_at
         FROM captures WHERE capture_id = ?1 AND clause_ordinal = ?2",
        rusqlite::params![capture_id, clause_ordinal],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    conn.execute(
        "INSERT INTO promises (
            capture_id, clause_ordinal, text, raw_message, source_app, source_ctx, recipient,
            confidence, score, status, ignore_count, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11)
         ON CONFLICT(capture_id, clause_ordinal) DO NOTHING",
        rusqlite::params![
            capture_id,
            clause_ordinal,
            text,
            raw_message,
            source_app,
            source_ctx,
            recipient,
            confidence,
            score,
            status,
            created_at,
        ],
    )?;
    conn.query_row(
        "SELECT id FROM promises WHERE capture_id = ?1 AND clause_ordinal = ?2",
        rusqlite::params![capture_id, clause_ordinal],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

fn insert_lease(conn: &Connection, lease: &SurfaceLease) -> Result<i64, DbError> {
    let now = now_unix();
    conn.execute(
        "INSERT INTO surface_attempts (
            promise_id, lease_token, action_token, state, expires_at, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            lease.promise_id,
            lease.lease_token,
            lease.action_token,
            lease.state.as_str(),
            lease.expires_at,
            now,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn now_unix() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(0)
}

fn validate_setting(key: &str, value: &str) -> Result<(), DbError> {
    match key {
        "daily_surface_cap" => {
            let parsed: u8 = value.parse().map_err(|_| invalid(key, "not an integer"))?;
            if (1..=3).contains(&parsed) {
                Ok(())
            } else {
                Err(invalid(key, "must be 1-3"))
            }
        }
        "min_gap_minutes" => {
            let parsed: u32 = value.parse().map_err(|_| invalid(key, "not an integer"))?;
            if parsed >= 90 {
                Ok(())
            } else {
                Err(invalid(key, "must be at least 90"))
            }
        }
        "quiet_hours_enabled" | "gmail_enabled" | "slack_enabled" | "autostart_enabled" => {
            if matches!(value, "true" | "false") {
                Ok(())
            } else {
                Err(invalid(key, "must be true or false"))
            }
        }
        "quiet_hours_start" | "quiet_hours_end" => {
            if value.is_empty() || is_hh_mm(value) {
                Ok(())
            } else {
                Err(invalid(key, "must be HH:MM"))
            }
        }
        "timezone"
        | "keyword_app_map"
        | "onboarding_completed_at"
        | "retention_days"
        | "global_shortcut"
        | "global_shortcut_fallback" => Ok(()),
        _ => Err(invalid(key, "unknown key")),
    }
}

fn is_hh_mm(value: &str) -> bool {
    let Some((hours, minutes)) = value.split_once(':') else {
        return false;
    };
    matches!(
        (hours.parse::<u8>(), minutes.parse::<u8>()),
        (Ok(0..=23), Ok(0..=59))
    )
}

fn invalid(key: &str, reason: &str) -> DbError {
    DbError::InvalidSetting {
        key: key.to_owned(),
        reason: reason.to_owned(),
    }
}

/// Helper used by later units when seeding kill-gate rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KillGateRecord {
    /// Stable gate identifier.
    pub id: String,
    /// `pending_user`, `passed`, or `failed`.
    pub status: String,
    /// Operator notes.
    pub notes: String,
}

/// Trigger row joined to a surfaceable promise.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceableRow {
    pub promise_id: i64,
    pub kind: String,
    pub match_value: String,
    pub priority: i32,
    pub deadline_ts: Option<i64>,
    pub confidence: f64,
    pub created_at: i64,
}
