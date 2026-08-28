//! Windows process identity lookup. Callbacks stay off this path.

#![allow(unsafe_code)]

use super::{FocusError, FocusEvent};
use std::sync::OnceLock;
use std::sync::mpsc::SyncSender;
use std::thread;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, MSG, RegisterClassW,
    TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_DESTROY,
    WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
use windows::core::{PCWSTR, PWSTR, w};

const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;
const PBT_APMSUSPEND: usize = 0x0004;
const PBT_APMRESUMESUSPEND: usize = 0x0007;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;

static EVENT_TX: OnceLock<SyncSender<FocusEvent>> = OnceLock::new();

/// Resolves a PID without panicking when the process is protected.
///
/// # Errors
///
/// Returns [`FocusError::LookupFailed`] only for unexpected conversion errors.
pub fn resolve_process_image(pid: u32) -> Result<Option<String>, FocusError> {
    if pid == 0 {
        return Ok(None);
    }
    // SAFETY: `pid` is an OS-supplied process id. `OpenProcess` with limited
    // query rights is valid and returns an error handle on access denial.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
    let Ok(handle) = handle else {
        return Ok(None);
    };
    let result = image_from_handle(handle);
    // SAFETY: `handle` was opened by this function and is not used after close.
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}

/// Starts an out-of-context WinEvent hook plus a hidden sink window for lock/sleep.
pub fn spawn_watcher(tx: SyncSender<FocusEvent>) {
    let _ = EVENT_TX.set(tx);
    let _ = thread::Builder::new()
        .name("callback-focus".into())
        .spawn(message_loop);
}

/// Current foreground PID used to restart dwell after unlock/resume.
pub fn current_foreground_pid() -> Option<u32> {
    // SAFETY: GetForegroundWindow is a standard user32 query.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    // SAFETY: hwnd came from the OS; the out-pointer is a local.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(std::ptr::from_mut(&mut pid)));
    }
    (pid != 0).then_some(pid)
}

fn message_loop() {
    // SAFETY: Hidden top-level window + out-of-context hook. Callbacks only try_send.
    unsafe {
        let _hwnd = create_sink_window();
        let _hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_hook),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        let mut msg = MSG::default();
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
}

unsafe fn create_sink_window() -> Option<HWND> {
    let class_name = w!("CallbackFocusSink");
    let hinstance = unsafe { GetModuleHandleW(PCWSTR::null()) }.ok()?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(sink_wnd_proc),
        hInstance: hinstance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&raw const class) };
    if atom == 0 {
        return None;
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name,
            w!("CallbackFocusSink"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    }
    .ok()?;
    let _ = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) };
    Some(hwnd)
}

unsafe extern "system" fn sink_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_WTSSESSION_CHANGE => {
            send_event(session_event(wparam));
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            send_event(power_event(wparam));
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn session_event(wparam: WPARAM) -> Option<FocusEvent> {
    match wparam.0 {
        WTS_SESSION_LOCK => Some(FocusEvent::SessionLock),
        WTS_SESSION_UNLOCK => Some(FocusEvent::SessionUnlock),
        _ => None,
    }
}

fn power_event(wparam: WPARAM) -> Option<FocusEvent> {
    match wparam.0 {
        PBT_APMSUSPEND => Some(FocusEvent::Sleep),
        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => Some(FocusEvent::Resume),
        _ => None,
    }
}

fn send_event(event: Option<FocusEvent>) {
    let Some(event) = event else {
        return;
    };
    if let Some(tx) = EVENT_TX.get() {
        let _ = tx.try_send(event);
    }
}

unsafe extern "system" fn foreground_hook(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let mut pid = 0u32;
    // SAFETY: hwnd is supplied by the OS for this event.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(std::ptr::from_mut(&mut pid)));
    }
    if let Some(tx) = EVENT_TX.get() {
        let _ = tx.try_send(FocusEvent::ForegroundPid(pid));
    }
}

fn image_from_handle(handle: HANDLE) -> Result<Option<String>, FocusError> {
    let mut buffer = [0u16; 512];
    let mut size = u32::try_from(buffer.len()).map_err(|_| FocusError::LookupFailed)?;
    // SAFETY: `buffer` is a writable WCHAR array; `size` is the capacity in
    // characters as required by `QueryFullProcessImageNameW`.
    let query = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &raw mut size,
        )
    };
    if query.is_err() || size == 0 {
        return Ok(None);
    }
    let len = usize::try_from(size).map_err(|_| FocusError::LookupFailed)?;
    Ok(Some(String::from_utf16_lossy(&buffer[..len])))
}
