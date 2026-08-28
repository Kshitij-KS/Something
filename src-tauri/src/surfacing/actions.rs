use crate::domain::{PromiseEvent, PromiseStatus, apply_promise};

/// Notification / review action presented with an action token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceAction {
    Done,
    Snooze,
    Reject,
    Ignore,
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

/// Crash-safe action callback: a token may be redeemed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionToken {
    pub token: String,
    pub consumed: bool,
    pub promise_status: PromiseStatus,
    pub ignore_count: u32,
}

/// Redeems a stored token. Duplicate and late callbacks are no-ops.
#[must_use]
pub fn redeem(token: &mut ActionToken, presented: &str, action: SurfaceAction) -> ActionResult {
    if token.token != presented {
        return ActionResult::UnknownToken;
    }
    if token.consumed {
        return ActionResult::Duplicate;
    }
    let event = match action {
        SurfaceAction::Done => PromiseEvent::Complete,
        SurfaceAction::Snooze => PromiseEvent::Snooze,
        SurfaceAction::Reject => PromiseEvent::Reject,
        SurfaceAction::Ignore => PromiseEvent::Ignore {
            count_after: token.ignore_count.saturating_add(1),
        },
    };
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
