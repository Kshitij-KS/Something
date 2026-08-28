use callback_lib::ipc::named_pipe::serve_connection;
use callback_protocol::{
    CHROME_TO_HOST_MAX, Envelope, HOST_TO_CHROME_MAX, MessageKind, PROTOCOL_VERSION,
    encode_envelope, write_frame,
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
fn main_app_restart_queue_keeps_host_origin_check() {
    assert!(callback_lib::native_host::install::ALLOWED_ORIGIN.contains("chrome-extension://"));
}
