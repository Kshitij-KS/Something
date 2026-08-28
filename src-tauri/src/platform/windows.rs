use super::{PlatformAdapter, PlatformError, PlatformKind};

pub(super) static WINDOWS_ADAPTER: WindowsAdapter = WindowsAdapter;

pub(super) struct WindowsAdapter;

impl PlatformAdapter for WindowsAdapter {
    fn kind(&self) -> PlatformKind {
        PlatformKind::WindowsBaseline
    }

    fn initialize(&self) -> Result<(), PlatformError> {
        Ok(())
    }

    fn opens_network_listener(&self) -> bool {
        false
    }
}
