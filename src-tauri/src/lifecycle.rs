//! Windows desktop lifecycle integration for local notification actions.

use crate::commands::AppState;
use crate::surfacing::actions::{ActionActivation, dispatch_activation, parse_cold_start_args};
use chrono::Offset;
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Runtime, Window, WindowEvent};
#[cfg(all(debug_assertions, windows))]
use tauri_plugin_deep_link::DeepLinkExt;

const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_OPEN_ID: &str = "callback-tray-open";
const TRAY_QUIT_ID: &str = "callback-tray-quit";

pub type PendingActivations = Arc<Mutex<Vec<ActionActivation>>>;

#[must_use]
pub fn pending_activations() -> PendingActivations {
    Arc::new(Mutex::new(Vec::new()))
}

/// Builds the first plugin in the Tauri chain and forwards second-instance
/// protocol arguments to the already-running application.
#[must_use]
pub fn single_instance_plugin(
    pending: PendingActivations,
) -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_single_instance::init(move |app, args, _working_directory| {
        let activation = match parse_cold_start_args(&args) {
            Ok(activation) => activation,
            Err(error) => {
                tracing::warn!(error = %error, "ignored malformed forwarded Callback action");
                None
            }
        };

        if let Some(activation) = activation {
            let mut queued = match pending.lock() {
                Ok(queued) => queued,
                Err(error) => {
                    tracing::error!(error = %error, "forwarded activation queue is poisoned");
                    return;
                }
            };
            if app.try_state::<AppState>().is_some() {
                drop(queued);
                redeem_managed_activation(app, &activation, "warm-single-instance");
            } else {
                queued.push(activation);
                tracing::debug!("queued forwarded Callback action until setup completes");
            }
        }

        if let Err(error) = show_main_window(app) {
            tracing::warn!(error = %error, "could not foreground Callback after second launch");
        }
    })
}

/// Registers configured development schemes. Release installers own installed
/// protocol registration through `tauri.conf.json`.
pub fn register_debug_deep_links(app: &App) -> Result<(), String> {
    #[cfg(all(debug_assertions, windows))]
    {
        app.deep_link()
            .register_all()
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(all(debug_assertions, windows)))]
    let _ = app;
    Ok(())
}

/// Redeems the cold-start action followed by actions that arrived while setup
/// was still creating managed state.
pub fn dispatch_initial_and_pending(
    app: &AppHandle,
    initial: Option<&ActionActivation>,
    pending: &PendingActivations,
) {
    if let Some(activation) = initial {
        redeem_managed_activation(app, activation, "cold-start");
    }

    let queued = match pending.lock() {
        Ok(mut queued) => queued.drain(..).collect::<Vec<_>>(),
        Err(error) => {
            tracing::error!(error = %error, "forwarded activation queue is poisoned");
            return;
        }
    };
    for activation in queued {
        redeem_managed_activation(app, &activation, "queued-single-instance");
    }
}

fn redeem_managed_activation(app: &AppHandle, activation: &ActionActivation, source: &'static str) {
    let Some(state) = app.try_state::<AppState>() else {
        tracing::error!(source, "managed state unavailable for Callback action");
        return;
    };
    let db = match state.db.lock() {
        Ok(db) => db,
        Err(error) => {
            tracing::error!(source, error = %error, "database lock poisoned during Callback action");
            return;
        }
    };
    let now_utc = chrono::Utc::now();
    let now = now_utc.timestamp();
    let timezone = match db.get_setting("timezone") {
        Ok(timezone) => timezone
            .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
            .unwrap_or(chrono_tz::UTC),
        Err(error) => {
            tracing::error!(source, error = %error, "could not load timezone for Callback action");
            return;
        }
    };
    let offset = now_utc
        .with_timezone(&timezone)
        .offset()
        .fix()
        .local_minus_utc();
    let local_day = crate::surfacing::rate_limit::local_day(now_utc, offset);
    match dispatch_activation(&db, activation, now, &local_day) {
        Ok(result) => tracing::info!(source, ?result, "notification action handled"),
        Err(error) => {
            tracing::error!(source, error = %error, "notification action failed");
        }
    }
}

/// Shows, restores, and focuses the configured main window.
pub fn show_main_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| "main window is unavailable".to_owned())?;
    window.unminimize().map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

/// Creates the persistent tray affordance required before close-to-background
/// is enabled.
pub fn install_tray(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| std::io::Error::other("configured tray icon is unavailable"))?;
    let open = MenuItem::with_id(app, TRAY_OPEN_ID, "Open Callback", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_ID, "Quit Callback", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            TRAY_OPEN_ID => {
                if let Err(error) = show_main_window(app) {
                    tracing::warn!(error = %error, "tray could not open Callback");
                }
            }
            TRAY_QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                if let Err(error) = show_main_window(tray.app_handle()) {
                    tracing::warn!(error = %error, "tray click could not open Callback");
                }
            }
        })
        .build(app)?;
    Ok(())
}

/// Keeps the main application resident while allowing auxiliary windows to
/// retain their normal close behavior.
pub fn handle_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW_LABEL {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        if let Err(error) = window.hide() {
            tracing::error!(error = %error, "could not hide Callback main window");
        }
    }
}
