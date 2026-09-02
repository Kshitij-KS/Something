use crate::db::{Database, PromiseDetail, PromiseListScope, PromiseSummary, validate_setting};
use crate::domain::PromiseStatus;
use crate::health::{HealthSnapshot, selector_banner, silence_remaining};
use crate::native_host::autostart::apply_autostart;
use crate::native_host::install::{install_host, reconnect};
use crate::platform::active_adapter;
use crate::platform::focus::LiveBrowserContext;
use crate::review::{ReviewAction, ReviewItem, apply_review, ingest_manual};
use crate::shortcut::{ShortcutOutcome, ShortcutPlan, open_quick_window, register_on_app};
use crate::surfacing::actions::{DEFAULT_SNOOZE_SECONDS, SurfaceAction};
use crate::surfacing::phase0::Phase0Rule;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State, WebviewWindow};

const MAIN_WINDOW_LABEL: &str = "main";
const MAX_TRACKED_PROMISE_ROUTES: usize = 128;

/// Shared runtime state. SQLite writes stay on this mutex.
pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub db_path: PathBuf,
    pub host_exe: PathBuf,
    pub app_exe: PathBuf,
    pub last_handshake_at: Arc<Mutex<Option<i64>>>,
    pub shortcut_status: Arc<Mutex<ShortcutOutcome>>,
    pub live_browser: Arc<Mutex<Option<LiveBrowserContext>>>,
    pub browser_transition_tx: std::sync::mpsc::SyncSender<()>,
    pub promise_routes: Arc<Mutex<PromiseRouteQueue>>,
}

/// Content-minimized notification route exposed to the main webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromiseRouteDto {
    pub route_id: String,
    pub promise_id: i64,
}

/// Privacy-minimized visible app identity exposed only to the main webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FocusAppDto {
    pub executable: String,
}

struct PendingPromiseRoute {
    route_id: String,
    promise_id: i64,
}

#[derive(Default)]
pub struct PromiseRouteQueue {
    pending: VecDeque<PendingPromiseRoute>,
    seen_activation_keys: VecDeque<String>,
    acknowledged_route_ids: VecDeque<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteEnqueueOutcome {
    Queued,
    Duplicate,
}

impl PromiseRouteQueue {
    pub fn enqueue(&mut self, promise_id: i64, activation_key: &str) -> RouteEnqueueOutcome {
        if self
            .seen_activation_keys
            .iter()
            .any(|key| key == activation_key)
        {
            return RouteEnqueueOutcome::Duplicate;
        }

        self.pending.push_back(PendingPromiseRoute {
            route_id: uuid::Uuid::new_v4().to_string(),
            promise_id,
        });
        if self.seen_activation_keys.len() >= MAX_TRACKED_PROMISE_ROUTES {
            self.seen_activation_keys.pop_front();
        }
        self.seen_activation_keys
            .push_back(activation_key.to_owned());
        RouteEnqueueOutcome::Queued
    }

    fn peek(&self) -> Option<PromiseRouteDto> {
        self.pending.front().map(|route| PromiseRouteDto {
            route_id: route.route_id.clone(),
            promise_id: route.promise_id,
        })
    }

    fn acknowledge(&mut self, route_id: &str) -> Result<(), &'static str> {
        if self
            .acknowledged_route_ids
            .iter()
            .any(|completed| completed == route_id)
        {
            return Ok(());
        }
        let Some(current) = self.pending.front() else {
            return Err("no notification route is pending");
        };
        if current.route_id != route_id {
            return Err("notification route changed; retry");
        }
        let Some(completed) = self.pending.pop_front() else {
            return Err("notification route disappeared while acknowledged");
        };
        if self.acknowledged_route_ids.len() >= MAX_TRACKED_PROMISE_ROUTES {
            self.acknowledged_route_ids.pop_front();
        }
        self.acknowledged_route_ids.push_back(completed.route_id);
        Ok(())
    }
}

#[must_use]
pub fn pending_promise_routes() -> Arc<Mutex<PromiseRouteQueue>> {
    Arc::new(Mutex::new(PromiseRouteQueue::default()))
}

fn require_main_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err("notification routes are only available to the main window".into())
    }
}

/// Peeks the FIFO route head without consuming it.
#[tauri::command]
pub fn peek_pending_promise_route(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Option<PromiseRouteDto>, String> {
    require_main_window(&window)?;
    let routes = state
        .promise_routes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(routes.peek())
}

/// Acknowledges only the current FIFO route after the UI has handled it.
#[tauri::command]
pub fn ack_pending_promise_route(
    window: WebviewWindow,
    state: State<'_, AppState>,
    route_id: String,
) -> Result<(), String> {
    require_main_window(&window)?;
    let mut routes = state
        .promise_routes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    routes
        .acknowledge(&route_id)
        .map_err(std::string::ToString::to_string)
}

#[derive(Serialize)]
pub struct KillGateDto {
    pub id: String,
    pub status: String,
    pub notes: String,
}

#[derive(Serialize)]
pub struct QuickCaptureResult {
    pub capture_id: String,
    pub promise_id: i64,
}

#[derive(Serialize)]
pub struct PurgeSchedule {
    pub scheduled: bool,
}

/// Promise Inbox status projection requested by the desktop UI.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromiseTab {
    Open,
    Snoozed,
    Review,
    Resolved,
}

/// Lifecycle action requested from Promise Detail.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromiseInboxAction {
    Promote,
    Done,
    Snooze,
    NotAPromise,
    Ignore,
    Resume,
}

/// Lists one Promise Inbox tab without exposing raw captured messages.
#[tauri::command]
pub fn list_promises(
    state: State<'_, AppState>,
    tab: PromiseTab,
) -> Result<Vec<PromiseSummary>, String> {
    let scope = match tab {
        PromiseTab::Open => PromiseListScope::Open,
        PromiseTab::Snoozed => PromiseListScope::Snoozed,
        PromiseTab::Review => PromiseListScope::Review,
        PromiseTab::Resolved => PromiseListScope::Resolved,
    };
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    db.list_promises(scope).map_err(|error| error.to_string())
}

/// Loads one local Promise Detail record.
#[tauri::command]
pub fn get_promise(state: State<'_, AppState>, id: i64) -> Result<Option<PromiseDetail>, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    db.promise_detail(id).map_err(|error| error.to_string())
}

/// Saves user-editable promise text and an optional absolute deadline.
#[tauri::command]
pub fn update_promise(
    state: State<'_, AppState>,
    id: i64,
    expected_status: PromiseStatus,
    expected_ignore_count: u32,
    text: String,
    deadline: Option<i64>,
    deadline_timezone: Option<String>,
) -> Result<PromiseDetail, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    db.update_promise_details(
        id,
        expected_status,
        expected_ignore_count,
        &text,
        deadline,
        deadline_timezone.as_deref(),
        now_unix(),
    )
    .map_err(|error| error.to_string())?;
    required_promise_detail(&db, id)
}

/// Applies one lifecycle-valid Promise Detail action with stale-snapshot protection.
#[tauri::command]
pub fn act_on_promise(
    state: State<'_, AppState>,
    id: i64,
    expected_status: PromiseStatus,
    expected_ignore_count: u32,
    action: PromiseInboxAction,
    snooze_until: Option<i64>,
) -> Result<PromiseDetail, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    let now = now_unix();
    match action {
        PromiseInboxAction::Promote => {
            if snooze_until.is_some() || expected_status != PromiseStatus::Review {
                return Err("only a review promise can be promoted".into());
            }
            let current = required_promise_detail(&db, id)?;
            if current.summary.status != expected_status.as_str()
                || current.summary.ignore_count != expected_ignore_count
            {
                return Err("promise changed; refresh and try again".into());
            }
            apply_review(&db, id, expected_status, ReviewAction::Promote, "", now)
                .map_err(|error| error.to_string())?;
        }
        PromiseInboxAction::Resume => {
            if snooze_until.is_some() {
                return Err("resume does not accept a snooze time".into());
            }
            db.wake_snoozed_promise(id, expected_status, expected_ignore_count, now)
                .map_err(|error| error.to_string())?;
        }
        PromiseInboxAction::Done
        | PromiseInboxAction::Snooze
        | PromiseInboxAction::NotAPromise
        | PromiseInboxAction::Ignore => {
            let surface_action = match action {
                PromiseInboxAction::Done => SurfaceAction::Done,
                PromiseInboxAction::Snooze => SurfaceAction::Snooze,
                PromiseInboxAction::NotAPromise => SurfaceAction::Reject,
                PromiseInboxAction::Ignore => SurfaceAction::Ignore,
                PromiseInboxAction::Promote | PromiseInboxAction::Resume => unreachable!(),
            };
            let until = if matches!(action, PromiseInboxAction::Snooze) {
                Some(snooze_until.unwrap_or_else(|| now.saturating_add(DEFAULT_SNOOZE_SECONDS)))
            } else {
                if snooze_until.is_some() {
                    return Err("only snooze accepts a snooze time".into());
                }
                None
            };
            db.apply_direct_promise_action(
                id,
                expected_status,
                expected_ignore_count,
                surface_action,
                now,
                until,
            )
            .map_err(|error| error.to_string())?;
        }
    }
    required_promise_detail(&db, id)
}

fn required_promise_detail(db: &Database, id: i64) -> Result<PromiseDetail, String> {
    db.promise_detail(id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "promise was removed by retention; refresh the inbox".into())
}

/// Lists review-status promises.
#[tauri::command]
pub fn list_review(state: State<'_, AppState>) -> Result<Vec<ReviewItem>, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    db.list_by_status(PromiseStatus::Review)
        .map(|rows| {
            rows.into_iter()
                .map(|(id, text, source_app, recipient, score)| ReviewItem {
                    id,
                    text,
                    source_app,
                    recipient,
                    score,
                    status: "review".into(),
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

/// Promotes or rejects a review item.
#[tauri::command]
pub fn review_promise(
    state: State<'_, AppState>,
    id: i64,
    action: ReviewAction,
    text: String,
) -> Result<String, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    let next = apply_review(&db, id, PromiseStatus::Review, action, &text, now_unix())
        .map_err(|error| error.to_string())?;
    Ok(next.as_str().into())
}

/// Lists visible top-level Windows apps for the Phase 0 picker.
#[tauri::command]
pub fn list_focus_apps(window: WebviewWindow) -> Result<Vec<FocusAppDto>, String> {
    require_main_window(&window)?;
    crate::platform::focus::list_focus_apps()
        .map(|apps| {
            apps.into_iter()
                .map(|executable| FocusAppDto { executable })
                .collect()
        })
        .map_err(|error| error.to_string())
}

/// Lists Phase 0 hardcoded rules.
#[tauri::command]
pub fn list_phase0(state: State<'_, AppState>) -> Result<Vec<Phase0Rule>, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    db.list_phase0_rules()
        .map(|rows| {
            rows.into_iter()
                .map(|(id, app_match, reminder_text, enabled)| Phase0Rule {
                    id,
                    app_match,
                    reminder_text,
                    enabled,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

/// Sets whether one Phase 0 rule can surface and returns the stored row.
#[tauri::command]
pub fn set_phase0_rule_enabled(
    window: WebviewWindow,
    state: State<'_, AppState>,
    id: i64,
    enabled: bool,
) -> Result<Phase0Rule, String> {
    require_main_window(&window)?;
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    let (id, app_match, reminder_text, enabled) = db
        .set_phase0_rule_enabled(id, enabled)
        .map_err(|error| error.to_string())?;
    Ok(Phase0Rule {
        id,
        app_match,
        reminder_text,
        enabled,
    })
}

/// Adds a Phase 0 rule.
#[tauri::command]
pub fn add_phase0(
    state: State<'_, AppState>,
    app_match: String,
    reminder_text: String,
) -> Result<Phase0Rule, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    let (id, app_match, reminder_text, enabled) = db
        .insert_phase0_rule(&app_match, &reminder_text)
        .map_err(|error| error.to_string())?;
    Ok(Phase0Rule {
        id,
        app_match,
        reminder_text,
        enabled,
    })
}

/// Quick-capture from the shortcut window. Does not read selected text.
#[tauri::command]
pub fn quick_capture(
    state: State<'_, AppState>,
    capture_id: String,
    text: String,
) -> Result<QuickCaptureResult, String> {
    if !capture_id.starts_with("manual-") || capture_id.len() > 128 {
        return Err("quick-capture id is invalid".into());
    }
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    let promise_id =
        ingest_manual(&db, &capture_id, &text, now_unix()).map_err(|error| error.to_string())?;
    Ok(QuickCaptureResult {
        capture_id,
        promise_id,
    })
}

/// Settings upsert with validation.
#[tauri::command]
pub fn save_setting(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    validate_setting(&key, &value).map_err(|error| error.to_string())?;
    if key == "autostart_enabled" {
        let applied =
            apply_autostart(&state.app_exe, value == "true").map_err(|error| error.to_string())?;
        if !applied {
            return Err("autostart is not supported on this platform".into());
        }
    }
    {
        let db = state.db.lock().map_err(|_| "db poisoned")?;
        db.upsert_setting(&key, &value)
            .map_err(|error| error.to_string())?;
        if key == "retention_days" {
            db.enforce_retention(now_unix())
                .map_err(|error| error.to_string())?;
        }
    }
    if value == "false" {
        let disabled_site = match key.as_str() {
            "gmail_enabled" => Some("gmail"),
            "slack_enabled" => Some("slack"),
            _ => None,
        };
        if let Some(site) = disabled_site {
            let mut live = state
                .live_browser
                .lock()
                .map_err(|_| "browser context poisoned")?;
            if live
                .as_ref()
                .is_some_and(|entry| entry.context.source_app == site)
            {
                *live = None;
                let _ = state.browser_transition_tx.try_send(());
            }
        }
    }
    if key == "global_shortcut" || key == "global_shortcut_fallback" {
        reregister_shortcut(&app, &state)?;
    }
    Ok(())
}

/// Reads one setting.
#[tauri::command]
pub fn load_setting(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    db.get_setting(&key).map_err(|error| error.to_string())
}

/// Health snapshot, including the local-only listener claim.
#[tauri::command]
pub fn health(state: State<'_, AppState>) -> Result<HealthSnapshot, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    let gmail = db
        .get_setting("gmail_enabled")
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "true".into());
    let slack = db
        .get_setting("slack_enabled")
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "true".into());
    let now = chrono::Utc::now();
    let selector_records = db.selector_health().map_err(|error| error.to_string())?;
    let selectors = selector_records
        .into_iter()
        .map(|record| {
            let enabled = match record.site.as_str() {
                "gmail" => gmail != "false",
                "slack" => slack != "false",
                _ => false,
            };
            let days = crate::health::days_without_capture(
                now.timestamp(),
                record.first_observed_at,
                record.last_capture_at,
            );
            let status = if enabled {
                record.state
            } else {
                "disabled".into()
            };
            let banner = enabled
                .then(|| selector_banner(&record.site, &status, days))
                .flatten();
            crate::health::SelectorHealthSnapshot {
                site: record.site,
                status,
                first_observed_at: record.first_observed_at,
                last_probe_at: record.last_probe_at,
                last_success_at: record.last_success_at,
                last_capture_at: record.last_capture_at,
                consecutive_failures: record.consecutive_failures,
                days_without_capture: days,
                banner,
            }
        })
        .collect::<Vec<_>>();
    let site_status = |site: &str| {
        selectors
            .iter()
            .find(|selector| selector.site == site)
            .map(|selector| selector.status.clone())
            .unwrap_or_else(|| "unknown".into())
    };
    let silence_until = db
        .get_setting("onboarding_completed_at")
        .map_err(|error| error.to_string())?;
    let last_handshake_at = state
        .last_handshake_at
        .lock()
        .map_err(|_| "health poisoned")?
        .as_ref()
        .copied();
    Ok(HealthSnapshot {
        connection: if last_handshake_at.is_some() {
            "connected".into()
        } else {
            "disconnected".into()
        },
        native_host: if state.host_exe.exists() {
            "present".into()
        } else {
            "missing".into()
        },
        gmail: site_status("gmail"),
        slack: site_status("slack"),
        selectors,
        last_handshake_at,
        silence_remaining_secs: silence_remaining(now, silence_until.as_deref()),
        opens_network_listener: active_adapter().opens_network_listener(),
        shortcut: state
            .shortcut_status
            .lock()
            .map(|status| status.status_label())
            .unwrap_or_else(|_| "unavailable".into()),
    })
}

/// Selector banner text, content-free.
#[tauri::command]
pub fn health_banner(site: String, status: String, days: u32) -> Option<String> {
    selector_banner(&site, &status, days)
}

/// Idempotent native-host reconnect.
#[tauri::command]
pub fn reconnect_extension(state: State<'_, AppState>) -> Result<String, String> {
    reconnect(&state.host_exe)
        .map(|report| report.message)
        .map_err(|error| error.to_string())
}

/// Completes onboarding and starts the 30-minute silence window.
#[tauri::command]
pub fn complete_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    let until = chrono::Utc::now() + chrono::Duration::minutes(30);
    db.upsert_setting("onboarding_completed_at", &until.timestamp().to_string())
        .map_err(|error| error.to_string())
}

/// Lists human-time kill gates and their recorded evidence status.
#[tauri::command]
pub fn list_kill_gates(state: State<'_, AppState>) -> Result<Vec<KillGateDto>, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    kill_gate_dtos(&db)
}

/// Records a human kill-gate decision, enforcing prerequisite order.
#[tauri::command]
pub fn record_kill_gate(
    state: State<'_, AppState>,
    id: String,
    status: String,
    notes: String,
) -> Result<Vec<KillGateDto>, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    db.update_kill_gate(&id, &status, &notes)
        .map_err(|error| error.to_string())?;
    kill_gate_dtos(&db)
}

fn kill_gate_dtos(db: &Database) -> Result<Vec<KillGateDto>, String> {
    db.kill_gates()
        .map(|rows| {
            rows.into_iter()
                .map(|row| KillGateDto {
                    id: row.id,
                    status: row.status,
                    notes: row.notes,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

/// Schedules a helper process to purge local data after this process exits.
#[tauri::command]
pub fn purge_data(app: AppHandle, state: State<'_, AppState>) -> Result<PurgeSchedule, String> {
    let mut command = std::process::Command::new(&state.app_exe);
    command
        .arg("--purge")
        .arg("--db")
        .arg(&state.db_path)
        .arg("--manifest")
        .arg(state.host_exe.with_extension("json"))
        .arg("--wait-pid")
        .arg(std::process::id().to_string());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .spawn()
        .map_err(|error| format!("start purge helper: {error}"))?;
    app.exit(0);
    Ok(PurgeSchedule { scheduled: true })
}

/// Opens the existing `?window=quick` window. In-app fallback when the hotkey failed.
#[tauri::command]
pub fn open_quick_capture(app: AppHandle) -> Result<(), String> {
    open_quick_window(&app)
}

/// Installs the native host at first run.
///
/// # Errors
///
/// Returns IO errors from host registration.
pub fn first_run_install(host_exe: &std::path::Path) -> Result<(), String> {
    install_host(host_exe)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Reloads accelerators from settings and retries primary then fallback.
pub fn reregister_shortcut(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let plan = shortcut_plan_from_state(state)?;
    let outcome = register_on_app(app, &plan);
    if let Ok(mut status) = state.shortcut_status.lock() {
        *status = outcome;
    }
    Ok(())
}

pub fn shortcut_plan_from_state(state: &AppState) -> Result<ShortcutPlan, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    let primary = db.get_setting("global_shortcut").ok().flatten();
    let fallback = db.get_setting("global_shortcut_fallback").ok().flatten();
    Ok(ShortcutPlan::from_settings(
        primary.as_deref(),
        fallback.as_deref(),
    ))
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}
