use std::time::{Duration, Instant};

#[cfg(all(target_os = "windows", feature = "windows-platform"))]
#[path = "sys_windows.rs"]
mod sys;
#[cfg(not(all(target_os = "windows", feature = "windows-platform")))]
#[path = "sys_noop.rs"]
mod sys;

/// Foreground process reported by the OS adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsFocus {
    /// Executable basename, lowercased by the caller when comparing.
    pub exe_name: String,
}

/// Active-tab context reported by the extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserContext {
    pub source_app: String,
    pub source_ctx: Option<String>,
    pub visible: bool,
    pub active: bool,
}

/// Combined focus identity used by debounce and trigger matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusTarget {
    pub app_id: String,
    pub context: Option<String>,
}

/// Outcome of a debounce step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebounceOutcome {
    Pending,
    Cancelled,
    Idle,
    Fired(FocusTarget),
}

/// OS session or power events that must invalidate an in-flight dwell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusEvent {
    /// Foreground window changed; payload is the new PID.
    ForegroundPid(u32),
    /// Workstation locked (WTS session lock).
    SessionLock,
    /// Workstation unlocked.
    SessionUnlock,
    /// Machine is entering sleep/suspend.
    Sleep,
    /// Machine resumed from sleep/suspend.
    Resume,
}

impl FocusEvent {
    /// Lock, unlock, sleep, and resume all bump the debounce generation.
    #[must_use]
    pub const fn invalidates_dwell(&self) -> bool {
        matches!(
            self,
            Self::SessionLock | Self::SessionUnlock | Self::Sleep | Self::Resume
        )
    }
}

/// Last extension heartbeat plus when it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveBrowserContext {
    pub context: BrowserContext,
    pub received_at: Instant,
}

impl LiveBrowserContext {
    /// Returns the heartbeat when it is still fresh enough to trust.
    #[must_use]
    pub fn fresh(&self, now: Instant, max_age: Duration) -> Option<&BrowserContext> {
        now.checked_duration_since(self.received_at)
            .is_some_and(|age| age <= max_age)
            .then_some(&self.context)
    }
}

/// Parses an extension context envelope. Accepts snake_case or camelCase keys.
#[must_use]
pub fn parse_browser_context(payload: &serde_json::Value) -> Option<BrowserContext> {
    let source_app = payload
        .get("source_app")
        .or_else(|| payload.get("sourceApp"))
        .and_then(serde_json::Value::as_str)?
        .to_ascii_lowercase();
    if source_app != "gmail" && source_app != "slack" {
        return None;
    }
    let source_ctx = payload
        .get("source_ctx")
        .or_else(|| payload.get("sourceCtx"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Some(BrowserContext {
        source_app,
        source_ctx,
        visible: payload
            .get("visible")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        active: payload
            .get("active")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

impl DebounceOutcome {
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    #[must_use]
    pub fn was_cancelled_or_idle(&self) -> bool {
        matches!(self, Self::Cancelled | Self::Idle)
    }

    #[must_use]
    pub fn fired_app(&self) -> Option<&str> {
        match self {
            Self::Fired(target) => Some(target.app_id.as_str()),
            _ => None,
        }
    }
}

/// Five-second cancellable dwell.
pub struct FocusDebouncer {
    dwell: Duration,
    pending: Option<(FocusTarget, Instant, u64)>,
    generation: u64,
}

impl FocusDebouncer {
    #[must_use]
    pub const fn new(dwell: Duration) -> Self {
        Self {
            dwell,
            pending: None,
            generation: 0,
        }
    }

    /// Monotonic generation. Lock/sleep/unlock/resume bump it so stale timers cannot fire.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = None;
    }

    /// Restarts the dwell when the OS identity changes.
    pub fn on_os_focus(&mut self, target: Option<FocusTarget>, now: Instant) -> DebounceOutcome {
        match target {
            None => {
                self.pending = None;
                DebounceOutcome::Cancelled
            }
            Some(next) => {
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|(current, _, scheduled)| {
                        current == &next && *scheduled == self.generation
                    })
                {
                    DebounceOutcome::Pending
                } else {
                    self.pending = Some((next, now, self.generation));
                    DebounceOutcome::Pending
                }
            }
        }
    }

    /// Cancels any in-flight dwell after lock or sleep.
    pub fn on_lock_or_sleep(&mut self) -> DebounceOutcome {
        self.bump_generation();
        DebounceOutcome::Cancelled
    }

    /// Forwards lock/unlock/sleep/resume into the debounce generation.
    pub fn apply_focus_event(&mut self, event: &FocusEvent, _now: Instant) -> DebounceOutcome {
        if event.invalidates_dwell() {
            self.on_lock_or_sleep()
        } else {
            DebounceOutcome::Pending
        }
    }

    /// Fires a previously scheduled dwell only when the generation is still current.
    pub fn fire_scheduled(&mut self, generation: u64, now: Instant) -> DebounceOutcome {
        if generation != self.generation {
            return DebounceOutcome::Cancelled;
        }
        self.on_tick(now)
    }

    /// Fires when the pending target has continuously dwelled long enough.
    pub fn on_tick(&mut self, now: Instant) -> DebounceOutcome {
        let Some((target, started, scheduled)) = self.pending.clone() else {
            return DebounceOutcome::Idle;
        };
        if scheduled != self.generation {
            self.pending = None;
            return DebounceOutcome::Cancelled;
        }
        if now.duration_since(started) >= self.dwell {
            self.pending = None;
            DebounceOutcome::Fired(target)
        } else {
            DebounceOutcome::Pending
        }
    }
}

/// Combines OS foreground identity with an extension heartbeat.
#[must_use]
pub fn combine_focus(os: Option<&OsFocus>, browser: Option<&BrowserContext>) -> FocusTarget {
    let app_id = os
        .map(|focus| focus.exe_name.to_ascii_lowercase())
        .unwrap_or_else(|| "unknown".into());
    let context = match (os, browser) {
        (Some(os_focus), Some(browser))
            if is_chrome(&os_focus.exe_name)
                && browser.visible
                && browser.active
                && (browser.source_app == "gmail" || browser.source_app == "slack") =>
        {
            Some(match &browser.source_ctx {
                Some(ctx) if !ctx.is_empty() => format!("{}:{ctx}", browser.source_app),
                _ => browser.source_app.clone(),
            })
        }
        _ => None,
    };
    FocusTarget { app_id, context }
}

fn is_chrome(exe_name: &str) -> bool {
    let lower = exe_name.to_ascii_lowercase();
    lower == "chrome.exe" || lower == "google chrome"
}

fn exe_basename(path: &str) -> &str {
    path.rsplit(['\\', '/'])
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(path)
}

/// Builds a dwell target from a process image path plus a live extension heartbeat.
#[must_use]
pub fn combine_live_focus(
    image_path: Option<String>,
    live: Option<&LiveBrowserContext>,
    now: Instant,
) -> Option<FocusTarget> {
    let path = image_path?;
    let os = OsFocus {
        exe_name: exe_basename(&path).to_owned(),
    };
    let browser = live.and_then(|item| item.fresh(now, Duration::from_secs(15)));
    let mut target = combine_focus(Some(&os), browser);
    target.app_id = path;
    Some(target)
}

/// Resolves a PID to an image path without panicking on access denial.
///
/// # Errors
///
/// Returns [`FocusError`] when the platform lookup itself fails unexpectedly.
pub fn resolve_process_image(pid: u32) -> Result<Option<String>, FocusError> {
    crate::platform::focus::sys::resolve_process_image(pid)
}

/// Focus lookup failure.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum FocusError {
    #[error("process lookup failed")]
    LookupFailed,
}

/// Starts the platform focus watcher. The callback only enqueues an event.
pub fn spawn_focus_watcher() -> std::sync::mpsc::Receiver<FocusEvent> {
    let (tx, rx) = std::sync::mpsc::sync_channel(32);
    sys::spawn_watcher(tx);
    rx
}

/// Current foreground PID, used to restart dwell after unlock/resume.
#[must_use]
pub fn current_foreground_pid() -> Option<u32> {
    sys::current_foreground_pid()
}
