use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One site's durable, content-free selector diagnostic.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SelectorHealthSnapshot {
    pub site: String,
    pub status: String,
    pub first_observed_at: Option<i64>,
    pub last_probe_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_capture_at: Option<i64>,
    pub consecutive_failures: u32,
    pub days_without_capture: u32,
    pub banner: Option<String>,
}

/// Connection and selector diagnostics shown in the health view.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub connection: String,
    pub native_host: String,
    pub gmail: String,
    pub slack: String,
    pub selectors: Vec<SelectorHealthSnapshot>,
    pub last_handshake_at: Option<i64>,
    pub silence_remaining_secs: i64,
    pub opens_network_listener: bool,
    pub shortcut: String,
}

/// Content-free probe payload accepted from the extension.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SelectorProbe {
    pub site: String,
    pub ok: bool,
    pub observed_at: i64,
}

/// Parses and validates a selector probe envelope payload.
#[must_use]
pub fn parse_selector_probe(payload: &serde_json::Value) -> Option<SelectorProbe> {
    let mut probe: SelectorProbe = serde_json::from_value(payload.clone()).ok()?;
    if !matches!(probe.site.as_str(), "gmail" | "slack") || probe.observed_at <= 0 {
        return None;
    }
    if probe.observed_at > 10_000_000_000 {
        probe.observed_at /= 1000;
    }
    Some(probe)
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

/// Whole days since the latest confirmed capture, or first observed activity when none exists.
#[must_use]
pub fn days_without_capture(
    now_unix: i64,
    first_observed_at: Option<i64>,
    last_capture_at: Option<i64>,
) -> u32 {
    let Some(reference) = last_capture_at.or(first_observed_at) else {
        return 0;
    };
    u32::try_from(now_unix.saturating_sub(reference) / 86_400).unwrap_or(u32::MAX)
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
