use callback_protocol::{
    CHROME_TO_HOST_MAX, Envelope, HOST_TO_CHROME_MAX, MessageKind, PROTOCOL_VERSION,
    decode_envelope, encode_envelope, log_event, origin_allowed, read_frame, write_frame,
};
use std::io::{Read, Write};
use std::time::Duration;

/// Pinned development extension origin derived from the manifest `key`.
pub const ALLOWED_ORIGIN: &str = "chrome-extension://difdpnmogohnpilhjlihgficnebdjphg";

/// Forwards one Chrome frame to the core and writes the acknowledgement.
///
/// # Errors
///
/// Returns protocol errors for origin, version, framing, or IO failures.
pub fn handle_message<R: Read, W: Write, C: Read + Write>(
    origin: &str,
    chrome_in: &mut R,
    chrome_out: &mut W,
    core: &mut C,
) -> Result<(), callback_protocol::ProtocolError> {
    if !origin_allowed(origin, ALLOWED_ORIGIN) {
        return Err(callback_protocol::ProtocolError::UnauthorizedOrigin(
            origin.to_owned(),
        ));
    }
    let bytes = read_frame(chrome_in, CHROME_TO_HOST_MAX)?;
    let envelope = decode_envelope(&bytes)?;
    log_event(
        &format!("{:?}", envelope.kind).to_ascii_lowercase(),
        &envelope.id,
        "in",
    );
    write_frame(core, &bytes, CHROME_TO_HOST_MAX)?;
    let ack_bytes = read_frame(core, HOST_TO_CHROME_MAX)?;
    let ack = decode_envelope(&ack_bytes)?;
    if ack.kind != MessageKind::Ack && ack.kind != MessageKind::Error {
        return Err(callback_protocol::ProtocolError::Malformed);
    }
    write_frame(chrome_out, &ack_bytes, HOST_TO_CHROME_MAX)?;
    Ok(())
}

/// Handshake helper used by diagnostics.
#[must_use]
pub fn handshake_ok(envelope: &Envelope) -> bool {
    envelope.protocol_version == PROTOCOL_VERSION && envelope.kind == MessageKind::Handshake
}

/// Retries a connect function with a fixed delay between attempts.
///
/// # Errors
///
/// Returns the last error when every attempt fails.
pub fn connect_with_backoff<F, T, E>(attempts: u32, delay: Duration, mut connect: F) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
{
    let attempts = attempts.max(1);
    let mut last = None;
    for index in 0..attempts {
        match connect() {
            Ok(value) => return Ok(value),
            Err(error) => {
                last = Some(error);
                if index + 1 < attempts {
                    std::thread::sleep(delay);
                }
            }
        }
    }
    Err(last.expect("at least one connect attempt"))
}

/// Encodes a version-mismatch error envelope.
///
/// # Errors
///
/// Returns a protocol error when encoding fails.
pub fn version_error(id: &str, found: u32) -> Result<Vec<u8>, callback_protocol::ProtocolError> {
    encode_envelope(&Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Error,
        id: id.to_owned(),
        payload: serde_json::json!({ "error": "version_mismatch", "found": found }),
    })
}
