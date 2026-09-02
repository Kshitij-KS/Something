use crate::db::{Database, DbError};
use crate::domain::{PromiseEvent, PromiseStatus, apply_promise};

pub const ACTION_SCHEME: &str = "callback-action";
pub const DEFAULT_SNOOZE_SECONDS: i64 = 60 * 60;

/// Notification / review action presented with an action token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceAction {
    Done,
    Snooze,
    Reject,
    Ignore,
}

impl SurfaceAction {
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Snooze => "snooze",
            Self::Reject => "reject",
            Self::Ignore => "ignore",
        }
    }

    #[must_use]
    pub const fn db_value(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Snooze => "snooze",
            Self::Reject => "not_a_promise",
            Self::Ignore => "ignored",
        }
    }

    #[must_use]
    pub const fn event(self, ignore_count: u32) -> PromiseEvent {
        match self {
            Self::Done => PromiseEvent::Complete,
            Self::Snooze => PromiseEvent::Snooze,
            Self::Reject => PromiseEvent::Reject,
            Self::Ignore => PromiseEvent::Ignore {
                count_after: ignore_count.saturating_add(1),
            },
        }
    }

    fn from_verb(value: &str) -> Option<Self> {
        match value {
            "done" => Some(Self::Done),
            "snooze" => Some(Self::Snooze),
            "reject" => Some(Self::Reject),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }
}

/// Outcome of redeeming an action token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionResult {
    Applied {
        next: PromiseStatus,
        event: PromiseEvent,
    },
    Duplicate,
    Late,
    UnknownToken,
}

/// Parsed mutating notification activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionActivation {
    pub action: SurfaceAction,
    pub action_token: String,
}

/// Parsed local protocol activation. Opening a notification is deliberately
/// separate from redeeming one of its lifecycle actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolActivation {
    Open { action_token: String },
    Action(ActionActivation),
}

impl ProtocolActivation {
    #[must_use]
    pub fn action_token(&self) -> &str {
        match self {
            Self::Open { action_token } | Self::Action(ActionActivation { action_token, .. }) => {
                action_token
            }
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ParseActionError {
    #[error("malformed Callback action URI")]
    Malformed,
    #[error("unknown Callback action")]
    UnknownAction,
    #[error("Callback action token is not a canonical UUID")]
    InvalidToken,
    #[error("multiple Callback actions were supplied")]
    Ambiguous,
}

/// Parses one argument without touching process or application state.
pub fn parse_action_argument(raw: &str) -> Result<Option<ProtocolActivation>, ParseActionError> {
    let Some(rest) = raw.strip_prefix("callback-action://") else {
        if raw.starts_with("callback-action:") {
            return Err(ParseActionError::Malformed);
        }
        return Ok(None);
    };
    let mut parts = rest.split('/');
    let verb = parts.next().filter(|part| !part.is_empty());
    let token = parts.next().filter(|part| !part.is_empty());
    if verb.is_none() || token.is_none() || parts.next().is_some() {
        return Err(ParseActionError::Malformed);
    }
    let verb = verb.unwrap_or_default();
    let token = token.unwrap_or_default();
    let parsed = uuid::Uuid::parse_str(token).map_err(|_| ParseActionError::InvalidToken)?;
    if parsed.to_string() != token {
        return Err(ParseActionError::InvalidToken);
    }
    if verb == "open" {
        return Ok(Some(ProtocolActivation::Open {
            action_token: token.to_owned(),
        }));
    }
    let action = SurfaceAction::from_verb(verb).ok_or(ParseActionError::UnknownAction)?;
    Ok(Some(ProtocolActivation::Action(ActionActivation {
        action,
        action_token: token.to_owned(),
    })))
}

/// Finds at most one Callback activation in cold-start process arguments.
pub fn parse_cold_start_args<I, S>(args: I) -> Result<Option<ProtocolActivation>, ParseActionError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut activation = None;
    for arg in args {
        if let Some(parsed) = parse_action_argument(arg.as_ref())? {
            if activation.is_some() {
                return Err(ParseActionError::Ambiguous);
            }
            activation = Some(parsed);
        }
    }
    Ok(activation)
}

/// Redeems a parsed mutating activation with a deterministic one-hour snooze policy.
pub fn dispatch_activation(
    db: &Database,
    activation: &ActionActivation,
    now: i64,
    local_day: &str,
) -> Result<ActionResult, DbError> {
    let snooze_until = matches!(activation.action, SurfaceAction::Snooze)
        .then_some(now.saturating_add(DEFAULT_SNOOZE_SECONDS));
    db.redeem_surface_action(
        &activation.action_token,
        activation.action,
        now,
        local_day,
        snooze_until,
    )
}

/// In-memory transition helper retained for pure state-machine tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionToken {
    pub token: String,
    pub consumed: bool,
    pub promise_status: PromiseStatus,
    pub ignore_count: u32,
}

/// Redeems an in-memory token. Durable production callbacks use
/// [`dispatch_activation`] instead.
#[must_use]
pub fn redeem(token: &mut ActionToken, presented: &str, action: SurfaceAction) -> ActionResult {
    if token.token != presented {
        return ActionResult::UnknownToken;
    }
    if token.consumed {
        return ActionResult::Duplicate;
    }
    let event = action.event(token.ignore_count);
    match apply_promise(token.promise_status, event) {
        Ok(next) => {
            token.consumed = true;
            token.promise_status = next;
            if matches!(action, SurfaceAction::Ignore) {
                token.ignore_count = token.ignore_count.saturating_add(1);
            }
            ActionResult::Applied { next, event }
        }
        Err(_) => ActionResult::Late,
    }
}
