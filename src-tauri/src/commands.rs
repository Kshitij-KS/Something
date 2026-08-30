use crate::db::{Database, validate_setting};
use crate::domain::PromiseStatus;
use crate::health::{HealthSnapshot, selector_banner, silence_remaining};
use crate::native_host::autostart::apply_autostart;
use crate::native_host::install::{install_host, reconnect};
use crate::platform::active_adapter;
use crate::platform::focus::LiveBrowserContext;
use crate::review::{ReviewAction, ReviewItem, apply_review, ingest_manual};
use crate::shortcut::{ShortcutOutcome, ShortcutPlan, open_quick_window, register_on_app};
use crate::surfacing::phase0::Phase0Rule;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};

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

/// Adds a Phase 0 rule.
#[tauri::command]
pub fn add_phase0(
    state: State<'_, AppState>,
    app_match: String,
    reminder_text: String,
) -> Result<i64, String> {
    let db = state.db.lock().map_err(|_| "db poisoned")?;
    db.insert_phase0_rule(&app_match, &reminder_text)
        .map_err(|error| error.to_string())
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
