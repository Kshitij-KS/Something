use crate::db::{Database, PreparedCapture, PreparedClause, PreparedTrigger};
use crate::domain::{PromiseEvent, PromiseStatus, SourceApp, apply_promise};
use crate::extraction::deadline::{DeadlineLexicon, parse_deadline};
use crate::extraction::{ExtractRequest, ExtractRoute, extract, skeleton};
use crate::triggers::{LinkInput, auto_link};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Review queue item shown in the desktop UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewItem {
    pub id: i64,
    pub text: String,
    pub source_app: String,
    pub recipient: Option<String>,
    pub score: i32,
    pub status: String,
}

/// Stores one user-confirmed quick capture without heuristic filtering.
///
/// # Errors
///
/// Returns a database error for empty text or persistence failures.
pub fn ingest_manual(
    db: &Database,
    capture_id: &str,
    text: &str,
    now: i64,
) -> Result<i64, crate::db::DbError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(crate::db::DbError::InvalidSetting {
            key: "quick_capture".into(),
            reason: "promise text cannot be empty".into(),
        });
    }
    let timezone = configured_timezone(db)?;
    let reference = chrono::DateTime::<chrono::Utc>::from_timestamp(now, 0).ok_or_else(|| {
        crate::db::DbError::InvalidSetting {
            key: "quick_capture".into(),
            reason: "capture timestamp is outside the supported range".into(),
        }
    })?;
    let deadline = parse_deadline(text, reference, 0, &timezone, &DeadlineLexicon::default()).map(
        |deadline| {
            (
                deadline.utc_ts,
                deadline.tz_label,
                deadline.precision.as_str().to_owned(),
            )
        },
    );
    let keyword_app_map = keyword_pairs(db)?;
    let links = auto_link(LinkInput {
        source_app: "manual",
        source_ctx: None,
        text,
        keyword_app_map: &keyword_app_map,
    })
    .into_iter()
    .map(|trigger| {
        (
            trigger.kind.as_str().to_owned(),
            trigger.match_value,
            trigger.priority,
        )
    })
    .collect::<Vec<_>>();
    let fingerprint = capture_fingerprint(SourceApp::Manual, None, None, text, now, &timezone);
    db.insert_manual_promise(
        capture_id,
        &fingerprint,
        text,
        now,
        &timezone,
        deadline,
        &links,
    )
}

/// Review action requested by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    Promote,
    Reject,
    Edit,
}

/// Applies a review action, creates triggers only after promotion, and learns rejections.
///
/// # Errors
///
/// Returns a database error when persistence fails.
pub fn apply_review(
    db: &Database,
    promise_id: i64,
    status: PromiseStatus,
    action: ReviewAction,
    text: &str,
    now: i64,
) -> Result<PromiseStatus, crate::db::DbError> {
    let event = match action {
        ReviewAction::Promote | ReviewAction::Edit => PromiseEvent::Promote,
        ReviewAction::Reject => PromiseEvent::Reject,
    };
    let next =
        apply_promise(status, event).map_err(|error| crate::db::DbError::InvalidSetting {
            key: "promise_status".into(),
            reason: format!(
                "invalid transition from {:?} via {:?}",
                error.from, error.event
            ),
        })?;

    if action == ReviewAction::Edit {
        db.update_review_text(promise_id, text)?;
    }
    db.set_promise_status(promise_id, next, now)?;

    match action {
        ReviewAction::Reject => db.upsert_blocklist(&skeleton(text), now)?,
        ReviewAction::Promote | ReviewAction::Edit => ensure_promotion_triggers(db, promise_id)?,
    }
    Ok(next)
}

/// Runs extraction and persists captureable / review clauses.
///
/// # Errors
///
/// Returns a database error when persistence fails.
pub fn ingest_message(
    db: &Database,
    capture_id: &str,
    source_app: SourceApp,
    source_ctx: Option<&str>,
    recipient: Option<&str>,
    raw_message: &str,
    sent_at: i64,
    committed_at: chrono::DateTime<chrono::Utc>,
    offset_seconds: i32,
    tz_label: &str,
) -> Result<usize, crate::db::DbError> {
    let deadline_reference = chrono::DateTime::<chrono::Utc>::from_timestamp(sent_at, 0)
        .ok_or_else(|| crate::db::DbError::InvalidSetting {
            key: "capture_timestamp".into(),
            reason: "capture timestamp is outside the supported range".into(),
        })?;
    let blocklist = db.blocklist_patterns()?;
    let keyword_app_map = keyword_pairs(db)?;
    let clauses = extract(ExtractRequest {
        raw_message,
        now_utc: deadline_reference,
        offset_seconds,
        tz_label,
        blocklist: &blocklist,
    });
    let prepared_clauses = clauses
        .into_iter()
        .filter(|clause| clause.route != ExtractRoute::Discard)
        .map(|clause| {
            let triggers = if clause.route == ExtractRoute::Capture {
                prepared_auto_links(
                    source_app.as_str(),
                    source_ctx,
                    &clause.original,
                    &keyword_app_map,
                )
            } else {
                Vec::new()
            };
            PreparedClause {
                ordinal: i64::try_from(clause.ordinal).unwrap_or(0),
                text: clause.original,
                score: clause.score,
                confidence: f64::from(clause.score) / 10.0,
                status: if clause.route == ExtractRoute::Capture {
                    PromiseStatus::Open
                } else {
                    PromiseStatus::Review
                },
                deadline: clause.deadline.map(|deadline| {
                    (
                        deadline.utc_ts,
                        deadline.tz_label,
                        deadline.precision.as_str().to_owned(),
                    )
                }),
                triggers,
            }
        })
        .collect::<Vec<_>>();
    let fingerprint = capture_fingerprint(
        source_app,
        source_ctx,
        recipient,
        raw_message,
        sent_at,
        tz_label,
    );
    let outcome = db.commit_prepared_capture(&PreparedCapture {
        capture_id: capture_id.to_owned(),
        payload_sha256: fingerprint,
        source_app,
        source_ctx: source_ctx.map(ToOwned::to_owned),
        recipient: recipient.map(ToOwned::to_owned),
        raw_message: raw_message.to_owned(),
        sent_at,
        created_at: committed_at.timestamp(),
        timezone: tz_label.to_owned(),
        clauses: prepared_clauses,
    })?;
    Ok(outcome.stored_clauses)
}

fn ensure_promotion_triggers(db: &Database, promise_id: i64) -> Result<(), crate::db::DbError> {
    let Some(record) = db.promise_link_record(promise_id)? else {
        return Err(crate::db::DbError::InvalidSetting {
            key: "promise_id".into(),
            reason: "review promise was not found".into(),
        });
    };
    let keyword_app_map = keyword_pairs(db)?;
    insert_auto_links(
        db,
        promise_id,
        &record.source_app,
        record.source_ctx.as_deref(),
        &record.text,
        &keyword_app_map,
    )
}

fn prepared_auto_links(
    source_app: &str,
    source_ctx: Option<&str>,
    text: &str,
    keyword_app_map: &[(String, String)],
) -> Vec<PreparedTrigger> {
    auto_link(LinkInput {
        source_app,
        source_ctx,
        text,
        keyword_app_map,
    })
    .into_iter()
    .map(|trigger| PreparedTrigger {
        kind: trigger.kind.as_str().to_owned(),
        match_value: trigger.match_value,
        priority: trigger.priority,
    })
    .collect()
}

fn insert_auto_links(
    db: &Database,
    promise_id: i64,
    source_app: &str,
    source_ctx: Option<&str>,
    text: &str,
    keyword_app_map: &[(String, String)],
) -> Result<(), crate::db::DbError> {
    for trigger in auto_link(LinkInput {
        source_app,
        source_ctx,
        text,
        keyword_app_map,
    }) {
        db.insert_trigger(
            promise_id,
            trigger.kind.as_str(),
            &trigger.match_value,
            trigger.priority,
        )?;
    }
    Ok(())
}

fn keyword_pairs(db: &Database) -> Result<Vec<(String, String)>, crate::db::DbError> {
    let Some(raw) = db.get_setting("keyword_app_map")? else {
        return Ok(Vec::new());
    };
    let map = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw).map_err(
        |error| crate::db::DbError::InvalidSetting {
            key: "keyword_app_map".into(),
            reason: error.to_string(),
        },
    )?;
    Ok(map
        .into_iter()
        .filter_map(|(key, value)| value.as_str().map(|exe| (key, exe.to_owned())))
        .collect())
}

fn configured_timezone(db: &Database) -> Result<String, crate::db::DbError> {
    let timezone = db
        .get_setting("timezone")?
        .unwrap_or_else(|| iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into()));
    timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| crate::db::DbError::InvalidSetting {
            key: "timezone".into(),
            reason: "must be a valid IANA timezone".into(),
        })?;
    Ok(timezone)
}

fn capture_fingerprint(
    source_app: SourceApp,
    source_ctx: Option<&str>,
    recipient: Option<&str>,
    raw_message: &str,
    sent_at: i64,
    _timezone: &str,
) -> String {
    let mut digest = Sha256::new();
    for value in [
        source_app.as_str(),
        source_ctx.unwrap_or(""),
        recipient.unwrap_or(""),
        raw_message,
    ] {
        let bytes = value.as_bytes();
        digest.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
        digest.update(bytes);
    }
    // A manual capture ID is the retry token for user-entered text. Its server-side
    // receipt must remain stable when an ambiguous IPC result is retried later.
    if !matches!(source_app, SourceApp::Manual) {
        digest.update(sent_at.to_be_bytes());
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}
