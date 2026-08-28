use callback_lib::domain::{
    ConnectionEvent, ConnectionState, PromiseEvent, PromiseStatus, SelectorHealthEvent,
    SelectorHealthState, SurfaceAttemptEvent, SurfaceAttemptState, apply_connection, apply_promise,
    apply_selector_health, apply_surface_attempt,
};

#[test]
fn promise_review_promotes_to_open_and_rejects_direct_done() {
    assert_eq!(
        apply_promise(PromiseStatus::Review, PromiseEvent::Promote).expect("promote"),
        PromiseStatus::Open
    );
    let error =
        apply_promise(PromiseStatus::Review, PromiseEvent::Complete).expect_err("review done");
    assert_eq!(error.from, PromiseStatus::Review);
    assert_eq!(error.event, PromiseEvent::Complete);
}

#[test]
fn promise_open_supports_done_snooze_reject_and_three_ignores_archive() {
    assert_eq!(
        apply_promise(PromiseStatus::Open, PromiseEvent::Complete).unwrap(),
        PromiseStatus::Done
    );
    assert_eq!(
        apply_promise(PromiseStatus::Open, PromiseEvent::Snooze).unwrap(),
        PromiseStatus::Snoozed
    );
    assert_eq!(
        apply_promise(PromiseStatus::Open, PromiseEvent::Reject).unwrap(),
        PromiseStatus::Dismissed
    );
    assert_eq!(
        apply_promise(PromiseStatus::Open, PromiseEvent::Ignore { count_after: 1 }).unwrap(),
        PromiseStatus::Open
    );
    assert_eq!(
        apply_promise(PromiseStatus::Open, PromiseEvent::Ignore { count_after: 3 }).unwrap(),
        PromiseStatus::Archived
    );
}

#[test]
fn snoozed_promise_returns_to_open_only_on_expiry() {
    assert_eq!(
        apply_promise(PromiseStatus::Snoozed, PromiseEvent::ExpireSnooze).unwrap(),
        PromiseStatus::Open
    );
    apply_promise(PromiseStatus::Snoozed, PromiseEvent::Complete).expect_err("still snoozed");
}

#[test]
fn terminal_promise_states_reject_further_work() {
    for status in [
        PromiseStatus::Done,
        PromiseStatus::Dismissed,
        PromiseStatus::Archived,
    ] {
        apply_promise(status, PromiseEvent::Promote).expect_err("terminal");
        apply_promise(status, PromiseEvent::Complete).expect_err("terminal");
    }
}

#[test]
fn connection_state_machine_covers_handshake_and_reconnect() {
    assert_eq!(
        apply_connection(
            ConnectionState::Disconnected,
            ConnectionEvent::StartHandshake
        )
        .unwrap(),
        ConnectionState::Handshaking
    );
    assert_eq!(
        apply_connection(ConnectionState::Handshaking, ConnectionEvent::Established).unwrap(),
        ConnectionState::Connected
    );
    assert_eq!(
        apply_connection(ConnectionState::Connected, ConnectionEvent::Drop).unwrap(),
        ConnectionState::Reconnecting
    );
    assert_eq!(
        apply_connection(ConnectionState::Reconnecting, ConnectionEvent::Established).unwrap(),
        ConnectionState::Connected
    );
}

#[test]
fn surface_attempt_lease_to_shown_to_acted_and_rejects_duplicate_action() {
    assert_eq!(
        apply_surface_attempt(SurfaceAttemptState::Leased, SurfaceAttemptEvent::Show).unwrap(),
        SurfaceAttemptState::Shown
    );
    assert_eq!(
        apply_surface_attempt(SurfaceAttemptState::Shown, SurfaceAttemptEvent::Act).unwrap(),
        SurfaceAttemptState::Acted
    );
    apply_surface_attempt(SurfaceAttemptState::Acted, SurfaceAttemptEvent::Act)
        .expect_err("duplicate");
}

#[test]
fn selector_health_moves_from_healthy_to_broken_after_failed_probes() {
    assert_eq!(
        apply_selector_health(
            SelectorHealthState::Healthy,
            SelectorHealthEvent::ProbeFailed
        )
        .unwrap(),
        SelectorHealthState::Degraded
    );
    assert_eq!(
        apply_selector_health(
            SelectorHealthState::Degraded,
            SelectorHealthEvent::ConsecutiveFailures { count: 3 }
        )
        .unwrap(),
        SelectorHealthState::Broken
    );
    assert_eq!(
        apply_selector_health(
            SelectorHealthState::Broken,
            SelectorHealthEvent::ProbeSucceeded
        )
        .unwrap(),
        SelectorHealthState::Healthy
    );
}
