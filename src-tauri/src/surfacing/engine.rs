//! Extracted-promise surfacing on completed focus dwell and maintenance ticks.

use crate::db::{Database, DbError};
use crate::platform::focus::FocusTarget;
use crate::platform::notifications::{NotificationRequest, NotificationSink, NotifyError};
use crate::surfacing::phase0::{Phase0Rule, match_phase0, notify_matched};
use crate::surfacing::rate_limit::{
    Eligibility, RateLimitConfig, RateLimitState, evaluate_candidate, local_day,
};
use crate::triggers::{Candidate, Trigger, TriggerKind, matching_priority, select_one};
use chrono::{DateTime, Duration, Offset, Utc};

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

/// Work performed by one periodic maintenance pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceResult {
    pub reopened_snoozes: u64,
    pub deadline_surface: Option<DwellAction>,
}

/// Durable surfacing or OS delivery failure.
#[derive(Debug, thiserror::Error)]
pub enum SurfaceError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Notification(#[from] NotifyError),
    #[error("{notification}; additionally failed to record delivery failure: {database}")]
    NotificationFinalize {
        notification: NotifyError,
        database: DbError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceCause {
    Context,
    Deadline,
}

/// Combines stored triggers with OS+extension focus, then surfaces at most one candidate.
pub fn handle_dwell(
    db: &Database,
    sink: &dyn NotificationSink,
    target: &FocusTarget,
    now: DateTime<Utc>,
    phase0_rules: &[Phase0Rule],
) -> Result<DwellAction, SurfaceError> {
    // A due snooze is reopened by maintenance but remains ineligible until a
    // newly completed dwell reaches this point.
    db.clear_reopened_snooze_markers(now.timestamp())?;
    if db.kill_gate_passed("extraction_precision_300")? {
        if let Some(action) = surface_extracted(db, sink, target, now)? {
            return Ok(action);
        }
    }
    if notify_matched(&target.app_id, phase0_rules, sink)? {
        if let Some(rule) = match_phase0(&target.app_id, phase0_rules) {
            return Ok(DwellAction::Phase0Shown { rule_id: rule.id });
        }
    }
    Ok(DwellAction::None)
}

/// Reopens due snoozes and surfaces at most one never-shown due deadline.
pub fn handle_maintenance_tick(
    db: &Database,
    sink: &dyn NotificationSink,
    now: DateTime<Utc>,
) -> Result<MaintenanceResult, SurfaceError> {
    let reopened_snoozes = db.reopen_due_snoozes(now.timestamp())?;
    let deadline_surface = if db.kill_gate_passed("extraction_precision_300")? {
        let mut candidates = db
            .list_due_deadline_candidates(now.timestamp())?
            .into_iter()
            .map(|row| Candidate {
                promise_id: row.promise_id,
                priority: 0,
                deadline_ts: Some(row.deadline_ts),
                confidence: row.confidence,
                created_at: row.created_at,
            })
            .collect::<Vec<_>>();
        match select_one(&mut candidates) {
            Some(candidate) => Some(deliver_candidate(
                db,
                sink,
                candidate,
                now,
                SurfaceCause::Deadline,
            )?),
            None => None,
        }
    } else {
        None
    };
    Ok(MaintenanceResult {
        reopened_snoozes,
        deadline_surface,
    })
}

fn surface_extracted(
    db: &Database,
    sink: &dyn NotificationSink,
    target: &FocusTarget,
    now: DateTime<Utc>,
) -> Result<Option<DwellAction>, SurfaceError> {
    let mut matched = matching_open_candidates(db, target, now.timestamp())?;
    let Some(winner) = select_one(&mut matched) else {
        return Ok(None);
    };
    deliver_candidate(db, sink, winner, now, SurfaceCause::Context).map(Some)
}

fn deliver_candidate(
    db: &Database,
    sink: &dyn NotificationSink,
    winner: Candidate,
    now: DateTime<Utc>,
    cause: SurfaceCause,
) -> Result<DwellAction, SurfaceError> {
    let (config, state, day) = load_policy(db, now)?;
    let already = db.promise_shown_on_day(winner.promise_id, &day)?;
    if matches!(
        evaluate_candidate(now, &day, already, &config, &state),
        Eligibility::Suppress(_)
    ) {
        return Ok(DwellAction::Suppressed);
    }

    let lease_token = uuid::Uuid::new_v4().to_string();
    let action_token = uuid::Uuid::new_v4().to_string();
    let surface_attempt_id = db.begin_notification_attempt(
        winner.promise_id,
        &lease_token,
        &action_token,
        now.timestamp(),
        now.timestamp() + 15 * 60,
    )?;
    let request = NotificationRequest::actionable(&action_token);
    match sink.show(&request) {
        Ok(()) => {
            db.finish_notification_delivered(
                surface_attempt_id,
                now.timestamp(),
                &day,
                matches!(cause, SurfaceCause::Deadline),
            )?;
            Ok(DwellAction::ExtractedShown {
                promise_id: winner.promise_id,
                action_token,
            })
        }
        Err(notification) => {
            match db.finish_notification_failed(
                surface_attempt_id,
                now.timestamp(),
                &notification.to_string(),
            ) {
                Ok(()) => Err(SurfaceError::Notification(notification)),
                Err(database) => Err(SurfaceError::NotificationFinalize {
                    notification,
                    database,
                }),
            }
        }
    }
}

fn matching_open_candidates(
    db: &Database,
    target: &FocusTarget,
    now_unix: i64,
) -> Result<Vec<Candidate>, DbError> {
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
) -> Result<(RateLimitConfig, RateLimitState, String), DbError> {
    let offset_seconds = configured_offset_seconds(db, now)?;
    let day = local_day(now, offset_seconds);
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
    Ok((config, state, day))
}

fn configured_offset_seconds(db: &Database, now: DateTime<Utc>) -> Result<i32, DbError> {
    let timezone = db.get_setting("timezone")?.unwrap_or_else(|| "UTC".into());
    let timezone = timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| DbError::InvalidSetting {
            key: "timezone".into(),
            reason: "must be a valid IANA timezone".into(),
        })?;
    Ok(now
        .with_timezone(&timezone)
        .offset()
        .fix()
        .local_minus_utc())
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
