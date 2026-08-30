use callback_lib::ipc::named_pipe::serve_connection;
use callback_protocol::{
    CHROME_TO_HOST_MAX, Envelope, HOST_TO_CHROME_MAX, MessageKind, PROTOCOL_VERSION,
    decode_envelope, encode_envelope, read_frame, write_frame,
};
use std::io::Cursor;

#[test]
fn acknowledgement_is_written_after_commit_callback() {
    let envelope = Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Capture,
        id: "cap-1".into(),
        payload: serde_json::json!({ "capture_id": "cap-1" }),
    };
    let bytes = encode_envelope(&envelope).expect("encode");
    let mut incoming = Vec::new();
    write_frame(&mut incoming, &bytes, CHROME_TO_HOST_MAX).expect("frame");
    let mut outgoing = Vec::new();
    let mut committed = false;
    serve_connection(&mut Cursor::new(incoming), &mut outgoing, |received| {
        committed = received.id == "cap-1";
        Ok(Envelope {
            protocol_version: PROTOCOL_VERSION,
            kind: MessageKind::Ack,
            id: received.id,
            payload: serde_json::json!({ "committed": true }),
        })
    })
    .expect("serve");
    assert!(committed);
    assert!(outgoing.len() > 4);
    assert!(outgoing.len() <= HOST_TO_CHROME_MAX + 4);
}

#[test]
fn one_pipe_connection_serves_multiple_envelopes() {
    let mut incoming = Vec::new();
    for id in ["hs-1", "cap-2"] {
        let bytes = encode_envelope(&Envelope {
            protocol_version: PROTOCOL_VERSION,
            kind: MessageKind::Handshake,
            id: id.into(),
            payload: serde_json::json!({}),
        })
        .expect("encode");
        write_frame(&mut incoming, &bytes, CHROME_TO_HOST_MAX).expect("frame");
    }

    let mut outgoing = Vec::new();
    let mut committed = Vec::new();
    serve_connection(&mut Cursor::new(incoming), &mut outgoing, |received| {
        committed.push(received.id.clone());
        Ok(Envelope {
            protocol_version: PROTOCOL_VERSION,
            kind: MessageKind::Ack,
            id: received.id,
            payload: serde_json::json!({ "committed": true }),
        })
    })
    .expect("serve");

    assert_eq!(committed, ["hs-1", "cap-2"]);
    let mut replies = Cursor::new(outgoing);
    for expected in ["hs-1", "cap-2"] {
        let bytes = read_frame(&mut replies, HOST_TO_CHROME_MAX).expect("reply frame");
        let ack = decode_envelope(&bytes).expect("reply envelope");
        assert_eq!(ack.kind, MessageKind::Ack);
        assert_eq!(ack.id, expected);
    }
}

#[test]
fn main_app_restart_queue_keeps_host_origin_check() {
    assert!(callback_lib::native_host::install::ALLOWED_ORIGIN.contains("chrome-extension://"));
}
