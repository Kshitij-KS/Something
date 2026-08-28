pub mod commands;
pub mod db;
pub mod domain;
pub mod extraction;
pub mod health;
pub mod ipc;
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
    DebounceOutcome, FocusDebouncer, FocusEvent, LiveBrowserContext, combine_live_focus,
    parse_browser_context,
};
use crate::shortcut::{ShortcutOutcome, ShortcutPlan};
use crate::surfacing::engine::handle_dwell;
use crate::surfacing::phase0::Phase0Rule;
use callback_protocol::MessageKind;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

/// Starts the Callback desktop runtime.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(crate::shortcut::plugin())
        .setup(|app| {
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
            let _ = commands::first_run_install(&host_exe);
            {
                let db = db.lock().map_err(|error| error.to_string())?;
                let _ = db.recover_leases(chrono::Utc::now().timestamp());
            }
            let last_handshake_at = Arc::new(Mutex::new(None));
            let live_browser = Arc::new(Mutex::new(None::<LiveBrowserContext>));
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
                if envelope.kind == MessageKind::Context {
                    if let Some(parsed) = parse_browser_context(&envelope.payload) {
                        if let Ok(mut slot) = browser_pipe.lock() {
                            *slot = Some(LiveBrowserContext {
                                context: parsed,
                                received_at: Instant::now(),
                            });
                        }
                    }
                }
                let db = db_pipe.lock().map_err(|_| "db poisoned".to_string())?;
                commit_envelope(&db, envelope)
            }))?;
            let db_focus = Arc::clone(&db);
            let live_focus = Arc::clone(&live_browser);
            let focus_rx = platform::focus::spawn_focus_watcher();
            std::thread::spawn(move || {
                let mut debounce = FocusDebouncer::new(Duration::from_secs(5));
                let sink = platform::notifications::platform_sink();
                loop {
                    match focus_rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(FocusEvent::ForegroundPid(pid)) => {
                            let path = platform::focus::resolve_process_image(pid).ok().flatten();
                            let live = live_focus.lock().ok();
                            let target = combine_live_focus(
                                path,
                                live.as_ref().and_then(|guard| guard.as_ref()),
                                Instant::now(),
                            );
                            debounce.on_os_focus(target, Instant::now());
                        }
                        Ok(event) if event.invalidates_dwell() => {
                            debounce.apply_focus_event(&event, Instant::now());
                            if matches!(event, FocusEvent::SessionUnlock | FocusEvent::Resume) {
                                if let Some(pid) = platform::focus::current_foreground_pid() {
                                    let path =
                                        platform::focus::resolve_process_image(pid).ok().flatten();
                                    let live = live_focus.lock().ok();
                                    let target = combine_live_focus(
                                        path,
                                        live.as_ref().and_then(|guard| guard.as_ref()),
                                        Instant::now(),
                                    );
                                    debounce.on_os_focus(target, Instant::now());
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    if let DebounceOutcome::Fired(target) = debounce.on_tick(Instant::now()) {
                        if let Ok(db) = db_focus.lock() {
                            let rules = db
                                .list_phase0_rules()
                                .unwrap_or_default()
                                .into_iter()
                                .map(|(id, app_match, reminder_text, enabled)| Phase0Rule {
                                    id,
                                    app_match,
                                    reminder_text,
                                    enabled,
                                })
                                .collect::<Vec<_>>();
                            let _ = handle_dwell(
                                &db,
                                sink.as_ref(),
                                &target,
                                chrono::Utc::now(),
                                &rules,
                            );
                        }
                    }
                }
            });
            app.manage(AppState {
                db: Arc::clone(&db),
                db_path,
                host_exe,
                app_exe,
                last_handshake_at,
                shortcut_status: Arc::clone(&shortcut_status),
            });
            let plan = {
                let db = db.lock().map_err(|error| error.to_string())?;
                let primary = db.get_setting("global_shortcut").ok().flatten();
                let fallback = db.get_setting("global_shortcut_fallback").ok().flatten();
                ShortcutPlan::from_settings(primary.as_deref(), fallback.as_deref())
            };
            let outcome = crate::shortcut::register_on_app(app.handle(), &plan);
            if let Ok(mut status) = shortcut_status.lock() {
                *status = outcome;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_review,
            commands::review_promise,
            commands::list_phase0,
            commands::add_phase0,
            commands::quick_capture,
            commands::save_setting,
            commands::load_setting,
            commands::health,
            commands::health_banner,
            commands::reconnect_extension,
            commands::complete_onboarding,
            commands::list_kill_gates,
            commands::purge_data,
            commands::open_quick_capture,
        ])
        .run(tauri::generate_context!())
        .expect("Callback failed to start");
}
