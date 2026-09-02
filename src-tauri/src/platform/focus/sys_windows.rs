//! Windows process identity lookup. Callbacks stay off this path.

#![allow(unsafe_code)]

use super::{FocusError, FocusEvent};
use std::sync::OnceLock;
use std::sync::mpsc::SyncSender;
use std::thread;
use windows::Win32::Foundation::{
    CloseHandle, CompareObjectHandles, FILETIME, HANDLE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};
use windows::Win32::System::Threading::{
    GetProcessId, GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, IsWindowVisible, MSG,
    RegisterClassW, TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_DESTROY,
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

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open(pid: u32) -> Option<Self> {
        if pid == 0 {
            return None;
        }
        // SAFETY: `pid` is OS-supplied. Limited query access is sufficient,
        // and access denial or an exited process is treated as unavailable.
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .ok()
            .map(Self)
    }

    fn as_raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: This wrapper exclusively owns a successful `OpenProcess`
        // result and closes it exactly once when leaving scope.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct ProcessIdentity {
    pid: u32,
    creation_time: u64,
}

fn process_identity(handle: HANDLE) -> Option<ProcessIdentity> {
    // SAFETY: `handle` is a live process handle owned by `ProcessHandle`.
    let pid = unsafe { GetProcessId(handle) };
    if pid == 0 {
        return None;
    }

    let mut creation_time = FILETIME::default();
    let mut exit_time = FILETIME::default();
    let mut kernel_time = FILETIME::default();
    let mut user_time = FILETIME::default();
    // SAFETY: All four output pointers reference distinct writable locals,
    // and the process handle remains owned for the duration of this query.
    unsafe {
        GetProcessTimes(
            handle,
            &raw mut creation_time,
            &raw mut exit_time,
            &raw mut kernel_time,
            &raw mut user_time,
        )
    }
    .ok()?;

    let creation_time =
        (u64::from(creation_time.dwHighDateTime) << 32) | u64::from(creation_time.dwLowDateTime);
    Some(ProcessIdentity { pid, creation_time })
}

fn same_process_object(first: HANDLE, second: HANDLE) -> bool {
    // SAFETY: Both handles are live process handles owned by wrappers in the
    // caller. The API only compares their underlying kernel objects.
    unsafe { CompareObjectHandles(first, second) }.as_bool()
}

/// Resolves a PID without panicking when the process is protected.
///
/// # Errors
///
/// Returns [`FocusError::LookupFailed`] only for unexpected conversion errors.
pub fn resolve_process_image(pid: u32) -> Result<Option<String>, FocusError> {
    let Some(handle) = ProcessHandle::open(pid) else {
        return Ok(None);
    };
    image_from_handle(handle.as_raw())
}

struct VisibleWindowProcess {
    hwnd: HWND,
    pid: u32,
}

fn window_pid(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: `hwnd` is treated as opaque OS data and the out-pointer is a
    // writable local. A zero thread id reports an invalid/destroyed window.
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    (thread_id != 0 && pid != 0).then_some(pid)
}

fn visible_window_pid(hwnd: HWND) -> Option<u32> {
    let before = window_pid(hwnd)?;
    // SAFETY: Invalid or destroyed window handles report not visible.
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let after = window_pid(hwnd)?;
    (visible && before == after).then_some(after)
}

fn resolve_visible_candidate(
    candidate: &VisibleWindowProcess,
) -> Result<Option<(ProcessIdentity, String)>, FocusError> {
    if visible_window_pid(candidate.hwnd) != Some(candidate.pid) {
        return Ok(None);
    }

    let Some(original_handle) = ProcessHandle::open(candidate.pid) else {
        return Ok(None);
    };
    let Some(original_identity) = process_identity(original_handle.as_raw()) else {
        return Ok(None);
    };
    if original_identity.pid != candidate.pid {
        return Ok(None);
    }
    let Some(path) = image_from_handle(original_handle.as_raw())? else {
        return Ok(None);
    };

    let Some(current_pid) = visible_window_pid(candidate.hwnd) else {
        return Ok(None);
    };
    let Some(current_handle) = ProcessHandle::open(current_pid) else {
        return Ok(None);
    };
    let Some(current_identity) = process_identity(current_handle.as_raw()) else {
        return Ok(None);
    };
    if current_identity != original_identity
        || !same_process_object(original_handle.as_raw(), current_handle.as_raw())
    {
        return Ok(None);
    }

    Ok(Some((original_identity, path)))
}

/// Lists visible top-level app executable basenames.
///
/// Window enumeration collects only stable window/PID samples. Image lookup
/// happens afterward while the original process handle remains open. A fresh
/// handle must resolve to the same creation identity and kernel process object
/// after window revalidation before the executable is accepted. Full paths are
/// reduced before leaving this module.
///
/// # Errors
///
/// Returns [`FocusError::LookupFailed`] when Windows cannot enumerate windows.
pub fn list_focus_apps() -> Result<Vec<String>, FocusError> {
    let mut candidates = Vec::<VisibleWindowProcess>::new();
    let context = LPARAM(std::ptr::from_mut(&mut candidates) as isize);
    // SAFETY: `context` points to `candidates` for the duration of this
    // synchronous enumeration. The callback only appends OS-supplied pairs.
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::EnumWindows(
            Some(collect_visible_window_process),
            context,
        )
    }
    .map_err(|_| FocusError::LookupFailed)?;

    let mut accepted_processes = std::collections::BTreeSet::<ProcessIdentity>::new();
    let mut apps = std::collections::BTreeMap::<String, String>::new();
    for candidate in candidates {
        let Ok(Some((identity, path))) = resolve_visible_candidate(&candidate) else {
            continue;
        };
        let Some(executable) = focus_app_basename(&path) else {
            continue;
        };
        if !is_user_focus_app(executable) || !accepted_processes.insert(identity) {
            continue;
        }
        apps.entry(executable.to_ascii_lowercase())
            .or_insert_with(|| executable.to_owned());
    }
    Ok(apps.into_values().collect())
}

unsafe extern "system" fn collect_visible_window_process(
    hwnd: HWND,
    context: LPARAM,
) -> windows::core::BOOL {
    let keep_enumerating = windows::core::BOOL(1);
    if context.0 == 0 {
        return keep_enumerating;
    }
    let Some(pid) = visible_window_pid(hwnd) else {
        return keep_enumerating;
    };

    // SAFETY: `context` was created from a live
    // `Vec<VisibleWindowProcess>` immediately before the synchronous
    // `EnumWindows` call.
    let candidates = unsafe { &mut *(context.0 as *mut Vec<VisibleWindowProcess>) };
    candidates.push(VisibleWindowProcess { hwnd, pid });
    keep_enumerating
}

fn focus_app_basename(path: &str) -> Option<&str> {
    path.rsplit(['\\', '/'])
        .next()
        .filter(|name| !name.is_empty())
}

fn is_user_focus_app(executable: &str) -> bool {
    const HIDDEN_HOSTS: &[&str] = &[
        "applicationframehost.exe",
        "callback-app.exe",
        "callback-native-host.exe",
        "dwm.exe",
        "lockapp.exe",
        "runtimebroker.exe",
        "searchhost.exe",
        "shellexperiencehost.exe",
        "startmenuexperiencehost.exe",
        "textinputhost.exe",
    ];
    !HIDDEN_HOSTS
        .iter()
        .any(|hidden| executable.eq_ignore_ascii_case(hidden))
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
