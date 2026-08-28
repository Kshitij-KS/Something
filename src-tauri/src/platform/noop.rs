use super::{PlatformAdapter, PlatformError, PlatformKind};

pub(super) static NOOP_ADAPTER: NoopAdapter = NoopAdapter;

pub(super) struct NoopAdapter;

impl PlatformAdapter for NoopAdapter {
    fn kind(&self) -> PlatformKind {
        PlatformKind::UnsupportedNoop
    }

    fn initialize(&self) -> Result<(), PlatformError> {
        Ok(())
    }

    fn opens_network_listener(&self) -> bool {
        false
    }
}
