#![allow(unsafe_code)]

use callback_native_host::{ALLOWED_ORIGIN, connect_with_backoff, handle_message};
use std::io::{self, BufReader, BufWriter};
use std::time::Duration;

fn main() {
    if let Err(error) = run() {
        callback_protocol::log_event("fatal", "-", &error.to_string());
        std::process::exit(1);
    }
}

fn run() -> Result<(), callback_protocol::ProtocolError> {
    set_binary_stdio().map_err(|_| callback_protocol::ProtocolError::Io)?;
    let origin = std::env::args().nth(1).unwrap_or_default();
    let origin = if origin.is_empty() {
        ALLOWED_ORIGIN
    } else {
        origin.as_str()
    };
    let mut stdin = BufReader::new(io::stdin());
    let mut stdout = BufWriter::new(io::stdout());
    let mut core = connect_with_backoff(20, Duration::from_millis(250), connect_core)?;
    loop {
        match handle_message(origin, &mut stdin, &mut stdout, &mut core) {
            Ok(()) => {}
            Err(callback_protocol::ProtocolError::Malformed) => break,
            Err(callback_protocol::ProtocolError::Io) => {
                core = connect_with_backoff(20, Duration::from_millis(250), connect_core)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn set_binary_stdio() -> io::Result<()> {
    #[cfg(windows)]
    {
        const O_BINARY: i32 = 0x8000;
        unsafe extern "C" {
            fn _setmode(fd: i32, mode: i32) -> i32;
        }
        // SAFETY: Chrome requires CRT stdio in binary mode before the first frame.
        unsafe {
            if _setmode(0, O_BINARY) == -1 || _setmode(1, O_BINARY) == -1 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

fn connect_core() -> Result<impl io::Read + io::Write, callback_protocol::ProtocolError> {
    #[cfg(windows)]
    {
        windows_pipe::connect().ok_or(callback_protocol::ProtocolError::Io)
    }
    #[cfg(not(windows))]
    {
        Err(callback_protocol::ProtocolError::Io)
    }
}

#[cfg(windows)]
mod windows_pipe {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    pub fn connect() -> Option<std::fs::File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(r"\\.\pipe\callback-com.callback.desktop")
            .ok()
    }
}
