pub mod commands;
pub mod db;
pub mod domain;
pub mod extraction;
pub mod health;
pub mod ipc;
pub mod lifecycle;
pub mod native_host;
pub mod platform;
pub mod purge;
pub mod review;
pub mod shortcut;
pub mod surfacing;
pub mod triggers;

use crate::commands::AppState;
use crate::db::Database;
use crate::ipc::commit::commit_envelope;
use crate::ipc::named_pipe::spawn_pipe_server;
use crate::platform::focus::{
    DebounceOutcome, FocusDebouncer, FocusEvent, FocusTarget, LiveBrowserContext,
    combine_live_focus, parse_browser_context,
};
use crate::shortcut::{ShortcutOutcome, ShortcutPlan};
use crate::surfacing::actions::ProtocolActivation;
use crate::surfacing::engine::{handle_dwell, handle_maintenance_tick};
use crate::surfacing::phase0::Phase0Rule;
use callback_protocol::MessageKind;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

const RETENTION_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Starts the Callback desktop runtime.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    run_with_activation(None);
}

/// Starts the desktop runtime and redeems an optional cold-start action after
/// the database has been recovered.
pub fn run_with_activation(initial_activation: Option<ProtocolActivation>) {
    let pending_activations = lifecycle::pending_activations();
    let pending_for_setup = Arc::clone(&pending_activations);
    tauri::Builder::default()
        .plugin(lifecycle::single_instance_plugin(pending_activations))
        .plugin(crate::shortcut::plugin())
        .setup(move |app| {
            platform::active_adapter().initialize()?;
            let app_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            std::fs::create_dir_all(&app_dir)?;
            let db_path = app_dir.join("callback.db");
            let db = Arc::new(Mutex::new(Database::open(&db_path)?));
            let app_exe = std::env::current_exe()?;
            let host_exe = app_exe
                .parent()
                .map(|dir| dir.join("callback-native-host.exe"))
                .ok_or_else(|| std::io::Error::other("missing install directory"))?;
            if let Err(error) = commands::first_run_install(&host_exe) {
                tracing::warn!(error = %error, path = %host_exe.display(), "native host registration failed");
            }
            let autostart_enabled = {
                let db = db.lock().map_err(|error| error.to_string())?;
                let now_utc = chrono::Utc::now();
                let now = now_utc.timestamp();
                if db.get_setting("timezone")?.is_none() {
                    let timezone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
                    let timezone = if timezone.parse::<chrono_tz::Tz>().is_ok() {
                        timezone
                    } else {
                        "UTC".into()
                    };
                    db.upsert_setting("timezone", &timezone)?;
                }
                if let Err(error) = db.enforce_retention(now) {
                    tracing::warn!(error = %error, "retention pass failed");
                }
                if let Err(error) = db.recover_leases(now) {
                    tracing::warn!(error = %error, "surface lease recovery failed");
                }
                db.get_setting("autostart_enabled")
                    .ok()
                    .flatten()
                    .is_some_and(|value| value == "true")
            };
            if let Err(error) = crate::native_host::autostart::apply_autostart(
                &app_exe,
                autostart_enabled,
            ) {
                tracing::warn!(error = %error, "autostart reconciliation failed");
            }
            let last_handshake_at = Arc::new(Mutex::new(None));
            let live_browser = Arc::new(Mutex::new(None::<LiveBrowserContext>));
            let (browser_transition_tx, browser_transition_rx) =
                std::sync::mpsc::sync_channel(32);
            let browser_transition_pipe = browser_transition_tx.clone();
            let default_plan = ShortcutPlan::from_settings(None, None);
            let shortcut_status = Arc::new(Mutex::new(ShortcutOutcome::Failed {
                primary: default_plan.primary.clone(),
                fallback: default_plan.fallback.clone(),
                reason: "not registered".into(),
            }));
            let db_pipe = Arc::clone(&db);
            let handshake_pipe = Arc::clone(&last_handshake_at);
            let browser_pipe = Arc::clone(&live_browser);
            spawn_pipe_server(Arc::new(move |envelope| {
                if envelope.kind == MessageKind::Handshake {
                    if let Ok(mut stamp) = handshake_pipe.lock() {
                        *stamp = Some(chrono::Utc::now().timestamp());
                    }
                }
                let db = db_pipe.lock().map_err(|_| "db poisoned".to_string())?;
                if envelope.kind == MessageKind::Context {
                    if let Some(parsed) = parse_browser_context(&envelope.payload) {
                        let enabled = db
                            .site_enabled(&parsed.source_app)
                            .map_err(|error| error.to_string())?;
                        let now = Instant::now();
                        let mut changed = false;
                        if let Ok(mut slot) = browser_pipe.lock() {
                            if enabled {
                                changed = slot
                                    .as_ref()
                                    .is_none_or(|live| live.context != parsed);
                                *slot = Some(LiveBrowserContext {
                                    context: parsed,
                                    received_at: now,
                                });
                            } else if slot
                                .as_ref()
                                .is_some_and(|live| live.context.source_app == parsed.source_app)
                            {
                                changed = true;
                                *slot = None;
                            }
                        }
                        if changed {
                            let _ = browser_transition_pipe.try_send(());
                        }
                    }
                }
                if envelope.kind == MessageKind::Probe {
                    let probe = crate::health::parse_selector_probe(&envelope.payload)
                        .ok_or_else(|| "invalid selector probe".to_string())?;
                    if db
                        .site_enabled(&probe.site)
                        .map_err(|error| error.to_string())?
                    {
                        db.record_selector_probe(&probe.site, probe.ok, probe.observed_at)
                            .map_err(|error| error.to_string())?;
                    }
                }
                commit_envelope(&db, envelope)
            }))?;
            let db_focus = Arc::clone(&db);
            let live_focus = Arc::clone(&live_browser);
            let focus_rx = platform::focus::spawn_focus_watcher();
            std::thread::spawn(move || {
                let mut debounce = FocusDebouncer::new(Duration::from_secs(5));
                let sink = platform::notifications::platform_sink();
                run_surface_maintenance(&db_focus, sink.as_ref(), &mut debounce);
                let mut last_maintenance = Instant::now();
                loop {
                    match focus_rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(FocusEvent::ForegroundPid(pid)) => {
                            let now = Instant::now();
                            let target = focus_target_for_pid(pid, &live_focus, now);
                            debounce.on_os_focus(target, now);
                        }
                        Ok(event) if event.invalidates_dwell() => {
                            let now = Instant::now();
                            debounce.apply_focus_event(&event, now);
                            if matches!(event, FocusEvent::SessionUnlock | FocusEvent::Resume) {
                                let target = current_focus_target(&live_focus, now);
                                debounce.on_os_focus(target, now);
                            }
                        }
                        Ok(_) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            tracing::warn!("focus watcher disconnected");
                            break;
                        }
                    }

                    let mut browser_changed = false;
                    while browser_transition_rx.try_recv().is_ok() {
                        browser_changed = true;
                    }
                    if browser_changed {
                        let now = Instant::now();
                        let target = current_focus_target(&live_focus, now);
                        debounce.on_os_focus(target, now);
                    }

                    let monotonic_now = Instant::now();
                    if monotonic_now.duration_since(last_maintenance) >= Duration::from_secs(1) {
                        run_surface_maintenance(&db_focus, sink.as_ref(), &mut debounce);
                        last_maintenance = monotonic_now;
                    }

                    if let DebounceOutcome::Fired(target) = debounce.on_tick(Instant::now()) {
                        run_focus_dwell(&db_focus, sink.as_ref(), &target);
                    }
                }
            });
            let db_retention = Arc::clone(&db);
            std::thread::spawn(move || loop {
                std::thread::sleep(RETENTION_MAINTENANCE_INTERVAL);
                run_retention_maintenance(&db_retention);
            });
            app.manage(AppState {
                db: Arc::clone(&db),
                db_path,
                host_exe,
                app_exe,
                last_handshake_at,
                shortcut_status: Arc::clone(&shortcut_status),
                live_browser: Arc::clone(&live_browser),
                browser_transition_tx,
                promise_routes: commands::pending_promise_routes(),
            });
            lifecycle::dispatch_initial_and_pending(
                app.handle(),
                initial_activation.as_ref(),
                &pending_for_setup,
            );
            lifecycle::install_tray(app)?;
            let plan = {
                let db = db.lock().map_err(|error| error.to_string())?;
                let primary = db.get_setting("global_shortcut").ok().flatten();
                let fallback = db
                    .get_setting("global_shortcut_fallback")
                    .ok()
                    .flatten();
                ShortcutPlan::from_settings(primary.as_deref(), fallback.as_deref())
            };
            let outcome = crate::shortcut::register_on_app(app.handle(), &plan);
            if let Ok(mut status) = shortcut_status.lock() {
                *status = outcome;
            }
            Ok(())
        })
        .on_window_event(lifecycle::handle_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::peek_pending_promise_route,
            commands::ack_pending_promise_route,
            commands::list_promises,
            commands::get_promise,
            commands::update_promise,
            commands::act_on_promise,
            commands::list_review,
            commands::review_promise,
            commands::list_focus_apps,
            commands::list_phase0,
            commands::set_phase0_rule_enabled,
            commands::add_phase0,
            commands::quick_capture,
            commands::save_setting,
            commands::load_setting,
            commands::health,
            commands::health_banner,
            commands::reconnect_extension,
            commands::complete_onboarding,
            commands::list_kill_gates,
            commands::record_kill_gate,
            commands::purge_data,
            commands::open_quick_capture,
        ])
        .run(tauri::generate_context!())
        .expect("Callback failed to start");
}

fn current_focus_target(
    live: &Mutex<Option<LiveBrowserContext>>,
    now: Instant,
) -> Option<FocusTarget> {
    let pid = platform::focus::current_foreground_pid()?;
    focus_target_for_pid(pid, live, now)
}

fn focus_target_for_pid(
    pid: u32,
    live: &Mutex<Option<LiveBrowserContext>>,
    now: Instant,
) -> Option<FocusTarget> {
    let path = platform::focus::resolve_process_image(pid).ok().flatten();
    let guard = live.lock().ok();
    combine_live_focus(path, guard.as_ref().and_then(|item| item.as_ref()), now)
}

fn run_retention_maintenance(database: &Arc<Mutex<Database>>) {
    let db = match database.lock() {
        Ok(db) => db,
        Err(error) => {
            tracing::error!(error = %error, "database lock poisoned during retention");
            return;
        }
    };
    match db.enforce_retention(chrono::Utc::now().timestamp()) {
        Ok(report) => tracing::debug!(
            cutoff_at = report.cutoff_at,
            deleted_captures = report.deleted_captures,
            deleted_receipts = report.deleted_receipts,
            redacted_captures = report.redacted_captures,
            redacted_promises = report.redacted_promises,
            "periodic retention completed"
        ),
        Err(error) => tracing::error!(error = %error, "periodic retention failed"),
    }
}

fn run_surface_maintenance(
    database: &Arc<Mutex<Database>>,
    sink: &dyn crate::platform::notifications::NotificationSink,
    debounce: &mut FocusDebouncer,
) {
    let db = match database.lock() {
        Ok(db) => db,
        Err(error) => {
            tracing::error!(error = %error, "database lock poisoned during surface maintenance");
            return;
        }
    };
    match handle_maintenance_tick(&db, sink, chrono::Utc::now()) {
        Ok(result) => {
            if result.reopened_snoozes != 0 {
                debounce.cancel_pending();
                tracing::debug!(
                    count = result.reopened_snoozes,
                    "due snoozes reopened; pending dwell cancelled"
                );
            }
            if result.deadline_surface.is_some() {
                tracing::debug!("deadline maintenance evaluated a surface");
            }
        }
        Err(error) => {
            tracing::error!(error = %error, "surface maintenance failed");
        }
    }
}

fn run_focus_dwell(
    database: &Arc<Mutex<Database>>,
    sink: &dyn crate::platform::notifications::NotificationSink,
    target: &FocusTarget,
) {
    let db = match database.lock() {
        Ok(db) => db,
        Err(error) => {
            tracing::error!(error = %error, "database lock poisoned during focus dwell");
            return;
        }
    };
    let rules = match db.list_phase0_rules() {
        Ok(rows) => rows
            .into_iter()
            .map(|(id, app_match, reminder_text, enabled)| Phase0Rule {
                id,
                app_match,
                reminder_text,
                enabled,
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            tracing::error!(error = %error, "failed to load Phase 0 rules");
            return;
        }
    };
    if let Err(error) = handle_dwell(&db, sink, target, chrono::Utc::now(), &rules) {
        tracing::error!(error = %error, "focus dwell surfacing failed");
    }
}
