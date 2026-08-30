//! Current-user named pipe used between the native host and the Tauri core.

use callback_protocol::{
    Envelope, HOST_TO_CHROME_MAX, MessageKind, PROTOCOL_VERSION, decode_envelope, encode_envelope,
    read_frame_or_eof, write_frame,
};
use std::io::{Read, Write};
use std::sync::Arc;

/// Well-known per-user pipe name. ACL is current-user only on Windows.
pub const PIPE_PATH: &str = r"\\.\pipe\callback-com.callback.desktop";

/// IPC failures.
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("named pipe unavailable on this platform")]
    Unsupported,
    #[error("pipe io failed: {0}")]
    Io(String),
    #[error(transparent)]
    Protocol(#[from] callback_protocol::ProtocolError),
}

/// Handles one host connection until the client disconnects cleanly.
///
/// # Errors
///
/// Returns [`IpcError`] when framing or the commit callback fails.
pub fn serve_connection<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    mut commit: F,
) -> Result<(), IpcError>
where
    R: Read,
    W: Write,
    F: FnMut(Envelope) -> Result<Envelope, String>,
{
    while let Some(bytes) = read_frame_or_eof(reader, callback_protocol::CHROME_TO_HOST_MAX)? {
        let envelope = decode_envelope(&bytes)?;
        let envelope_id = envelope.id.clone();
        let ack = commit(envelope).unwrap_or_else(|error| Envelope {
            protocol_version: PROTOCOL_VERSION,
            kind: MessageKind::Error,
            id: envelope_id,
            payload: serde_json::json!({ "error": error }),
        });
        let encoded = encode_envelope(&ack)?;
        write_frame(writer, &encoded, HOST_TO_CHROME_MAX)?;
    }
    Ok(())
}

/// Forwards accepted envelopes through a durable commit callback.
///
/// # Errors
///
/// Returns [`IpcError`] if the server thread cannot start.
pub fn spawn_pipe_server(commit: CommitFn) -> Result<(), IpcError> {
    #[cfg(all(windows, feature = "windows-platform"))]
    {
        windows_impl::spawn(commit)
    }
    #[cfg(not(all(windows, feature = "windows-platform")))]
    {
        let _ = commit;
        Ok(())
    }
}

/// Commit callback used by the named-pipe server.
pub type CommitFn = Arc<dyn Fn(Envelope) -> Result<Envelope, String> + Send + Sync>;

#[cfg(all(windows, feature = "windows-platform"))]
mod windows_impl {
    #![allow(unsafe_code)]

    use super::{CommitFn, IpcError, PIPE_PATH, serve_connection};
    use std::fs::File;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::sync::Arc;
    use std::thread;
    use windows::Win32::Foundation::{HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree};
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows::core::PCWSTR;

    const ERROR_PIPE_CONNECTED_HRESULT: i32 = -2_147_024_361;

    pub fn spawn(commit: CommitFn) -> Result<(), IpcError> {
        thread::Builder::new()
            .name("callback-pipe".into())
            .spawn(move || {
                loop {
                    match accept_one(&commit) {
                        Ok(()) => {}
                        Err(error) => {
                            tracing::warn!(error = %error, "pipe accept failed");
                            thread::sleep(std::time::Duration::from_millis(500));
                        }
                    }
                }
            })
            .map_err(|error| IpcError::Io(error.to_string()))?;
        Ok(())
    }

    fn accept_one(commit: &CommitFn) -> Result<(), IpcError> {
        let handle = create_pipe()?;
        // SAFETY: `handle` is a dedicated pipe instance for this accept.
        if let Err(error) = unsafe { ConnectNamedPipe(handle, None) } {
            if error.code().0 != ERROR_PIPE_CONNECTED_HRESULT {
                drop_handle(handle);
                return Err(IpcError::Io(format!("ConnectNamedPipe failed: {error}")));
            }
        }
        // SAFETY: `handle` is an exclusive pipe HANDLE; File takes ownership.
        let file = unsafe { File::from_raw_handle(handle.0 as RawHandle) };
        let callback = Arc::clone(commit);
        thread::Builder::new()
            .name("callback-pipe-client".into())
            .spawn(move || {
                let mut reader = file;
                let writer = reader.try_clone();
                match writer {
                    Ok(mut writer) => {
                        if let Err(error) = serve_connection(&mut reader, &mut writer, |envelope| {
                            callback(envelope)
                        }) {
                            tracing::debug!(error = %error, "pipe client disconnected");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "pipe handle clone failed");
                    }
                }
            })
            .map_err(|error| IpcError::Io(error.to_string()))?;
        Ok(())
    }

    fn create_pipe() -> Result<HANDLE, IpcError> {
        let name: Vec<u16> = std::ffi::OsStr::new(PIPE_PATH)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let descriptor = owner_only_descriptor()?;
        let mut security = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                .map_err(|error| IpcError::Io(error.to_string()))?,
            lpSecurityDescriptor: descriptor.as_ptr(),
            bInheritHandle: false.into(),
        };
        // SAFETY: null-terminated pipe name and validated owner-only descriptor.
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0x0000_0003),
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                Some(std::ptr::from_mut(&mut security)),
            )
        };
        if handle.is_invalid() || handle == INVALID_HANDLE_VALUE {
            return Err(IpcError::Io("CreateNamedPipeW failed".into()));
        }
        Ok(handle)
    }

    struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

    impl OwnedSecurityDescriptor {
        fn as_ptr(&self) -> *mut core::ffi::c_void {
            self.0.0
        }
    }

    impl Drop for OwnedSecurityDescriptor {
        fn drop(&mut self) {
            // SAFETY: the descriptor was allocated by LocalAlloc through the SDDL converter.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0.0)));
            }
        }
    }

    fn owner_only_descriptor() -> Result<OwnedSecurityDescriptor, IpcError> {
        use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;

        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let sddl: Vec<u16> = "D:P(A;;GA;;;OW)"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: SDDL is a valid null-terminated UTF-16 owner-only descriptor.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                1,
                std::ptr::from_mut(&mut descriptor),
                None,
            )
        }
        .map_err(|error| IpcError::Io(format!("owner-only ACL creation failed: {error}")))?;
        if descriptor.0.is_null() {
            return Err(IpcError::Io(
                "owner-only ACL creation returned a null descriptor".into(),
            ));
        }
        Ok(OwnedSecurityDescriptor(descriptor))
    }

    fn drop_handle(handle: HANDLE) {
        use windows::Win32::Foundation::CloseHandle;
        // SAFETY: exclusive HANDLE from CreateNamedPipeW.
        unsafe {
            let _ = CloseHandle(handle);
        }
    }
}
