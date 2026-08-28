pub mod focus;
pub mod notifications;

#[cfg(not(all(target_os = "windows", feature = "windows-platform")))]
mod noop;
#[cfg(all(target_os = "windows", feature = "windows-platform"))]
mod windows;

use std::error::Error;
use std::fmt::{Display, Formatter};

#[cfg(not(all(target_os = "windows", feature = "windows-platform")))]
use noop::NOOP_ADAPTER as ACTIVE_ADAPTER;
#[cfg(all(target_os = "windows", feature = "windows-platform"))]
use windows::WINDOWS_ADAPTER as ACTIVE_ADAPTER;

/// Identifies which compile-time platform boundary is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    /// Windows is selected, but focus and notification behavior remain gated to U3.
    WindowsBaseline,
    /// The current platform deliberately supplies no runtime integration.
    UnsupportedNoop,
}

/// Reserved initialization error for future platform integrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformError;

impl Display for PlatformError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("platform initialization failed")
    }
}

impl Error for PlatformError {}

/// Compile-safe boundary for platform-specific runtime integrations.
pub trait PlatformAdapter: Sync {
    /// Reports the selected compile-time adapter.
    fn kind(&self) -> PlatformKind;

    /// Initializes the adapter without opening a network listener.
    ///
    /// # Errors
    ///
    /// Future adapters may report platform initialization failures.
    fn initialize(&self) -> Result<(), PlatformError>;

    /// Declares whether initialization opens a TCP or UDP listener.
    fn opens_network_listener(&self) -> bool;
}

/// Returns the adapter selected by the operating-system and Cargo feature gates.
#[must_use]
pub fn active_adapter() -> &'static dyn PlatformAdapter {
    &ACTIVE_ADAPTER
}
