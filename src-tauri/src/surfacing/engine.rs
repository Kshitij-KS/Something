//! Extracted-promise surfacing on a completed focus dwell.

use crate::db::Database;
use crate::domain::{LeaseState, SurfaceLease};
use crate::platform::focus::FocusTarget;
use crate::platform::notifications::{NotificationRequest, NotificationSink};
use crate::surfacing::phase0::{Phase0Rule, notify_matched};
use crate::surfacing::rate_limit::{
    Eligibility, RateLimitConfig, RateLimitState, evaluate_candidate, local_day,
};
use crate::triggers::{Candidate, Trigger, TriggerKind, matching_priority, select_one};
use chrono::{DateTime, Duration, Utc};

/// What the dwell thread actually presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DwellAction {
    ExtractedShown {
        promise_id: i64,
        action_token: String,
    },
    Phase0Shown {
        rule_id: i64,
    },
    Suppressed,
    None,
}

/// Combines stored triggers with OS+extension focus, then surfaces at most one candidate.
///
/// # Errors
///
/// Returns a database error when matching, leasing, or settings lookup fails.
pub fn handle_dwell(
    db: &Database,
    sink: &dyn NotificationSink,
    target: &FocusTarget,
    now: DateTime<Utc>,
    phase0_rules: &[Phase0Rule],
) -> Result<DwellAction, crate::db::DbError> {
    if let Some(action) = surface_extracted(db, sink, target, now)? {
        return Ok(action);
    }
    if notify_matched(&target.app_id, phase0_rules, sink).unwrap_or(false) {
        if let Some(rule) = crate::surfacing::phase0::match_phase0(&target.app_id, phase0_rules) {
            return Ok(DwellAction::Phase0Shown { rule_id: rule.id });
        }
    }
    Ok(DwellAction::None)
}

fn surface_extracted(
    db: &Database,
    sink: &dyn NotificationSink,
    target: &FocusTarget,
    now: DateTime<Utc>,
) -> Result<Option<DwellAction>, crate::db::DbError> {
    let mut matched = matching_open_candidates(db, target, now.timestamp())?;
    let Some(winner) = select_one(&mut matched) else {
        return Ok(None);
    };
    let (config, state) = load_policy(db, now)?;
    let day = local_day(now, 0);
    let already = db.promise_shown_on_day(winner.promise_id, &day)?;
    match evaluate_candidate(now, &day, already, &config, &state) {
        Eligibility::Suppress(_) => Ok(Some(DwellAction::Suppressed)),
        Eligibility::Allow => {
            let text = db
                .promise_text(winner.promise_id)?
                .unwrap_or_else(|| "Callback reminder".into());
            let lease_token = uuid::Uuid::new_v4().to_string();
            let action_token = uuid::Uuid::new_v4().to_string();
            let mut lease = SurfaceLease::new(winner.promise_id, &lease_token, &action_token);
            lease.state = LeaseState::Leased;
            lease.expires_at = now.timestamp() + 15 * 60;
            db.insert_lease(lease)?;
            sink.show(&NotificationRequest {
                title: "Callback".into(),
                body: text,
                action_token: action_token.clone(),
            })
            .map_err(|error| crate::db::DbError::InvalidSetting {
                key: "notification".into(),
                reason: error.to_string(),
            })?;
            db.mark_lease_shown(&lease_token, now.timestamp(), &day)?;
            Ok(Some(DwellAction::ExtractedShown {
                promise_id: winner.promise_id,
                action_token,
            }))
        }
    }
}

fn matching_open_candidates(
    db: &Database,
    target: &FocusTarget,
    now_unix: i64,
) -> Result<Vec<Candidate>, crate::db::DbError> {
    let rows = db.list_surfaceable_rows(now_unix)?;
    let mut by_promise: Vec<Candidate> = Vec::new();
    for row in rows {
        let Some(kind) = TriggerKind::parse(&row.kind) else {
            continue;
        };
        let trigger = Trigger {
            kind,
            match_value: row.match_value,
            priority: row.priority,
        };
        let Some(priority) = matching_priority(target, &trigger) else {
            continue;
        };
        if let Some(existing) = by_promise
            .iter_mut()
            .find(|candidate| candidate.promise_id == row.promise_id)
        {
            if priority > existing.priority {
                existing.priority = priority;
            }
            continue;
        }
        by_promise.push(Candidate {
            promise_id: row.promise_id,
            priority,
            deadline_ts: row.deadline_ts,
            confidence: row.confidence,
            created_at: row.created_at,
        });
    }
    Ok(by_promise)
}

fn load_policy(
    db: &Database,
    now: DateTime<Utc>,
) -> Result<(RateLimitConfig, RateLimitState), crate::db::DbError> {
    let day = local_day(now, 0);
    let daily_cap = db
        .get_setting("daily_surface_cap")?
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let min_gap = db
        .get_setting("min_gap_minutes")?
        .and_then(|value| value.parse().ok())
        .unwrap_or(90);
    let quiet_enabled = db.get_setting("quiet_hours_enabled")? == Some("true".into());
    let quiet_start = if quiet_enabled {
        db.get_setting("quiet_hours_start")?
            .as_deref()
            .and_then(parse_hh_mm)
    } else {
        None
    };
    let quiet_end = if quiet_enabled {
        db.get_setting("quiet_hours_end")?
            .as_deref()
            .and_then(parse_hh_mm)
    } else {
        None
    };
    let silence_until = db
        .get_setting("onboarding_completed_at")?
        .and_then(|raw| raw.parse::<i64>().ok())
        .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0));
    let config = RateLimitConfig {
        daily_cap,
        min_gap: Duration::minutes(min_gap),
        quiet_start_minutes: quiet_start,
        quiet_end_minutes: quiet_end,
        silence_until,
    };
    let last_shown = db.last_shown_at()?;
    let last_surface_at = last_shown.and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0));
    let state = RateLimitState {
        surfaces_today: db.count_surfaces_on_day(&day)?,
        local_day: db.last_surface_day()?.unwrap_or_else(|| day.clone()),
        last_surface_at,
        last_observed_now: last_surface_at.unwrap_or(now),
        active_surface: db.has_active_surface(now.timestamp())?,
    };
    Ok((config, state))
}

fn parse_hh_mm(value: &str) -> Option<u32> {
    let (hours, minutes) = value.split_once(':')?;
    let hours: u32 = hours.parse().ok()?;
    let minutes: u32 = minutes.parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(hours * 60 + minutes)
}
