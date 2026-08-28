use crate::db::Database;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Result of a local data purge.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PurgeReport {
    pub deleted_db: bool,
    pub unregistered_host: bool,
}

/// Deletes the SQLite file and unregisters the native host. Does not transmit data.
///
/// # Errors
///
/// Returns IO errors when files cannot be removed.
pub fn purge_local_data_path(db_path: &Path) -> Result<PurgeReport, std::io::Error> {
    let _ = fs::remove_file(db_path);
    let _ = fs::remove_file(format!("{}-wal", db_path.display()));
    let _ = fs::remove_file(format!("{}-shm", db_path.display()));
    #[cfg(windows)]
    let unregistered = unregister_host();
    #[cfg(not(windows))]
    let unregistered = false;
    Ok(PurgeReport {
        deleted_db: !db_path.exists(),
        unregistered_host: unregistered,
    })
}

/// Deletes the SQLite file after dropping the writer.
///
/// # Errors
///
/// Returns IO errors when files cannot be removed.
pub fn purge_local_data(_db: &Database, db_path: &Path) -> Result<PurgeReport, std::io::Error> {
    purge_local_data_path(db_path)
}

/// Default SQLite path used by the Tauri identifier `com.callback.desktop`.
#[must_use]
pub fn default_db_path() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.callback.desktop")
            .join("callback.db")
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/share/com.callback.desktop/callback.db")
    }
}

/// Parses `--purge [--db <path>]` from process arguments.
///
/// # Errors
///
/// Returns IO errors from [`purge_local_data_path`], or when `--purge` is missing.
pub fn purge_from_args(args: &[String]) -> Result<PurgeReport, std::io::Error> {
    if !args.iter().any(|arg| arg == "--purge") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "missing --purge",
        ));
    }
    let db = args
        .windows(2)
        .find(|pair| pair[0] == "--db")
        .map(|pair| PathBuf::from(&pair[1]))
        .unwrap_or_else(default_db_path);
    purge_local_data_path(&db)
}

#[cfg(windows)]
fn unregister_host() -> bool {
    std::process::Command::new("reg")
        .args([
            "delete",
            r"HKCU\Software\Google\Chrome\NativeMessagingHosts\com.callback.host",
            "/f",
        ])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
