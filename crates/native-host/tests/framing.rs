use callback_native_host::{ALLOWED_ORIGIN, handle_message};
use callback_protocol::{
    CHROME_TO_HOST_MAX, Envelope, HOST_TO_CHROME_MAX, MessageKind, PROTOCOL_VERSION,
    decode_envelope, encode_envelope, read_frame, write_frame,
};
use std::io::Cursor;

#[test]
fn unicode_round_trip_preserves_json() {
    let envelope = Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Capture,
        id: "cap-ユニコード".into(),
        payload: serde_json::json!({ "note": "Priya — invoice" }),
    };
    let bytes = encode_envelope(&envelope).expect("encode");
    let mut buffer = Vec::new();
    write_frame(&mut buffer, &bytes, CHROME_TO_HOST_MAX).expect("write");
    let mut cursor = Cursor::new(buffer);
    let decoded = read_frame(&mut cursor, CHROME_TO_HOST_MAX).expect("read");
    assert_eq!(decode_envelope(&decoded).expect("json"), envelope);
}

#[test]
fn partial_reads_reassemble_a_frame() {
    let payload = br#"{"protocol_version":1,"kind":"ack","id":"a","payload":{}}"#;
    let mut framed = Vec::new();
    write_frame(&mut framed, payload, HOST_TO_CHROME_MAX).expect("frame");
    let mut reader = PartialReader {
        inner: framed,
        max_chunk: 3,
    };
    let out = read_frame(&mut reader, HOST_TO_CHROME_MAX).expect("partial");
    assert_eq!(out, payload);
}

#[test]
fn malformed_and_oversized_frames_are_rejected() {
    let mut tiny = Cursor::new([1, 0, 0, 0]);
    assert!(read_frame(&mut tiny, HOST_TO_CHROME_MAX).is_err());
    let huge = vec![0u8; 8];
    let mut writer = Vec::new();
    assert!(write_frame(&mut writer, &huge, 4).is_err());
}

#[test]
fn stdout_contains_only_framed_bytes() {
    let mut chrome_in = Vec::new();
    let payload = encode_envelope(&Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Handshake,
        id: "h1".into(),
        payload: serde_json::json!({}),
    })
    .expect("env");
    write_frame(&mut chrome_in, &payload, CHROME_TO_HOST_MAX).expect("in");
    let ack = encode_envelope(&Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Ack,
        id: "h1".into(),
        payload: serde_json::json!({ "committed": true }),
    })
    .expect("ack");
    let mut core_buf = Vec::new();
    write_frame(&mut core_buf, &ack, HOST_TO_CHROME_MAX).expect("core ack");
    let mut core = Combined {
        reader: Cursor::new(core_buf),
        writer: Vec::new(),
    };
    let mut stdout = Vec::new();
    handle_message(
        ALLOWED_ORIGIN,
        &mut Cursor::new(chrome_in),
        &mut stdout,
        &mut core,
    )
    .expect("handle");
    assert_eq!(stdout[0..4].len(), 4);
    assert!(!stdout.starts_with(b"callback"));
}

#[test]
fn unauthorized_origin_is_rejected() {
    let mut empty = Cursor::new(Vec::<u8>::new());
    let mut stdout = Vec::new();
    let mut core = Combined {
        reader: Cursor::new(Vec::new()),
        writer: Vec::new(),
    };
    let error = handle_message(
        "chrome-extension://not-allowed",
        &mut empty,
        &mut stdout,
        &mut core,
    )
    .expect_err("origin");
    assert!(matches!(
        error,
        callback_protocol::ProtocolError::UnauthorizedOrigin(_)
    ));
}

#[test]
fn connect_with_backoff_retries_then_succeeds() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    let attempts = AtomicU32::new(0);
    let result = callback_native_host::connect_with_backoff(4, Duration::from_millis(1), || {
        let n = attempts.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Err("no pipe")
        } else {
            Ok("connected")
        }
    });
    assert_eq!(result, Ok("connected"));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[test]
fn connect_with_backoff_exhausts_attempts() {
    use std::time::Duration;

    let result: Result<(), &str> =
        callback_native_host::connect_with_backoff(3, Duration::from_millis(1), || Err("down"));
    assert_eq!(result, Err("down"));
}

#[test]
fn version_mismatch_is_rejected() {
    let bytes = br#"{"protocol_version":9,"kind":"handshake","id":"x","payload":{}}"#;
    assert!(matches!(
        decode_envelope(bytes),
        Err(callback_protocol::ProtocolError::VersionMismatch(9))
    ));
}

struct PartialReader {
    inner: Vec<u8>,
    max_chunk: usize,
}

impl std::io::Read for PartialReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.inner.is_empty() {
            return Ok(0);
        }
        let n = self.max_chunk.min(buf.len()).min(self.inner.len());
        buf[..n].copy_from_slice(&self.inner[..n]);
        self.inner.drain(..n);
        Ok(n)
    }
}

struct Combined {
    reader: Cursor<Vec<u8>>,
    writer: Vec<u8>,
}

impl std::io::Read for Combined {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

impl std::io::Write for Combined {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

#[test]
fn clean_eof_is_distinct_from_a_truncated_frame() {
    let mut empty = Cursor::new(Vec::<u8>::new());
    assert_eq!(
        callback_protocol::read_frame_or_eof(&mut empty, HOST_TO_CHROME_MAX),
        Ok(None)
    );

    let mut truncated = Cursor::new(vec![1_u8, 0]);
    assert_eq!(
        callback_protocol::read_frame_or_eof(&mut truncated, HOST_TO_CHROME_MAX),
        Err(callback_protocol::ProtocolError::Malformed)
    );
}
