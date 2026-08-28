//! Current-user Windows autostart via HKCU Run. Disclosure stays in Settings.

use std::io;
use std::path::Path;

/// Registry value name written under the current-user Run key.
pub const AUTOSTART_VALUE: &str = "Callback";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// Builds `reg.exe` arguments to enable or disable autostart.
#[must_use]
pub fn autostart_reg_args(exe: &Path, enabled: bool) -> Vec<String> {
    if enabled {
        vec![
            "add".into(),
            RUN_KEY.into(),
            "/v".into(),
            AUTOSTART_VALUE.into(),
            "/t".into(),
            "REG_SZ".into(),
            "/d".into(),
            exe.display().to_string(),
            "/f".into(),
        ]
    } else {
        vec![
            "delete".into(),
            RUN_KEY.into(),
            "/v".into(),
            AUTOSTART_VALUE.into(),
            "/f".into(),
        ]
    }
}

/// Applies the current-user Run key for Callback. No-op off Windows.
///
/// # Errors
///
/// Returns IO errors when `reg.exe` cannot be started.
pub fn apply_autostart(exe: &Path, enabled: bool) -> io::Result<bool> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new("reg")
            .args(autostart_reg_args(exe, enabled))
            .creation_flags(0x0800_0000)
            .status()?;
        Ok(status.success())
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, enabled);
        Ok(false)
    }
}
