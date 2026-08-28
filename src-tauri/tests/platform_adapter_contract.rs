use callback_lib::platform::{PlatformKind, active_adapter};

#[test]
fn active_adapter_matches_the_compiled_platform_gate() {
    let adapter = active_adapter();

    #[cfg(all(target_os = "windows", feature = "windows-platform"))]
    assert_eq!(adapter.kind(), PlatformKind::WindowsBaseline);

    #[cfg(not(all(target_os = "windows", feature = "windows-platform")))]
    assert_eq!(adapter.kind(), PlatformKind::UnsupportedNoop);
}

#[test]
fn baseline_adapter_initialization_is_side_effect_free() {
    let adapter = active_adapter();

    assert_eq!(adapter.initialize(), Ok(()));
    assert!(!adapter.opens_network_listener());
}
