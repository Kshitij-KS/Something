use crate::db::Database;
use crate::domain::{PromiseEvent, PromiseStatus, apply_promise};
use crate::extraction::{ExtractRequest, ExtractRoute, extract, skeleton};
use crate::triggers::{LinkInput, auto_link};
use serde::{Deserialize, Serialize};

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

/// Review action requested by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    Promote,
    Reject,
    Edit,
}

/// Applies a review action and, on reject, upserts a blocklist skeleton.
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
        ReviewAction::Promote => PromiseEvent::Promote,
        ReviewAction::Reject => PromiseEvent::Reject,
        ReviewAction::Edit => PromiseEvent::Promote,
    };
    let next =
        apply_promise(status, event).map_err(|error| crate::db::DbError::InvalidSetting {
            key: "promise_status".into(),
            reason: format!(
                "invalid transition from {:?} via {:?}",
                error.from, error.event
            ),
        })?;
    db.set_promise_status(promise_id, next, now)?;
    if action == ReviewAction::Reject {
        db.upsert_blocklist(&skeleton(text), now)?;
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
    source_app: crate::domain::SourceApp,
    source_ctx: Option<&str>,
    recipient: Option<&str>,
    raw_message: &str,
    sent_at: i64,
    now_utc: chrono::DateTime<chrono::Utc>,
    offset_seconds: i32,
    tz_label: &str,
) -> Result<usize, crate::db::DbError> {
    let blocklist = db.blocklist_patterns()?;
    let keyword_app_map = keyword_pairs(db);
    let clauses = extract(ExtractRequest {
        raw_message,
        now_utc,
        offset_seconds,
        tz_label,
        blocklist: &blocklist,
    });
    let mut stored = 0;
    for clause in clauses {
        if clause.route == ExtractRoute::Discard {
            continue;
        }
        let ordinal = i64::try_from(clause.ordinal).unwrap_or(0);
        db.insert_capture(&crate::domain::CaptureRecord {
            capture_id: capture_id.to_owned(),
            clause_ordinal: ordinal,
            source_app,
            source_ctx: source_ctx.map(ToOwned::to_owned),
            recipient: recipient.map(ToOwned::to_owned),
            raw_message: raw_message.to_owned(),
            sent_at,
            created_at: now_utc.timestamp(),
        })?;
        let promise_id = db.insert_extracted_promise(
            capture_id,
            ordinal,
            &clause.original,
            clause.score,
            f64::from(clause.score) / 10.0,
            clause.deadline.as_ref().map(|deadline| {
                (
                    deadline.utc_ts,
                    deadline.tz_label.clone(),
                    deadline.precision.as_str().to_owned(),
                )
            }),
        )?;
        if db.trigger_count(promise_id)? == 0 {
            for trigger in auto_link(LinkInput {
                source_app: source_app.as_str(),
                source_ctx,
                text: &clause.original,
                keyword_app_map: &keyword_app_map,
            }) {
                db.insert_trigger(
                    promise_id,
                    trigger.kind.as_str(),
                    &trigger.match_value,
                    trigger.priority,
                )?;
            }
        }
        stored += 1;
    }
    Ok(stored)
}

fn keyword_pairs(db: &Database) -> Vec<(String, String)> {
    db.get_setting("keyword_app_map")
        .ok()
        .flatten()
        .and_then(|raw| {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw).ok()
        })
        .map(|map| {
            map.into_iter()
                .filter_map(|(key, value)| value.as_str().map(|exe| (key, exe.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}
