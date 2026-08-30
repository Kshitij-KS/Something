//! Versioned Callback native-messaging protocol and 32-bit native-endian framing.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Current protocol version.
pub const PROTOCOL_VERSION: u32 = 1;
/// Host-to-Chrome payload limit.
pub const HOST_TO_CHROME_MAX: usize = 1024 * 1024;
/// Chrome-to-host payload limit.
pub const CHROME_TO_HOST_MAX: usize = 64 * 1024 * 1024;
/// Native messaging host name.
pub const HOST_NAME: &str = "com.callback.host";

/// Framing or protocol validation failure.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("frame exceeds {max} bytes")]
    Oversized { max: usize },
    #[error("malformed frame")]
    Malformed,
    #[error("unexpected protocol version {0}")]
    VersionMismatch(u32),
    #[error("unauthorized origin {0}")]
    UnauthorizedOrigin(String),
    #[error("io failure")]
    Io,
}

/// Envelope exchanged by the extension, native host, and core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub protocol_version: u32,
    pub kind: MessageKind,
    pub id: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Handshake,
    Capture,
    Context,
    Ack,
    Probe,
    Reconnect,
    Error,
}

/// Writes a native-endian 32-bit length prefix plus payload.
///
/// # Errors
///
/// Returns [`ProtocolError::Oversized`] or [`ProtocolError::Io`].
pub fn write_frame<W: Write>(
    writer: &mut W,
    payload: &[u8],
    max: usize,
) -> Result<(), ProtocolError> {
    if payload.len() > max {
        return Err(ProtocolError::Oversized { max });
    }
    let len = u32::try_from(payload.len()).map_err(|_| ProtocolError::Oversized { max })?;
    writer
        .write_all(&len.to_ne_bytes())
        .map_err(|_| ProtocolError::Io)?;
    writer.write_all(payload).map_err(|_| ProtocolError::Io)?;
    writer.flush().map_err(|_| ProtocolError::Io)
}

/// Reads one framed payload, looping across partial reads.
///
/// # Errors
///
/// Returns [`ProtocolError`] for malformed, oversized, truncated, or missing frames.
pub fn read_frame<R: Read>(reader: &mut R, max: usize) -> Result<Vec<u8>, ProtocolError> {
    read_frame_or_eof(reader, max)?.ok_or(ProtocolError::Malformed)
}

/// Reads one framed payload while treating EOF before a new prefix as a clean disconnect.
///
/// EOF after any prefix or payload byte remains malformed, so truncated frames are never
/// accepted as graceful connection shutdown.
///
/// # Errors
///
/// Returns [`ProtocolError`] for malformed, oversized, truncated, or failed reads.
pub fn read_frame_or_eof<R: Read>(
    reader: &mut R,
    max: usize,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    let mut len_buf = [0u8; 4];
    loop {
        match reader.read(&mut len_buf[..1]) {
            Ok(0) => return Ok(None),
            Ok(1) => break,
            Ok(_) => unreachable!("one-byte read returned more than one byte"),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(ProtocolError::Io),
        }
    }
    read_exact_partial(reader, &mut len_buf[1..])?;
    let len = usize::try_from(u32::from_ne_bytes(len_buf)).map_err(|_| ProtocolError::Malformed)?;
    if len > max {
        return Err(ProtocolError::Oversized { max });
    }
    let mut payload = vec![0u8; len];
    read_exact_partial(reader, &mut payload)?;
    Ok(Some(payload))
}

fn read_exact_partial<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<(), ProtocolError> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => return Err(ProtocolError::Malformed),
            Ok(n) => filled += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(ProtocolError::Io),
        }
    }
    Ok(())
}

/// Decodes a JSON envelope and checks the protocol version.
///
/// # Errors
///
/// Returns [`ProtocolError::Malformed`] or [`ProtocolError::VersionMismatch`].
pub fn decode_envelope(bytes: &[u8]) -> Result<Envelope, ProtocolError> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ProtocolError::Malformed)?;
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolError::VersionMismatch(envelope.protocol_version));
    }
    Ok(envelope)
}

/// Encodes an envelope to UTF-8 JSON bytes.
///
/// # Errors
///
/// Returns [`ProtocolError::Malformed`] if serialization fails.
pub fn encode_envelope(envelope: &Envelope) -> Result<Vec<u8>, ProtocolError> {
    serde_json::to_vec(envelope).map_err(|_| ProtocolError::Malformed)
}

/// Validates the Chrome-supplied origin against the pinned development ID.
#[must_use]
pub fn origin_allowed(origin: &str, allowed: &str) -> bool {
    origin
        .trim_end_matches('/')
        .eq_ignore_ascii_case(allowed.trim_end_matches('/'))
}

/// Content-free stderr log line. Never include message bodies.
pub fn log_event(kind: &str, id: &str, extra: &str) {
    eprintln!("callback-host kind={kind} id={id} {extra}");
}
