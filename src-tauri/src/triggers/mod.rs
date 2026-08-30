use crate::platform::focus::FocusTarget;

/// Trigger kinds auto-linked at capture time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub kind: TriggerKind,
    pub match_value: String,
    pub priority: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    AppFocus,
    AppCtxFocus,
    Deadline,
    Manual,
}

impl TriggerKind {
    /// Stable database spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppFocus => "app_focus",
            Self::AppCtxFocus => "app_ctx_focus",
            Self::Deadline => "deadline",
            Self::Manual => "manual",
        }
    }

    /// Parses a persisted trigger kind.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "app_focus" => Some(Self::AppFocus),
            "app_ctx_focus" => Some(Self::AppCtxFocus),
            "deadline" => Some(Self::Deadline),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

/// Promise fields needed to auto-link triggers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInput<'a> {
    pub source_app: &'a str,
    pub source_ctx: Option<&'a str>,
    pub text: &'a str,
    pub keyword_app_map: &'a [(String, String)],
}

/// Creates context-specific, app-level, and optional keyword fallbacks.
#[must_use]
pub fn auto_link(input: LinkInput<'_>) -> Vec<Trigger> {
    let mut triggers = Vec::new();
    if input.source_app.eq_ignore_ascii_case("manual") {
        triggers.push(Trigger {
            kind: TriggerKind::Manual,
            match_value: "manual".to_owned(),
            priority: 0,
        });
    } else {
        if let Some(ctx) = input.source_ctx.filter(|value| !value.is_empty()) {
            triggers.push(Trigger {
                kind: TriggerKind::AppCtxFocus,
                match_value: format!("{}:{ctx}", input.source_app),
                priority: 100,
            });
        }
        triggers.push(Trigger {
            kind: TriggerKind::AppFocus,
            match_value: input.source_app.to_owned(),
            priority: 10,
        });
    }
    let lower = input.text.to_ascii_lowercase();
    for (keyword, exe) in input.keyword_app_map {
        if lower.contains(keyword) {
            triggers.push(Trigger {
                kind: TriggerKind::AppFocus,
                match_value: exe.clone(),
                priority: 5,
            });
        }
    }
    triggers
}

/// Picks one promise among matches: deadline proximity, then confidence, then age.
#[must_use]
pub fn select_one(candidates: &mut [Candidate]) -> Option<Candidate> {
    candidates.sort_by(|left, right| {
        left.deadline_ts
            .unwrap_or(i64::MAX)
            .cmp(&right.deadline_ts.unwrap_or(i64::MAX))
            .then_with(|| {
                right
                    .confidence
                    .partial_cmp(&left.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| right.priority.cmp(&left.priority))
    });
    candidates.first().cloned()
}

/// A matching promise considered for surfacing.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub promise_id: i64,
    pub priority: i32,
    pub deadline_ts: Option<i64>,
    pub confidence: f64,
    pub created_at: i64,
}

/// Matches a dwelled focus target against stored triggers.
#[must_use]
pub fn matching_priority(target: &FocusTarget, trigger: &Trigger) -> Option<i32> {
    match trigger.kind {
        TriggerKind::AppCtxFocus => {
            let context = target.context.as_deref()?;
            (context == trigger.match_value).then_some(trigger.priority)
        }
        TriggerKind::AppFocus => {
            let app = basename(&target.app_id);
            let expected = basename(&trigger.match_value);
            if app.eq_ignore_ascii_case(expected) || web_app_matches(target, &trigger.match_value) {
                Some(trigger.priority)
            } else {
                None
            }
        }
        TriggerKind::Deadline | TriggerKind::Manual => None,
    }
}

fn web_app_matches(target: &FocusTarget, source_app: &str) -> bool {
    let Some(context) = &target.context else {
        return false;
    };
    let app = context.split(':').next().unwrap_or(context);
    app.eq_ignore_ascii_case(source_app) && target.app_id.to_ascii_lowercase().contains("chrome")
}

fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/'])
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(path)
}
