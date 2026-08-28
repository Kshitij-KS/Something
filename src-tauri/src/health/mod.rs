use chrono::{DateTime, Utc};
use serde::Serialize;

/// Connection and selector diagnostics shown in the health view.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub connection: String,
    pub native_host: String,
    pub gmail: String,
    pub slack: String,
    pub last_handshake_at: Option<i64>,
    pub silence_remaining_secs: i64,
    pub opens_network_listener: bool,
    pub shortcut: String,
}

/// Seconds remaining in the post-onboarding silence window.
#[must_use]
pub fn silence_remaining(now: DateTime<Utc>, until_unix: Option<&str>) -> i64 {
    let Some(raw) = until_unix else {
        return 0;
    };
    let Ok(until) = raw.parse::<i64>() else {
        return 0;
    };
    (until - now.timestamp()).max(0)
}

/// Selector banner copy based on failed probes, never including message text.
#[must_use]
pub fn selector_banner(site: &str, state: &str, days_without_capture: u32) -> Option<String> {
    if state == "broken" || days_without_capture >= 7 {
        Some(format!(
            "{site} capture may be broken — check for a selector update."
        ))
    } else if state == "degraded" {
        Some(format!(
            "{site} selectors missed a probe. Capture may be unreliable."
        ))
    } else {
        None
    }
}
