use callback_lib::domain::PromiseStatus;
use callback_lib::surfacing::actions::{ActionResult, ActionToken, SurfaceAction, redeem};
use callback_lib::surfacing::rate_limit::{
    Eligibility, RateLimitConfig, RateLimitState, SuppressReason, evaluate_candidate, local_day,
};
use chrono::{Duration, TimeZone, Utc};

fn cfg() -> RateLimitConfig {
    RateLimitConfig::default()
}

#[test]
fn daily_cap_and_gap_and_quiet_hours_suppress() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .unwrap();
    let day = local_day(now, 0);
    let mut state = RateLimitState::new(now, day.clone());
    state.surfaces_today = 3;
    assert_eq!(
        evaluate_candidate(now, &day, false, &cfg(), &state),
        Eligibility::Suppress(SuppressReason::DailyCap)
    );
    state.surfaces_today = 0;
    state.last_surface_at = Some(now - Duration::minutes(10));
    assert_eq!(
        evaluate_candidate(now, &day, false, &cfg(), &state),
        Eligibility::Suppress(SuppressReason::MinGap)
    );
}

#[test]
fn no_backlog_burst_and_active_surface_blocks() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .unwrap();
    let day = local_day(now, 0);
    let mut state = RateLimitState::new(now, day.clone());
    state.active_surface = true;
    assert_eq!(
        evaluate_candidate(now, &day, false, &cfg(), &state),
        Eligibility::Suppress(SuppressReason::ActiveSurface)
    );
}

#[test]
fn same_promise_cannot_surface_twice_in_a_day() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .unwrap();
    let day = local_day(now, 0);
    let state = RateLimitState::new(now, day.clone());
    assert_eq!(
        evaluate_candidate(now, &day, true, &cfg(), &state),
        Eligibility::Suppress(SuppressReason::SamePromiseToday)
    );
}

#[test]
fn day_rollover_resets_cap_via_new_local_day() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 1, 0, 0).single().unwrap();
    let mut state = RateLimitState::new(now - Duration::days(1), "2026-08-27".into());
    state.surfaces_today = 3;
    assert_eq!(
        evaluate_candidate(now, "2026-08-28", false, &cfg(), &state),
        Eligibility::Allow
    );
}

#[test]
fn clock_rollback_does_not_burst() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .unwrap();
    let day = local_day(now, 0);
    let state = RateLimitState::new(now, day.clone());
    let rolled = now - Duration::hours(3);
    assert_eq!(
        evaluate_candidate(rolled, &day, false, &cfg(), &state),
        Eligibility::Suppress(SuppressReason::ClockRollback)
    );
}

#[test]
fn onboarding_silence_and_quiet_hours() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .unwrap();
    let day = local_day(now, 0);
    let mut config = cfg();
    config.silence_until = Some(now + Duration::minutes(30));
    let state = RateLimitState::new(now, day.clone());
    assert_eq!(
        evaluate_candidate(now, &day, false, &config, &state),
        Eligibility::Suppress(SuppressReason::OnboardingSilence)
    );
}

#[test]
fn duplicate_and_late_actions_are_ignored() {
    let mut token = ActionToken {
        token: "tok".into(),
        consumed: false,
        promise_status: PromiseStatus::Open,
        ignore_count: 0,
    };
    assert!(matches!(
        redeem(&mut token, "tok", SurfaceAction::Done),
        ActionResult::Applied {
            next: PromiseStatus::Done,
            ..
        }
    ));
    assert_eq!(
        redeem(&mut token, "tok", SurfaceAction::Done),
        ActionResult::Duplicate
    );
    let mut snoozed = ActionToken {
        token: "late".into(),
        consumed: false,
        promise_status: PromiseStatus::Snoozed,
        ignore_count: 0,
    };
    assert_eq!(
        redeem(&mut snoozed, "late", SurfaceAction::Done),
        ActionResult::Late
    );
}

#[test]
fn three_ignores_archive() {
    let mut token = ActionToken {
        token: "ig".into(),
        consumed: false,
        promise_status: PromiseStatus::Open,
        ignore_count: 2,
    };
    let result = redeem(&mut token, "ig", SurfaceAction::Ignore);
    assert!(matches!(
        result,
        ActionResult::Applied {
            next: PromiseStatus::Archived,
            ..
        }
    ));
}

#[test]
fn snooze_requires_a_new_focus_transition_not_immediate_resurface() {
    let now = Utc
        .with_ymd_and_hms(2026, 8, 27, 12, 0, 0)
        .single()
        .unwrap();
    let day = local_day(now, 0);
    let mut state = RateLimitState::new(now, day.clone());
    state.active_surface = true;
    assert_eq!(
        evaluate_candidate(now, &day, false, &cfg(), &state),
        Eligibility::Suppress(SuppressReason::ActiveSurface)
    );
    state.active_surface = false;
    state.last_surface_at = Some(now);
    assert_eq!(
        evaluate_candidate(now + Duration::minutes(1), &day, false, &cfg(), &state),
        Eligibility::Suppress(SuppressReason::MinGap)
    );
}
