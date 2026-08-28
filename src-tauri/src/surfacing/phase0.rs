use crate::platform::notifications::{NotificationRequest, NotificationSink, NotifyError};
use serde::{Deserialize, Serialize};

/// Hardcoded Phase 0 reminder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase0Rule {
    pub id: i64,
    pub app_match: String,
    pub reminder_text: String,
    pub enabled: bool,
}

/// Matches an executable path against Phase 0 rules.
#[must_use]
pub fn match_phase0<'a>(app_path: &str, rules: &'a [Phase0Rule]) -> Option<&'a Phase0Rule> {
    let haystack = basename(app_path).to_ascii_lowercase();
    rules
        .iter()
        .find(|rule| rule.enabled && basename(&rule.app_match).eq_ignore_ascii_case(&haystack))
}

/// Shows a Phase 0 reminder when `app_path` matches an enabled rule.
///
/// # Errors
///
/// Returns [`NotifyError`] when the sink cannot deliver.
pub fn notify_matched(
    app_path: &str,
    rules: &[Phase0Rule],
    sink: &dyn NotificationSink,
) -> Result<bool, NotifyError> {
    let Some(rule) = match_phase0(app_path, rules) else {
        return Ok(false);
    };
    sink.show(&NotificationRequest {
        title: "Callback".into(),
        body: rule.reminder_text.clone(),
        action_token: format!("phase0:{}", rule.id),
    })?;
    Ok(true)
}

fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/'])
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(path)
}
