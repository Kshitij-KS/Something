//! Durable commit of native-messaging envelopes into SQLite.

use crate::db::Database;
use crate::domain::SourceApp;
use crate::review::ingest_message;
use callback_protocol::{Envelope, MessageKind, PROTOCOL_VERSION};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CapturePayload {
    #[serde(alias = "captureId")]
    capture_id: Option<String>,
    #[serde(alias = "sourceApp")]
    source_app: Option<String>,
    #[serde(alias = "sourceCtx")]
    source_ctx: Option<String>,
    recipient: Option<String>,
    #[serde(alias = "rawMessage")]
    raw_message: Option<String>,
    #[serde(alias = "sentAt")]
    sent_at: Option<i64>,
}

/// Persists a capture envelope, then returns an acknowledgement.
///
/// Handshake, context, probe, and reconnect envelopes are acknowledged without
/// storing message bodies. The extension removes an outbox item only when
/// `committed` is true.
///
/// # Errors
///
/// Returns a string when SQLite or extraction persistence fails.
pub fn commit_envelope(db: &Database, envelope: Envelope) -> Result<Envelope, String> {
    match envelope.kind {
        MessageKind::Capture => persist_capture(db, &envelope)?,
        MessageKind::Handshake
        | MessageKind::Context
        | MessageKind::Probe
        | MessageKind::Reconnect
        | MessageKind::Ack
        | MessageKind::Error => {}
    }
    Ok(Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Ack,
        id: envelope.id,
        payload: serde_json::json!({ "committed": true }),
    })
}

fn persist_capture(db: &Database, envelope: &Envelope) -> Result<(), String> {
    let payload: CapturePayload =
        serde_json::from_value(envelope.payload.clone()).unwrap_or(CapturePayload {
            capture_id: None,
            source_app: None,
            source_ctx: None,
            recipient: None,
            raw_message: None,
            sent_at: None,
        });
    let capture_id = payload
        .capture_id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| envelope.id.clone());
    let source_app = match payload.source_app.as_deref() {
        Some(value) => SourceApp::parse(value)?,
        None => SourceApp::Manual,
    };
    let raw_message = payload.raw_message.unwrap_or_default();
    let sent_at = unix_seconds(
        payload
            .sent_at
            .unwrap_or_else(|| chrono::Utc::now().timestamp()),
    );
    ingest_message(
        db,
        &capture_id,
        source_app,
        payload.source_ctx.as_deref(),
        payload.recipient.as_deref(),
        &raw_message,
        sent_at,
        chrono::Utc::now(),
        0,
        "UTC",
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn unix_seconds(value: i64) -> i64 {
    if value > 10_000_000_000 {
        value / 1000
    } else {
        value
    }
}
