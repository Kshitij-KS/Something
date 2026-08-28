use serde::{Deserialize, Serialize};

/// Capture origin constrained to v1 surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceApp {
    /// Slack web.
    Slack,
    /// Gmail web.
    Gmail,
    /// Quick-capture or Phase 0 manual entry.
    Manual,
}

impl SourceApp {
    /// Stable database spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Gmail => "gmail",
            Self::Manual => "manual",
        }
    }

    /// Parses a constrained source value.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is not allowed.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "slack" => Ok(Self::Slack),
            "gmail" => Ok(Self::Gmail),
            "manual" => Ok(Self::Manual),
            other => Err(format!("unsupported source {other}")),
        }
    }
}

/// Durable promise lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromiseStatus {
    Review,
    Open,
    Snoozed,
    Done,
    Dismissed,
    Archived,
}

impl PromiseStatus {
    /// Stable database spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Open => "open",
            Self::Snoozed => "snoozed",
            Self::Done => "done",
            Self::Dismissed => "dismissed",
            Self::Archived => "archived",
        }
    }

    /// Routes a heuristic score onto a persisted status.
    #[must_use]
    pub const fn from_score(score: i32) -> Self {
        if score >= 6 { Self::Open } else { Self::Review }
    }
}

/// Legal promise transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromiseEvent {
    Promote,
    Complete,
    Snooze,
    Reject,
    Ignore { count_after: u32 },
    ExpireSnooze,
}

/// Failed transition details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransition<S, E> {
    pub from: S,
    pub event: E,
}

/// Applies a promise lifecycle event.
///
/// # Errors
///
/// Returns the rejected transition when the event is illegal.
pub fn apply_promise(
    from: PromiseStatus,
    event: PromiseEvent,
) -> Result<PromiseStatus, InvalidTransition<PromiseStatus, PromiseEvent>> {
    let next = match (from, event) {
        (PromiseStatus::Review, PromiseEvent::Promote) => PromiseStatus::Open,
        (PromiseStatus::Review, PromiseEvent::Reject) => PromiseStatus::Dismissed,
        (PromiseStatus::Open, PromiseEvent::Complete) => PromiseStatus::Done,
        (PromiseStatus::Open, PromiseEvent::Snooze) => PromiseStatus::Snoozed,
        (PromiseStatus::Open, PromiseEvent::Reject) => PromiseStatus::Dismissed,
        (PromiseStatus::Open, PromiseEvent::Ignore { count_after }) => {
            if count_after >= 3 {
                PromiseStatus::Archived
            } else {
                PromiseStatus::Open
            }
        }
        (PromiseStatus::Snoozed, PromiseEvent::ExpireSnooze) => PromiseStatus::Open,
        (PromiseStatus::Snoozed, PromiseEvent::Reject) => PromiseStatus::Dismissed,
        _ => {
            return Err(InvalidTransition { from, event });
        }
    };
    Ok(next)
}

/// Native-host connection lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Handshaking,
    Connected,
    Reconnecting,
}

/// Connection lifecycle events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionEvent {
    StartHandshake,
    Established,
    Drop,
}

/// Applies a connection lifecycle event.
///
/// # Errors
///
/// Returns the rejected transition when the event is illegal.
pub fn apply_connection(
    from: ConnectionState,
    event: ConnectionEvent,
) -> Result<ConnectionState, InvalidTransition<ConnectionState, ConnectionEvent>> {
    let next = match (from, event) {
        (ConnectionState::Disconnected, ConnectionEvent::StartHandshake)
        | (ConnectionState::Reconnecting, ConnectionEvent::StartHandshake) => {
            ConnectionState::Handshaking
        }
        (ConnectionState::Handshaking, ConnectionEvent::Established)
        | (ConnectionState::Reconnecting, ConnectionEvent::Established) => {
            ConnectionState::Connected
        }
        (ConnectionState::Connected, ConnectionEvent::Drop)
        | (ConnectionState::Handshaking, ConnectionEvent::Drop) => ConnectionState::Reconnecting,
        (ConnectionState::Reconnecting, ConnectionEvent::Drop) => ConnectionState::Reconnecting,
        _ => return Err(InvalidTransition { from, event }),
    };
    Ok(next)
}

/// Surface attempt / lease lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceAttemptState {
    Leased,
    Shown,
    Acted,
    Expired,
    Suppressed,
}

/// Surface attempt events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceAttemptEvent {
    Show,
    Act,
    Expire,
    Suppress,
}

/// Applies a surface-attempt lifecycle event.
///
/// # Errors
///
/// Returns the rejected transition when the event is illegal.
pub fn apply_surface_attempt(
    from: SurfaceAttemptState,
    event: SurfaceAttemptEvent,
) -> Result<SurfaceAttemptState, InvalidTransition<SurfaceAttemptState, SurfaceAttemptEvent>> {
    let next = match (from, event) {
        (SurfaceAttemptState::Leased, SurfaceAttemptEvent::Show) => SurfaceAttemptState::Shown,
        (SurfaceAttemptState::Leased | SurfaceAttemptState::Shown, SurfaceAttemptEvent::Expire) => {
            SurfaceAttemptState::Expired
        }
        (SurfaceAttemptState::Shown, SurfaceAttemptEvent::Act) => SurfaceAttemptState::Acted,
        (SurfaceAttemptState::Leased, SurfaceAttemptEvent::Suppress) => {
            SurfaceAttemptState::Suppressed
        }
        _ => return Err(InvalidTransition { from, event }),
    };
    Ok(next)
}

/// Selector pack health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SelectorHealthState {
    Healthy,
    Degraded,
    Broken,
}

/// Selector probe events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorHealthEvent {
    ProbeSucceeded,
    ProbeFailed,
    ConsecutiveFailures { count: u32 },
}

/// Applies a selector-health event.
///
/// # Errors
///
/// Returns the rejected transition when the event is illegal.
pub fn apply_selector_health(
    from: SelectorHealthState,
    event: SelectorHealthEvent,
) -> Result<SelectorHealthState, InvalidTransition<SelectorHealthState, SelectorHealthEvent>> {
    let next = match (from, event) {
        (_, SelectorHealthEvent::ProbeSucceeded) => SelectorHealthState::Healthy,
        (SelectorHealthState::Healthy, SelectorHealthEvent::ProbeFailed) => {
            SelectorHealthState::Degraded
        }
        (SelectorHealthState::Degraded, SelectorHealthEvent::ProbeFailed) => {
            SelectorHealthState::Degraded
        }
        (_, SelectorHealthEvent::ConsecutiveFailures { count }) if count >= 3 => {
            SelectorHealthState::Broken
        }
        (SelectorHealthState::Degraded, SelectorHealthEvent::ConsecutiveFailures { .. }) => {
            SelectorHealthState::Degraded
        }
        (SelectorHealthState::Broken, SelectorHealthEvent::ProbeFailed) => {
            SelectorHealthState::Broken
        }
        _ => return Err(InvalidTransition { from, event }),
    };
    Ok(next)
}

/// Persisted capture envelope before extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub capture_id: String,
    pub clause_ordinal: i64,
    pub source_app: SourceApp,
    pub source_ctx: Option<String>,
    pub recipient: Option<String>,
    pub raw_message: String,
    pub sent_at: i64,
    pub created_at: i64,
}

impl CaptureRecord {
    /// Test helper for a unique capture key.
    #[must_use]
    pub fn fixture(capture_id: &str, clause_ordinal: i64) -> Self {
        Self {
            capture_id: capture_id.to_owned(),
            clause_ordinal,
            source_app: SourceApp::Slack,
            source_ctx: Some("D0123".into()),
            recipient: Some("Priya".into()),
            raw_message: "I will send the invoice".into(),
            sent_at: 1_700_000_000,
            created_at: 1_700_000_000,
        }
    }
}

/// Lease row used for crash-safe surfacing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceLease {
    pub promise_id: i64,
    pub lease_token: String,
    pub action_token: String,
    pub state: LeaseState,
    pub expires_at: i64,
}

impl SurfaceLease {
    /// Creates a fresh leased attempt.
    #[must_use]
    pub fn new(promise_id: i64, lease_token: &str, action_token: &str) -> Self {
        Self {
            promise_id,
            lease_token: lease_token.to_owned(),
            action_token: action_token.to_owned(),
            state: LeaseState::Leased,
            expires_at: 1_800_000_000,
        }
    }
}

/// Database spelling of [`SurfaceAttemptState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    Leased,
    Shown,
    Acted,
    Expired,
    Suppressed,
}

impl LeaseState {
    /// Stable database spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Leased => "leased",
            Self::Shown => "shown",
            Self::Acted => "acted",
            Self::Expired => "expired",
            Self::Suppressed => "suppressed",
        }
    }

    /// Parses a database spelling.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown values.
    pub fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "leased" => Ok(Self::Leased),
            "shown" => Ok(Self::Shown),
            "acted" => Ok(Self::Acted),
            "expired" => Ok(Self::Expired),
            "suppressed" => Ok(Self::Suppressed),
            other => Err(format!("unknown state {other}")),
        }
    }
}
