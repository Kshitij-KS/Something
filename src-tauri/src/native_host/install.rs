//! Atomic native-messaging host manifest registration.

use serde_json::json;
use std::fs;
use std::io;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

/// Host manifest name used in Chrome registration.
pub const HOST_NAME: &str = "com.callback.host";
/// Pinned development origin.
pub const ALLOWED_ORIGIN: &str = "chrome-extension://difdpnmogohnpilhjlihgficnebdjphg/";

/// Result of writing and registering the host manifest.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstallReport {
    pub manifest_path: String,
    pub registered: bool,
    pub message: String,
}

/// Writes the host manifest next to the native-host binary and registers it.
///
/// # Errors
///
/// Returns IO errors when the manifest cannot be written.
pub fn install_host(host_exe: &Path) -> io::Result<InstallReport> {
    let manifest_path = host_exe.with_extension("json");
    let manifest = json!({
        "name": HOST_NAME,
        "description": "Callback native messaging host",
        "path": host_exe.display().to_string(),
        "type": "stdio",
        "allowed_origins": [ALLOWED_ORIGIN],
    });
    let tmp = manifest_path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&manifest)?)?;
    fs::rename(&tmp, &manifest_path)?;
    let registered = register(&manifest_path)?;
    Ok(InstallReport {
        manifest_path: manifest_path.display().to_string(),
        registered,
        message: if registered {
            "Native host registered".into()
        } else {
            "Manifest written; registry registration skipped on this platform".into()
        },
    })
}

/// Idempotent reconnect used by diagnostics.
///
/// # Errors
///
/// Returns IO errors when the manifest cannot be rewritten.
pub fn reconnect(host_exe: &Path) -> io::Result<InstallReport> {
    install_host(host_exe)
}

fn register(manifest_path: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    {
        register_windows(manifest_path)
    }
    #[cfg(target_os = "macos")]
    {
        let dest = macos_path()?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(manifest_path, dest)?;
        Ok(true)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dest = linux_path()?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(manifest_path, dest)?;
        Ok(true)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = manifest_path;
        Ok(false)
    }
}

#[cfg(windows)]
fn register_windows(manifest_path: &Path) -> io::Result<bool> {
    use std::os::windows::process::CommandExt;
    let path = manifest_path.display().to_string().replace('\\', "\\\\");
    let status = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Google\Chrome\NativeMessagingHosts\com.callback.host",
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            &manifest_path.display().to_string(),
            "/f",
        ])
        .creation_flags(0x0800_0000)
        .status()?;
    let _ = path;
    Ok(status.success())
}

#[cfg(target_os = "macos")]
fn macos_path() -> io::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|error| io::Error::other(error))?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support/Google/Chrome/NativeMessagingHosts")
        .join(format!("{HOST_NAME}.json")))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_path() -> io::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|error| io::Error::other(error))?;
    Ok(PathBuf::from(home)
        .join(".config/google-chrome/NativeMessagingHosts")
        .join(format!("{HOST_NAME}.json")))
}
