#![cfg_attr(windows, allow(unsafe_code))]

use crate::native_host::autostart::apply_autostart;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Result of a local data purge.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PurgeReport {
    pub deleted_db: bool,
    pub deleted_manifest: bool,
    pub unregistered_host: bool,
    pub autostart_removed: bool,
}

/// Deletes a closed SQLite database and all of its journaling sidecars.
///
/// This file-only helper intentionally does not alter the real user registry,
/// which keeps integration tests isolated. Production CLI purge uses
/// [`purge_from_args`] and also removes registration and autostart.
///
/// # Errors
///
/// Returns the first filesystem error other than `NotFound`.
pub fn purge_local_data_path(db_path: &Path) -> Result<PurgeReport, io::Error> {
    remove_database_files(db_path)?;
    Ok(PurgeReport {
        deleted_db: database_files_absent(db_path),
        deleted_manifest: false,
        unregistered_host: false,
        autostart_removed: false,
    })
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

/// Parses and executes `--purge [--db <path>] [--wait-pid <pid>]`.
///
/// The GUI starts this helper as a second process and exits. Waiting for the
/// parent first ensures SQLite and its WAL are closed before deletion on
/// Windows. `--skip-registration` is reserved for isolated tests and skips
/// all real-user platform cleanup, including notification history.
///
/// # Errors
///
/// Returns an error for malformed arguments, a parent wait timeout, failed
/// deletion, or failed current-user registry cleanup.
pub fn purge_from_args(args: &[String]) -> Result<PurgeReport, io::Error> {
    if !args.iter().any(|arg| arg == "--purge") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing --purge",
        ));
    }

    let skip_platform_cleanup = args.iter().any(|arg| arg == "--skip-registration");
    if let Some(raw_pid) = arg_value(args, "--wait-pid") {
        let pid = raw_pid
            .parse::<u32>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid --wait-pid value"))?;
        wait_for_parent(pid)?;
    }
    let history_cleanup_error = if skip_platform_cleanup {
        None
    } else {
        crate::platform::notifications::clear_callback_history()
            .err()
            .map(|error| io::Error::other(error.to_string()))
    };

    let db_path = arg_value(args, "--db")
        .map(PathBuf::from)
        .unwrap_or_else(default_db_path);
    let manifest_path = arg_value(args, "--manifest")
        .map(PathBuf::from)
        .or_else(default_manifest_path);

    remove_database_files(&db_path)?;
    let deleted_manifest = if let Some(path) = manifest_path {
        remove_if_exists(&path)?;
        remove_if_exists(&path.with_extension("json.tmp"))?;
        !path.exists()
    } else {
        false
    };

    #[cfg(windows)]
    let (unregistered_host, autostart_removed) = if skip_platform_cleanup {
        (false, false)
    } else {
        (unregister_host()?, apply_autostart(Path::new(""), false)?)
    };
    #[cfg(not(windows))]
    let (unregistered_host, autostart_removed) = {
        let _ = skip_platform_cleanup;
        (false, false)
    };

    if let Some(error) = history_cleanup_error {
        return Err(error);
    }
    Ok(PurgeReport {
        deleted_db: database_files_absent(&db_path),
        deleted_manifest,
        unregistered_host,
        autostart_removed,
    })
}

fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn default_manifest_path() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|directory| directory.join("callback-native-host.json"))
    })
}

fn database_paths(db_path: &Path) -> [PathBuf; 4] {
    [
        db_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", db_path.display())),
        PathBuf::from(format!("{}-shm", db_path.display())),
        PathBuf::from(format!("{}-journal", db_path.display())),
    ]
}

fn remove_database_files(db_path: &Path) -> io::Result<()> {
    for path in database_paths(db_path) {
        remove_if_exists(&path)?;
    }
    Ok(())
}

fn database_files_absent(db_path: &Path) -> bool {
    database_paths(db_path).iter().all(|path| !path.exists())
}

fn remove_if_exists(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("delete {}: {error}", path.display()),
        )),
    }
}

#[cfg(windows)]
fn wait_for_parent(pid: u32) -> io::Result<()> {
    if pid == std::process::id() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "purge helper cannot wait for itself",
        ));
    }

    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;
    const WAIT_FAILED: u32 = u32::MAX;
    const ERROR_INVALID_PARAMETER: i32 = 87;
    const WAIT_MS: u32 = 30_000;

    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit_handle: i32, process_id: u32) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }

    // SAFETY: the returned process handle is checked and closed exactly once.
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) {
            return Ok(());
        }
        return Err(error);
    }

    // SAFETY: `handle` is a valid synchronization handle from OpenProcess.
    let wait = unsafe { WaitForSingleObject(handle, WAIT_MS) };
    // SAFETY: `handle` is no longer used after this close.
    let _ = unsafe { CloseHandle(handle) };
    match wait {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "timed out waiting for the Callback process to exit",
        )),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        other => Err(io::Error::other(format!(
            "unexpected process wait result {other}"
        ))),
    }
}

#[cfg(not(windows))]
fn wait_for_parent(pid: u32) -> io::Result<()> {
    if pid == std::process::id() {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "purge helper cannot wait for itself",
        ))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn unregister_host() -> io::Result<bool> {
    const KEY: &str = r"HKCU\Software\Google\Chrome\NativeMessagingHosts\com.callback.host";
    if !run_reg(&["query", KEY])?.success() {
        return Ok(true);
    }
    let status = run_reg(&["delete", KEY, "/f"])?;
    if status.success() {
        Ok(true)
    } else {
        Err(io::Error::other(format!(
            "reg.exe rejected native-host removal with {status}"
        )))
    }
}

#[cfg(windows)]
fn run_reg(args: &[&str]) -> io::Result<std::process::ExitStatus> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg")
        .args(args)
        .creation_flags(0x0800_0000)
        .status()
}
