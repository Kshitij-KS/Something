//! Compile-safe focus lookup for non-Windows targets.

use super::{FocusError, FocusEvent};
use std::sync::mpsc::SyncSender;

pub fn list_focus_apps() -> Result<Vec<String>, FocusError> {
    Ok(Vec::new())
}

pub fn resolve_process_image(_pid: u32) -> Result<Option<String>, FocusError> {
    Ok(None)
}

pub fn spawn_watcher(_tx: SyncSender<FocusEvent>) {}

pub fn current_foreground_pid() -> Option<u32> {
    None
}
