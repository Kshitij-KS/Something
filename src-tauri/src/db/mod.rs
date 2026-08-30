use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::{CaptureRecord, LeaseState, PromiseStatus, SurfaceLease};

const INITIAL_SCHEMA: &str = include_str!("../../migrations/0001_initial.sql");
const SELECTOR_HEALTH_ACTIVITY: &str =
    include_str!("../../migrations/0002_selector_health_activity.sql");
const CAPTURE_RECEIPTS_AND_ACTIONS: &str =
    include_str!("../../migrations/0003_capture_receipts_and_actions.sql");
const DURABLE_NOTIFICATION_ATTEMPTS: &str =
    include_str!("../../migrations/0004_durable_notification_attempts.sql");

/// Schema version applied by the bundled production migrations.
pub const CURRENT_SCHEMA_VERSION: i32 = 4;

/// A numbered SQL migration.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// Monotonic schema version applied by this script.
    pub version: i32,
    /// SQL executed inside a single transaction.
    pub sql: &'static str,
}

/// Production migrations applied to a Callback store.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: INITIAL_SCHEMA,
    },
    Migration {
        version: 2,
        sql: SELECTOR_HEALTH_ACTIVITY,
    },
    Migration {
        version: 3,
        sql: CAPTURE_RECEIPTS_AND_ACTIONS,
    },
    Migration {
        version: 4,
        sql: DURABLE_NOTIFICATION_ATTEMPTS,
    },
];

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

/// Counts from one retention-policy pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionReport {
    /// Oldest timestamp that remains unredacted.
    pub cutoff_at: i64,
    /// Expired captures removed with their resolved or review promises.
    pub deleted_captures: usize,
    /// Expired retry receipts removed after no retained capture depends on them.
    pub deleted_receipts: usize,
    /// Expired capture bodies redacted while preserving unresolved promises.
    pub redacted_captures: usize,
    /// Expired promise source bodies redacted while preserving reminder text.
    pub redacted_promises: usize,
}

/// One complete trigger link prepared before a capture transaction begins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTrigger {
    pub kind: String,
    pub match_value: String,
    pub priority: i32,
}

/// One persisted clause prepared by the deterministic extractor.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedClause {
    pub ordinal: i64,
    pub text: String,
    pub score: i32,
    pub confidence: f64,
    pub status: PromiseStatus,
    pub deadline: Option<(i64, String, String)>,
    pub triggers: Vec<PreparedTrigger>,
}

/// Immutable write set for one retry-safe capture id.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedCapture {
    pub capture_id: String,
    pub payload_sha256: String,
    pub source_app: crate::domain::SourceApp,
    pub source_ctx: Option<String>,
    pub recipient: Option<String>,
    pub raw_message: String,
    pub sent_at: i64,
    pub created_at: i64,
    pub timezone: String,
    pub clauses: Vec<PreparedClause>,
}

/// Result of committing a complete capture write set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureCommitOutcome {
    pub stored_clauses: usize,
    pub duplicate: bool,
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
    /// A retry reused a capture id for a different canonical payload.
    #[error("capture id {capture_id} conflicts with previously committed content")]
    CaptureConflict {
        /// Conflicting retry-safe identifier.
        capture_id: String,
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

    /// Atomically commits a full extracted capture, including a content-free
    /// retry receipt, clauses, deadlines, links, and selector health.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::CaptureConflict`] if a capture id is reused with a
    /// different canonical payload, or another [`DbError`] on SQLite failure.
    pub fn commit_prepared_capture(
        &self,
        prepared: &PreparedCapture,
    ) -> Result<CaptureCommitOutcome, DbError> {
        if prepared.payload_sha256.len() != 64
            || !prepared
                .payload_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(invalid(
                "capture_fingerprint",
                "must be a lowercase SHA-256 hex digest",
            ));
        }
        self.with_writer(|conn| {
            let transaction = conn.unchecked_transaction()?;
            let existing = {
                let mut statement = transaction.prepare(
                    "SELECT payload_sha256, stored_clauses
                     FROM capture_receipts WHERE capture_id = ?1",
                )?;
                let mut rows = statement.query([&prepared.capture_id])?;
                match rows.next()? {
                    Some(row) => Some((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    None => None,
                }
            };
            if let Some((fingerprint, stored_clauses)) = existing {
                if fingerprint != prepared.payload_sha256 {
                    return Err(DbError::CaptureConflict {
                        capture_id: prepared.capture_id.clone(),
                    });
                }
                return Ok(CaptureCommitOutcome {
                    stored_clauses: usize::try_from(stored_clauses).unwrap_or(0),
                    duplicate: true,
                });
            }

            // Databases upgraded from schema v2 have captures but no receipts.
            // Reconcile an exact legacy retry once, while rejecting reuse of the
            // same identifier for different content.
            let legacy = {
                let mut statement = transaction.prepare(
                    "SELECT source_app, source_ctx, recipient, raw_message, sent_at, created_at,
                            (SELECT COUNT(*) FROM captures WHERE capture_id = ?1)
                     FROM captures
                     WHERE capture_id = ?1
                     ORDER BY clause_ordinal
                     LIMIT 1",
                )?;
                let mut rows = statement.query([&prepared.capture_id])?;
                match rows.next()? {
                    Some(row) => Some((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    )),
                    None => None,
                }
            };
            if let Some((
                source_app,
                source_ctx,
                recipient,
                raw_message,
                sent_at,
                created_at,
                stored_clauses,
            )) = legacy
            {
                if source_app != prepared.source_app.as_str()
                    || source_ctx.as_deref() != prepared.source_ctx.as_deref()
                    || recipient.as_deref() != prepared.recipient.as_deref()
                    || raw_message != prepared.raw_message
                    || sent_at != prepared.sent_at
                {
                    return Err(DbError::CaptureConflict {
                        capture_id: prepared.capture_id.clone(),
                    });
                }
                transaction.execute(
                    "INSERT INTO capture_receipts (
                        capture_id, payload_sha256, source_app, sent_at, timezone,
                        stored_clauses, committed_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        prepared.capture_id,
                        prepared.payload_sha256,
                        prepared.source_app.as_str(),
                        prepared.sent_at,
                        prepared.timezone,
                        stored_clauses,
                        created_at,
                    ],
                )?;
                transaction.commit()?;
                return Ok(CaptureCommitOutcome {
                    stored_clauses: usize::try_from(stored_clauses).unwrap_or(0),
                    duplicate: true,
                });
            }

            transaction.execute(
                "INSERT INTO capture_receipts (
                    capture_id, payload_sha256, source_app, sent_at, timezone,
                    stored_clauses, committed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                rusqlite::params![
                    prepared.capture_id,
                    prepared.payload_sha256,
                    prepared.source_app.as_str(),
                    prepared.sent_at,
                    prepared.timezone,
                    prepared.created_at,
                ],
            )?;

            for clause in &prepared.clauses {
                insert_capture(
                    &transaction,
                    &CaptureRecord {
                        capture_id: prepared.capture_id.clone(),
                        clause_ordinal: clause.ordinal,
                        source_app: prepared.source_app,
                        source_ctx: prepared.source_ctx.clone(),
                        recipient: prepared.recipient.clone(),
                        raw_message: prepared.raw_message.clone(),
                        sent_at: prepared.sent_at,
                        created_at: prepared.created_at,
                    },
                )?;
                let promise_id = insert_promise_from_capture(
                    &transaction,
                    &prepared.capture_id,
                    clause.ordinal,
                    &clause.text,
                    clause.score,
                    clause.confidence,
                )?;
                transaction.execute(
                    "UPDATE promises
                     SET status = ?1, deadline = ?2, deadline_tz = ?3,
                         deadline_precision = ?4
                     WHERE id = ?5",
                    rusqlite::params![
                        clause.status.as_str(),
                        clause.deadline.as_ref().map(|deadline| deadline.0),
                        clause.deadline.as_ref().map(|deadline| &deadline.1),
                        clause.deadline.as_ref().map(|deadline| &deadline.2),
                        promise_id,
                    ],
                )?;
                for trigger in &clause.triggers {
                    transaction.execute(
                        "INSERT INTO triggers (promise_id, kind, match_value, priority)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(promise_id, kind, match_value)
                         DO UPDATE SET priority = excluded.priority",
                        rusqlite::params![
                            promise_id,
                            trigger.kind,
                            trigger.match_value,
                            trigger.priority,
                        ],
                    )?;
                }
            }

            if matches!(
                prepared.source_app,
                crate::domain::SourceApp::Gmail | crate::domain::SourceApp::Slack
            ) {
                record_selector_capture_on(
                    &transaction,
                    prepared.source_app.as_str(),
                    prepared.sent_at,
                )?;
            }
            let stored_clauses = prepared.clauses.len();
            let stored_clauses_i64 = i64::try_from(stored_clauses).unwrap_or(i64::MAX);
            transaction.execute(
                "UPDATE capture_receipts SET stored_clauses = ?2 WHERE capture_id = ?1",
                rusqlite::params![prepared.capture_id, stored_clauses_i64],
            )?;
            transaction.commit()?;
            Ok(CaptureCommitOutcome {
                stored_clauses,
                duplicate: false,
            })
        })
    }

    /// Atomically stores one explicit manual promise and its trigger links.
    /// Reusing `capture_id` returns the existing promise without duplicate links.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on validation or SQLite failures.
    pub fn insert_manual_promise(
        &self,
        capture_id: &str,
        payload_sha256: &str,
        text: &str,
        created_at: i64,
        timezone: &str,
        deadline: Option<(i64, String, String)>,
        links: &[(String, String, i32)],
    ) -> Result<i64, DbError> {
        if text.trim().is_empty() {
            return Err(invalid("quick_capture", "promise text cannot be empty"));
        }
        let prepared = PreparedCapture {
            capture_id: capture_id.to_owned(),
            payload_sha256: payload_sha256.to_owned(),
            source_app: crate::domain::SourceApp::Manual,
            source_ctx: None,
            recipient: None,
            raw_message: text.to_owned(),
            sent_at: created_at,
            created_at,
            timezone: timezone.to_owned(),
            clauses: vec![PreparedClause {
                ordinal: 0,
                text: text.to_owned(),
                score: 10,
                confidence: 1.0,
                status: PromiseStatus::Open,
                deadline,
                triggers: links
                    .iter()
                    .map(|(kind, match_value, priority)| PreparedTrigger {
                        kind: kind.clone(),
                        match_value: match_value.clone(),
                        priority: *priority,
                    })
                    .collect(),
            }],
        };
        self.commit_prepared_capture(&prepared)?;
        self.with_writer(|conn| {
            conn.query_row(
                "SELECT id FROM promises WHERE capture_id = ?1 AND clause_ordinal = 0",
                [capture_id],
                |row| row.get(0),
            )
            .map_err(DbError::from)
        })
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
                "INSERT INTO triggers (promise_id, kind, match_value, priority)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(promise_id, kind, match_value)
                 DO UPDATE SET priority = excluded.priority",
                rusqlite::params![promise_id, kind, match_value, priority],
            )?;
            conn.query_row(
                "SELECT id FROM triggers
                 WHERE promise_id = ?1 AND kind = ?2 AND match_value = ?3",
                rusqlite::params![promise_id, kind, match_value],
                |row| row.get(0),
            )
            .map_err(DbError::from)
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

    /// Creates one leased surface and its pending notification record atomically.
    pub fn begin_notification_attempt(
        &self,
        promise_id: i64,
        lease_token: &str,
        action_token: &str,
        created_at: i64,
        expires_at: i64,
    ) -> Result<i64, DbError> {
        self.with_writer(|conn| {
            let transaction = conn.unchecked_transaction()?;
            let status = {
                let mut statement =
                    transaction.prepare("SELECT status FROM promises WHERE id = ?1")?;
                let mut rows = statement.query([promise_id])?;
                match rows.next()? {
                    Some(row) => Some(row.get::<_, String>(0)?),
                    None => None,
                }
            };
            if status.as_deref() != Some("open") {
                return Err(invalid("surface", "promise is no longer open"));
            }
            let active: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM surface_attempts
                 WHERE state IN ('leased', 'shown')
                   AND expires_at > ?1
                   AND action IS NULL",
                [created_at],
                |row| row.get(0),
            )?;
            if active != 0 {
                return Err(invalid("surface", "another notification is active"));
            }
            transaction.execute(
                "INSERT INTO surface_attempts (
                    promise_id, lease_token, action_token, state, expires_at, created_at
                 ) VALUES (?1, ?2, ?3, 'leased', ?4, ?5)",
                rusqlite::params![
                    promise_id,
                    lease_token,
                    action_token,
                    expires_at,
                    created_at,
                ],
            )?;
            let surface_attempt_id = transaction.last_insert_rowid();
            transaction.execute(
                "INSERT INTO notification_attempts (
                    surface_attempt_id, delivered, error, created_at
                 ) VALUES (?1, 0, NULL, ?2)",
                rusqlite::params![surface_attempt_id, created_at],
            )?;
            transaction.commit()?;
            Ok(surface_attempt_id)
        })
    }

    /// Records successful OS delivery and marks the lease shown atomically.
    pub fn finish_notification_delivered(
        &self,
        surface_attempt_id: i64,
        shown_at: i64,
        local_day: &str,
        deadline_escalation: bool,
    ) -> Result<(), DbError> {
        self.with_writer(|conn| {
            let transaction = conn.unchecked_transaction()?;
            let notification_changed = transaction.execute(
                "UPDATE notification_attempts
                 SET delivered = 1, error = NULL
                 WHERE surface_attempt_id = ?1 AND delivered = 0 AND error IS NULL",
                [surface_attempt_id],
            )?;
            let surface_changed = transaction.execute(
                "UPDATE surface_attempts
                 SET state = 'shown', shown_at = ?2, local_day = ?3
                 WHERE id = ?1 AND state = 'leased' AND action IS NULL",
                rusqlite::params![surface_attempt_id, shown_at, local_day],
            )?;
            if notification_changed != 1 || surface_changed != 1 {
                return Err(invalid(
                    "notification",
                    "delivery result no longer matches a pending lease",
                ));
            }
            if deadline_escalation {
                transaction.execute(
                    "UPDATE promises
                     SET deadline_escalated_at = ?2
                     WHERE id = (
                         SELECT promise_id FROM surface_attempts WHERE id = ?1
                     ) AND deadline_escalated_at IS NULL",
                    rusqlite::params![surface_attempt_id, shown_at],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
    }

    /// Records failed OS delivery and immediately releases the lease.
    pub fn finish_notification_failed(
        &self,
        surface_attempt_id: i64,
        failed_at: i64,
        error: &str,
    ) -> Result<(), DbError> {
        let bounded_error = error.chars().take(512).collect::<String>();
        self.with_writer(|conn| {
            let transaction = conn.unchecked_transaction()?;
            let notification_changed = transaction.execute(
                "UPDATE notification_attempts
                 SET delivered = 0, error = ?2
                 WHERE surface_attempt_id = ?1 AND delivered = 0 AND error IS NULL",
                rusqlite::params![surface_attempt_id, bounded_error],
            )?;
            let surface_changed = transaction.execute(
                "UPDATE surface_attempts
                 SET state = 'expired', expires_at = ?2
                 WHERE id = ?1 AND state = 'leased' AND action IS NULL",
                rusqlite::params![surface_attempt_id, failed_at],
            )?;
            if notification_changed != 1 || surface_changed != 1 {
                return Err(invalid(
                    "notification",
                    "failure result no longer matches a pending lease",
                ));
            }
            transaction.commit()?;
            Ok(())
        })
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
            let transaction = conn.unchecked_transaction()?;
            transaction.execute(
                "UPDATE notification_attempts
                 SET error = 'delivery result not recorded before lease expiry'
                 WHERE delivered = 0 AND error IS NULL
                   AND surface_attempt_id IN (
                     SELECT id FROM surface_attempts
                     WHERE state IN ('leased', 'shown')
                       AND expires_at <= ?1
                       AND action IS NULL
                   )",
                [now_unix],
            )?;
            let changed = transaction.execute(
                "UPDATE surface_attempts
                 SET state = 'expired'
                 WHERE state IN ('leased', 'shown')
                   AND expires_at <= ?1
                   AND action IS NULL",
                [now_unix],
            )?;
            transaction.commit()?;
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

    /// Open promises joined to their context triggers.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on SQLite failures.
    pub fn list_surfaceable_rows(&self, _now_unix: i64) -> Result<Vec<SurfaceableRow>, DbError> {
        self.with_writer(|conn| {
            let mut stmt = conn.prepare(
                "SELECT p.id, t.kind, t.match_value, t.priority, p.deadline, p.confidence, p.created_at
                 FROM promises p
                 JOIN triggers t ON t.promise_id = p.id
                 WHERE p.status = 'open' AND p.snooze_until IS NULL",
            )?;
            let rows = stmt.query_map([], |row| {
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

    /// Reopens due snoozes while retaining a marker that requires a fresh dwell.
    pub fn reopen_due_snoozes(&self, now_unix: i64) -> Result<u64, DbError> {
        self.with_writer(|conn| {
            let changed = conn.execute(
                "UPDATE promises
                 SET status = 'open'
                 WHERE status = 'snoozed'
                   AND snooze_until IS NOT NULL
                   AND snooze_until <= ?1",
                [now_unix],
            )?;
            Ok(changed as u64)
        })
    }

    /// Clears reopened-snooze markers at the start of a newly completed dwell.
    pub fn clear_reopened_snooze_markers(&self, now_unix: i64) -> Result<u64, DbError> {
        self.with_writer(|conn| {
            let changed = conn.execute(
                "UPDATE promises
                 SET snooze_until = NULL
                 WHERE status = 'open'
                   AND snooze_until IS NOT NULL
                   AND snooze_until <= ?1",
                [now_unix],
            )?;
            Ok(changed as u64)
        })
    }

    /// Lists due deadlines eligible for their single fallback escalation.
    pub fn list_due_deadline_candidates(
        &self,
        now_unix: i64,
    ) -> Result<Vec<DeadlineSurfaceRow>, DbError> {
        self.with_writer(|conn| {
            let mut statement = conn.prepare(
                "SELECT p.id, p.deadline, p.confidence, p.created_at
                 FROM promises p
                 WHERE p.status = 'open'
                   AND p.snooze_until IS NULL
                   AND p.deadline IS NOT NULL
                   AND p.deadline <= ?1
                   AND p.deadline_escalated_at IS NULL
                   AND NOT EXISTS (
                     SELECT 1 FROM surface_attempts s
                     WHERE s.promise_id = p.id AND s.shown_at IS NOT NULL
                   )",
            )?;
            let rows = statement.query_map([now_unix], |row| {
                Ok(DeadlineSurfaceRow {
                    promise_id: row.get(0)?,
                    deadline_ts: row.get(1)?,
                    confidence: row.get(2)?,
                    created_at: row.get(3)?,
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

    /// Redeems a notification action token exactly once with its promise update.
    pub fn redeem_surface_action(
        &self,
        action_token: &str,
        action: crate::surfacing::actions::SurfaceAction,
        now_unix: i64,
        local_day: &str,
        snooze_until: Option<i64>,
    ) -> Result<crate::surfacing::actions::ActionResult, DbError> {
        use crate::surfacing::actions::{ActionResult, SurfaceAction};

        match (action, snooze_until) {
            (SurfaceAction::Snooze, Some(until)) if until > now_unix => {}
            (SurfaceAction::Snooze, _) => {
                return Err(invalid("snooze", "snooze time must be in the future"));
            }
            (_, None) => {}
            (_, Some(_)) => {
                return Err(invalid(
                    "snooze",
                    "only a snooze action may include a snooze time",
                ));
            }
        }

        self.with_writer(|conn| {
            let transaction = conn.unchecked_transaction()?;
            let record = {
                let mut statement = transaction.prepare(
                    "SELECT s.id, s.state, s.action, s.expires_at,
                            p.id, p.status, p.ignore_count, p.text
                     FROM surface_attempts s
                     JOIN promises p ON p.id = s.promise_id
                     WHERE s.action_token = ?1",
                )?;
                let mut rows = statement.query([action_token])?;
                match rows.next()? {
                    Some(row) => Some((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                    )),
                    None => None,
                }
            };
            let Some((
                surface_id,
                surface_state,
                stored_action,
                expires_at,
                promise_id,
                promise_status,
                ignore_count,
                promise_text,
            )) = record
            else {
                return Ok(ActionResult::UnknownToken);
            };
            if stored_action.is_some() || surface_state == "acted" {
                return Ok(ActionResult::Duplicate);
            }
            if !matches!(surface_state.as_str(), "leased" | "shown") || expires_at <= now_unix {
                return Ok(ActionResult::Late);
            }
            let status = PromiseStatus::parse(&promise_status).map_err(|reason| {
                invalid(
                    "promise_status",
                    &format!("stored status is invalid: {reason}"),
                )
            })?;
            let ignore_count_u32 = u32::try_from(ignore_count).unwrap_or(u32::MAX);
            let event = action.event(ignore_count_u32);
            let Ok(next) = crate::domain::apply_promise(status, event) else {
                return Ok(ActionResult::Late);
            };

            transaction.execute(
                "UPDATE notification_attempts
                 SET delivered = 1, error = NULL
                 WHERE surface_attempt_id = ?1 AND delivered = 0 AND error IS NULL",
                [surface_id],
            )?;
            transaction.execute(
                "UPDATE surface_attempts
                 SET state = 'shown', shown_at = COALESCE(shown_at, ?2),
                     local_day = COALESCE(local_day, ?3)
                 WHERE id = ?1 AND state = 'leased' AND action IS NULL",
                rusqlite::params![surface_id, now_unix, local_day],
            )?;
            let acted = transaction.execute(
                "UPDATE surface_attempts
                 SET state = 'acted', acted_at = ?2, action = ?3,
                     shown_at = COALESCE(shown_at, ?2),
                     local_day = COALESCE(local_day, ?4)
                 WHERE id = ?1 AND state = 'shown' AND action IS NULL
                   AND expires_at > ?2",
                rusqlite::params![surface_id, now_unix, action.db_value(), local_day],
            )?;
            if acted != 1 {
                return Ok(ActionResult::Late);
            }

            let next_ignore_count = if matches!(action, SurfaceAction::Ignore) {
                ignore_count.saturating_add(1)
            } else {
                ignore_count
            };
            let terminal = matches!(
                next,
                PromiseStatus::Done | PromiseStatus::Dismissed | PromiseStatus::Archived
            );
            let changed = transaction.execute(
                "UPDATE promises
                 SET status = ?2, ignore_count = ?3, snooze_until = ?4,
                     resolved_at = CASE WHEN ?5 THEN ?6 ELSE NULL END
                 WHERE id = ?1 AND status = 'open'",
                rusqlite::params![
                    promise_id,
                    next.as_str(),
                    next_ignore_count,
                    snooze_until,
                    terminal,
                    now_unix,
                ],
            )?;
            if changed != 1 {
                return Ok(ActionResult::Late);
            }
            if matches!(action, SurfaceAction::Reject) {
                let pattern = crate::extraction::skeleton(&promise_text);
                transaction.execute(
                    "INSERT INTO blocklist (pattern, hits, created_at) VALUES (?1, 1, ?2)
                     ON CONFLICT(pattern) DO UPDATE SET hits = hits + 1",
                    rusqlite::params![pattern, now_unix],
                )?;
            }
            transaction.commit()?;
            Ok(ActionResult::Applied { next, event })
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

    /// Returns whether capture/context processing is enabled for a web source.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] for an unknown site or a settings read failure.
    pub fn site_enabled(&self, site: &str) -> Result<bool, DbError> {
        let key = match site {
            "gmail" => "gmail_enabled",
            "slack" => "slack_enabled",
            _ => return Err(invalid("site", "site must be gmail or slack")),
        };
        Ok(self.get_setting(key)?.as_deref() != Some("false"))
    }

    /// Returns the two-site policy snapshot sent over local native messaging.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] on settings read failures.
    pub fn site_policy(&self) -> Result<(bool, bool), DbError> {
        Ok((self.site_enabled("gmail")?, self.site_enabled("slack")?))
    }

    /// Enforces the configured retention window in one transaction.
    ///
    /// Expired review and terminal promises are removed through their capture.
    /// Open and snoozed promises remain actionable, but their duplicate raw
    /// source bodies are replaced with an empty string. Old retry receipts are
    /// removed only after no retained capture depends on them.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] for an invalid policy or SQLite failure.
    pub fn enforce_retention(&self, now: i64) -> Result<RetentionReport, DbError> {
        let raw_days = self
            .get_setting("retention_days")?
            .unwrap_or_else(|| "365".to_owned());
        let days = parse_retention_days(&raw_days)?;
        let cutoff_at = now.saturating_sub(days.saturating_mul(86_400));

        self.with_writer(|conn| {
            let transaction = conn.unchecked_transaction()?;
            let redacted_promises = transaction.execute(
                "UPDATE promises
                 SET raw_message = ''
                 WHERE created_at < ?1
                   AND status IN ('open', 'snoozed')
                   AND raw_message <> ''",
                [cutoff_at],
            )?;
            let redacted_captures = transaction.execute(
                "UPDATE captures
                 SET raw_message = ''
                 WHERE created_at < ?1
                   AND raw_message <> ''
                   AND EXISTS (
                     SELECT 1 FROM promises
                     WHERE promises.capture_id = captures.capture_id
                       AND promises.clause_ordinal = captures.clause_ordinal
                       AND promises.status IN ('open', 'snoozed')
                   )",
                [cutoff_at],
            )?;
            let deleted_captures = transaction.execute(
                "DELETE FROM captures
                 WHERE created_at < ?1
                   AND NOT EXISTS (
                     SELECT 1 FROM promises
                     WHERE promises.capture_id = captures.capture_id
                       AND promises.clause_ordinal = captures.clause_ordinal
                       AND promises.status IN ('open', 'snoozed')
                   )",
                [cutoff_at],
            )?;
            let deleted_receipts = transaction.execute(
                "DELETE FROM capture_receipts
                 WHERE committed_at < ?1
                   AND NOT EXISTS (
                     SELECT 1 FROM captures
                     WHERE captures.capture_id = capture_receipts.capture_id
                   )",
                [cutoff_at],
            )?;
            transaction.commit()?;
            Ok(RetentionReport {
                cutoff_at,
                deleted_captures,
                deleted_receipts,
                redacted_captures,
                redacted_promises,
            })
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

/// Validates one persisted setting value without mutating the database.
///
/// # Errors
///
/// Returns [`DbError::InvalidSetting`] for unknown keys or invalid values.
pub fn validate_setting(key: &str, value: &str) -> Result<(), DbError> {
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
        "retention_days" => parse_retention_days(value).map(|_| ()),
        "timezone" => value
            .parse::<chrono_tz::Tz>()
            .map(|_| ())
            .map_err(|_| invalid(key, "must be an IANA timezone such as America/New_York")),
        "keyword_app_map"
        | "onboarding_completed_at"
        | "global_shortcut"
        | "global_shortcut_fallback" => Ok(()),
        _ => Err(invalid(key, "unknown key")),
    }
}

fn parse_retention_days(value: &str) -> Result<i64, DbError> {
    let parsed: i64 = value
        .parse()
        .map_err(|_| invalid("retention_days", "not an integer"))?;
    if (1..=3_650).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(invalid("retention_days", "must be between 1 and 3650"))
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

/// Open promise whose deadline may receive one fallback escalation.
#[derive(Debug, Clone, PartialEq)]
pub struct DeadlineSurfaceRow {
    pub promise_id: i64,
    pub deadline_ts: i64,
    pub confidence: f64,
    pub created_at: i64,
}

/// Source fields needed to create triggers when a reviewed promise is promoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromiseLinkRecord {
    pub source_app: String,
    pub source_ctx: Option<String>,
    pub text: String,
}

impl Database {
    /// Returns whether a named human-evidence gate has passed.
    pub fn kill_gate_passed(&self, id: &str) -> Result<bool, DbError> {
        self.with_writer(|conn| gate_is_passed(conn, id))
    }

    /// Records a kill-gate decision while enforcing the plan's prerequisite order.
    pub fn update_kill_gate(&self, id: &str, status: &str, notes: &str) -> Result<(), DbError> {
        const IDS: [&str; 3] = [
            "phase0_five_day",
            "extraction_precision_300",
            "acceptance_two_week",
        ];
        if !IDS.contains(&id) {
            return Err(invalid("kill_gate", "unknown gate"));
        }
        if !matches!(status, "pending_user" | "passed" | "failed") {
            return Err(invalid("kill_gate", "invalid status"));
        }
        let notes = notes.trim();
        if status != "pending_user" && notes.len() < 8 {
            return Err(invalid(
                "kill_gate",
                "passed or failed gates require evidence notes",
            ));
        }

        self.with_writer(|conn| {
            if status == "passed" {
                let prerequisite = match id {
                    "extraction_precision_300" => Some("phase0_five_day"),
                    "acceptance_two_week" => Some("extraction_precision_300"),
                    _ => None,
                };
                if let Some(required) = prerequisite {
                    if !gate_is_passed(conn, required)? {
                        return Err(invalid(
                            "kill_gate",
                            "the preceding plan gate has not passed",
                        ));
                    }
                }
            } else {
                let dependent = match id {
                    "phase0_five_day" => Some("extraction_precision_300"),
                    "extraction_precision_300" => Some("acceptance_two_week"),
                    _ => None,
                };
                if let Some(next) = dependent {
                    if gate_is_passed(conn, next)? {
                        return Err(invalid(
                            "kill_gate",
                            "a passed downstream gate must be reset first",
                        ));
                    }
                }
            }

            let changed = conn.execute(
                "UPDATE kill_gates SET status = ?1, notes = ?2 WHERE id = ?3",
                rusqlite::params![status, notes, id],
            )?;
            if changed != 1 {
                return Err(invalid("kill_gate", "gate record is missing"));
            }
            Ok(())
        })
    }

    /// Returns source fields for trigger creation after review promotion.
    pub fn promise_link_record(
        &self,
        promise_id: i64,
    ) -> Result<Option<PromiseLinkRecord>, DbError> {
        self.with_writer(|conn| {
            match conn.query_row(
                "SELECT source_app, source_ctx, text FROM promises WHERE id = ?1",
                [promise_id],
                |row| {
                    Ok(PromiseLinkRecord {
                        source_app: row.get(0)?,
                        source_ctx: row.get(1)?,
                        text: row.get(2)?,
                    })
                },
            ) {
                Ok(record) => Ok(Some(record)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(error) => Err(DbError::from(error)),
            }
        })
    }

    /// Updates text while an item is still in review.
    pub fn update_review_text(&self, promise_id: i64, text: &str) -> Result<(), DbError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(invalid("review_text", "text cannot be empty"));
        }
        self.with_writer(|conn| {
            let changed = conn.execute(
                "UPDATE promises SET text = ?1 WHERE id = ?2 AND status = 'review'",
                rusqlite::params![text, promise_id],
            )?;
            if changed != 1 {
                return Err(invalid("review_text", "review promise was not found"));
            }
            Ok(())
        })
    }
}

fn gate_is_passed(conn: &Connection, id: &str) -> Result<bool, DbError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM kill_gates WHERE id = ?1 AND status = 'passed'",
        [id],
        |row| row.get(0),
    )?;
    Ok(count == 1)
}

/// Persisted content-free selector diagnostics for one capture site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorHealthRecord {
    pub site: String,
    pub first_observed_at: Option<i64>,
    pub last_probe_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub consecutive_failures: u32,
    pub last_capture_at: Option<i64>,
    pub state: String,
}

impl Database {
    /// Records a content-free selector probe and advances durable health state.
    pub fn record_selector_probe(
        &self,
        site: &str,
        succeeded: bool,
        observed_at: i64,
    ) -> Result<(), DbError> {
        validate_selector_observation(site, observed_at)?;
        self.with_writer(|conn| {
            conn.execute(
                "INSERT INTO selector_health (
                    site, first_observed_at, last_probe_at, last_success_at,
                    consecutive_failures, state
                 ) VALUES (
                    ?1, ?2, ?2, CASE WHEN ?3 THEN ?2 END,
                    CASE WHEN ?3 THEN 0 ELSE 1 END,
                    CASE WHEN ?3 THEN 'healthy' ELSE 'degraded' END
                 )
                 ON CONFLICT(site) DO UPDATE SET
                    first_observed_at = COALESCE(selector_health.first_observed_at, excluded.last_probe_at),
                    last_probe_at = excluded.last_probe_at,
                    last_success_at = CASE
                        WHEN ?3 THEN excluded.last_probe_at
                        ELSE selector_health.last_success_at
                    END,
                    consecutive_failures = CASE
                        WHEN ?3 THEN 0
                        ELSE selector_health.consecutive_failures + 1
                    END,
                    state = CASE
                        WHEN ?3 THEN 'healthy'
                        WHEN selector_health.consecutive_failures + 1 >= 3 THEN 'broken'
                        ELSE 'degraded'
                    END",
                rusqlite::params![site, observed_at, succeeded],
            )?;
            Ok(())
        })
    }

    /// Records a confirmed capture as proof that a site's selectors are operational.
    pub fn record_selector_capture(&self, site: &str, captured_at: i64) -> Result<(), DbError> {
        validate_selector_observation(site, captured_at)?;
        self.with_writer(|conn| record_selector_capture_on(conn, site, captured_at))
    }

    /// Lists selector diagnostics without any captured content.
    pub fn selector_health(&self) -> Result<Vec<SelectorHealthRecord>, DbError> {
        self.with_writer(|conn| {
            let mut statement = conn.prepare(
                "SELECT site, first_observed_at, last_probe_at, last_success_at,
                        consecutive_failures, last_capture_at, state
                 FROM selector_health ORDER BY site",
            )?;
            let rows = statement.query_map([], |row| {
                Ok(SelectorHealthRecord {
                    site: row.get(0)?,
                    first_observed_at: row.get(1)?,
                    last_probe_at: row.get(2)?,
                    last_success_at: row.get(3)?,
                    consecutive_failures: row.get(4)?,
                    last_capture_at: row.get(5)?,
                    state: row.get(6)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
        })
    }
}

fn record_selector_capture_on(
    conn: &Connection,
    site: &str,
    captured_at: i64,
) -> Result<(), DbError> {
    validate_selector_observation(site, captured_at)?;
    conn.execute(
        "INSERT INTO selector_health (
            site, first_observed_at, last_success_at, consecutive_failures,
            last_capture_at, state
         ) VALUES (?1, ?2, ?2, 0, ?2, 'healthy')
         ON CONFLICT(site) DO UPDATE SET
            first_observed_at = COALESCE(selector_health.first_observed_at, excluded.last_capture_at),
            last_success_at = excluded.last_capture_at,
            consecutive_failures = 0,
            last_capture_at = excluded.last_capture_at,
            state = 'healthy'",
        rusqlite::params![site, captured_at],
    )?;
    Ok(())
}

fn validate_selector_observation(site: &str, observed_at: i64) -> Result<(), DbError> {
    if !matches!(site, "gmail" | "slack") {
        return Err(invalid("selector_site", "site must be gmail or slack"));
    }
    if observed_at <= 0 {
        return Err(invalid(
            "selector_observed_at",
            "timestamp must be positive",
        ));
    }
    Ok(())
}
