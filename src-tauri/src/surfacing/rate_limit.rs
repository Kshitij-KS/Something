use chrono::{DateTime, Duration, Local, Timelike, Utc};

/// Rolling surfacing policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitConfig {
    pub daily_cap: u32,
    pub min_gap: Duration,
    pub quiet_start_minutes: Option<u32>,
    pub quiet_end_minutes: Option<u32>,
    pub silence_until: Option<DateTime<Utc>>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            daily_cap: 3,
            min_gap: Duration::minutes(90),
            quiet_start_minutes: None,
            quiet_end_minutes: None,
            silence_until: None,
        }
    }
}

/// Durable counters used to enforce caps without bursting a backlog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitState {
    pub surfaces_today: u32,
    pub local_day: String,
    pub last_surface_at: Option<DateTime<Utc>>,
    pub last_observed_now: DateTime<Utc>,
    pub active_surface: bool,
}

impl RateLimitState {
    #[must_use]
    pub fn new(now: DateTime<Utc>, local_day: String) -> Self {
        Self {
            surfaces_today: 0,
            local_day,
            last_surface_at: None,
            last_observed_now: now,
            active_surface: false,
        }
    }
}

/// Why a candidate was not shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressReason {
    DailyCap,
    MinGap,
    QuietHours,
    OnboardingSilence,
    ActiveSurface,
    SamePromiseToday,
    ClockRollback,
}

/// Result of evaluating one candidate after a new eligible focus transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    Allow,
    Suppress(SuppressReason),
}

/// Evaluates a single candidate. Never drains a suppressed backlog.
#[must_use]
pub fn evaluate_candidate(
    now: DateTime<Utc>,
    local_day: &str,
    already_shown_today: bool,
    config: &RateLimitConfig,
    state: &RateLimitState,
) -> Eligibility {
    let now = if now < state.last_observed_now - Duration::minutes(5) {
        return Eligibility::Suppress(SuppressReason::ClockRollback);
    } else {
        now.max(state.last_observed_now)
    };
    if state.active_surface {
        return Eligibility::Suppress(SuppressReason::ActiveSurface);
    }
    if local_day != state.local_day {
        // Day rolled over: counters reset in the caller. A new day is eligible.
    } else if state.surfaces_today >= config.daily_cap {
        return Eligibility::Suppress(SuppressReason::DailyCap);
    }
    if already_shown_today {
        return Eligibility::Suppress(SuppressReason::SamePromiseToday);
    }
    if let Some(until) = config.silence_until {
        if now < until {
            return Eligibility::Suppress(SuppressReason::OnboardingSilence);
        }
    }
    if in_quiet_hours(now, config) {
        return Eligibility::Suppress(SuppressReason::QuietHours);
    }
    if let Some(last) = state.last_surface_at {
        if now < last + config.min_gap {
            return Eligibility::Suppress(SuppressReason::MinGap);
        }
    }
    Eligibility::Allow
}

/// Local calendar day used for the daily cap.
#[must_use]
pub fn local_day(now: DateTime<Utc>, offset_seconds: i32) -> String {
    chrono::FixedOffset::east_opt(offset_seconds).map_or_else(
        || now.date_naive().to_string(),
        |offset| now.with_timezone(&offset).date_naive().to_string(),
    )
}

fn in_quiet_hours(now: DateTime<Utc>, config: &RateLimitConfig) -> bool {
    let (Some(start), Some(end)) = (config.quiet_start_minutes, config.quiet_end_minutes) else {
        return false;
    };
    let local = now.with_timezone(&Local);
    let minutes = local.time().num_seconds_from_midnight() / 60;
    if start <= end {
        minutes >= start && minutes < end
    } else {
        minutes >= start || minutes < end
    }
}
