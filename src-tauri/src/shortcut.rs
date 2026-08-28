//! Global shortcut policy for the existing `?window=quick` capture surface.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Default accelerator requested by the spec.
pub const DEFAULT_PRIMARY: &str = "Ctrl+Shift+K";
/// Configurable fallback when the primary accelerator is already claimed.
pub const DEFAULT_FALLBACK: &str = "Ctrl+Alt+K";
/// Tauri window label for the quick-capture surface.
pub const QUICK_WINDOW_LABEL: &str = "quick";
/// URL that the existing React tree already treats as quick capture.
pub const QUICK_WINDOW_URL: &str = "index.html?window=quick";

/// How the quick-capture window is opened. Never silently reads other apps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickWindowSpec {
    pub label: &'static str,
    pub url: &'static str,
    pub reads_selected_text: bool,
    pub auto_prefill_clipboard: bool,
}

/// Returns the existing quick-capture window contract.
#[must_use]
pub const fn open_quick_spec() -> QuickWindowSpec {
    QuickWindowSpec {
        label: QUICK_WINDOW_LABEL,
        url: QUICK_WINDOW_URL,
        reads_selected_text: false,
        auto_prefill_clipboard: false,
    }
}

/// Primary plus fallback accelerators loaded from settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutPlan {
    pub primary: String,
    pub fallback: String,
}

impl ShortcutPlan {
    /// Builds a plan from persisted settings, using spec defaults when unset.
    #[must_use]
    pub fn from_settings(primary: Option<&str>, fallback: Option<&str>) -> Self {
        Self {
            primary: normalize_accel(primary).unwrap_or_else(|| DEFAULT_PRIMARY.into()),
            fallback: normalize_accel(fallback).unwrap_or_else(|| DEFAULT_FALLBACK.into()),
        }
    }
}

fn normalize_accel(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Result of attempting to register the global shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutOutcome {
    RegisteredPrimary {
        accelerator: String,
    },
    RegisteredFallback {
        accelerator: String,
        reason: String,
    },
    Failed {
        primary: String,
        fallback: String,
        reason: String,
    },
}

impl ShortcutOutcome {
    #[must_use]
    pub const fn is_registered(&self) -> bool {
        matches!(
            self,
            Self::RegisteredPrimary { .. } | Self::RegisteredFallback { .. }
        )
    }

    /// Short diagnostic string for Health/Settings.
    #[must_use]
    pub fn status_label(&self) -> String {
        match self {
            Self::RegisteredPrimary { accelerator } => accelerator.clone(),
            Self::RegisteredFallback {
                accelerator,
                reason,
            } => format!("fallback:{accelerator} ({reason})"),
            Self::Failed { reason, .. } => format!("unavailable ({reason})"),
        }
    }
}

/// Tries the primary accelerator, then the configured fallback.
pub fn choose_registration(
    plan: &ShortcutPlan,
    mut try_register: impl FnMut(&str) -> Result<(), String>,
) -> ShortcutOutcome {
    match try_register(&plan.primary) {
        Ok(()) => ShortcutOutcome::RegisteredPrimary {
            accelerator: plan.primary.clone(),
        },
        Err(primary_reason) => match try_register(&plan.fallback) {
            Ok(()) => ShortcutOutcome::RegisteredFallback {
                accelerator: plan.fallback.clone(),
                reason: primary_reason,
            },
            Err(fallback_reason) => ShortcutOutcome::Failed {
                primary: plan.primary.clone(),
                fallback: plan.fallback.clone(),
                reason: fallback_reason,
            },
        },
    }
}

/// Opens or focuses the existing `?window=quick` surface. Never reads other apps.
///
/// # Errors
///
/// Returns a string when the window cannot be created or focused.
pub fn open_quick_window(app: &AppHandle) -> Result<(), String> {
    let spec = open_quick_spec();
    if let Some(existing) = app.get_webview_window(spec.label) {
        let _ = existing.unminimize();
        existing.show().map_err(|error| error.to_string())?;
        existing.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }
    WebviewWindowBuilder::new(app, spec.label, WebviewUrl::App(spec.url.into()))
        .title("Quick capture")
        .inner_size(440.0, 320.0)
        .build()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Registers the plugin handler that opens quick capture on any Callback shortcut.
#[must_use]
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                let _ = open_quick_window(app);
            }
        })
        .build()
}

/// Attempts primary then fallback registration on the live Tauri runtime.
#[must_use]
pub fn register_on_app(app: &AppHandle, plan: &ShortcutPlan) -> ShortcutOutcome {
    let _ = app.global_shortcut().unregister_all();
    choose_registration(plan, |accel| {
        app.global_shortcut()
            .register(accel)
            .map_err(|error| error.to_string())
    })
}
