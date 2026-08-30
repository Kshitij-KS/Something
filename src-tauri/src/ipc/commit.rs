//! Durable commit of native-messaging envelopes into SQLite.

use crate::db::Database;
use crate::domain::SourceApp;
use crate::review::ingest_message;
use callback_protocol::{Envelope, MessageKind, PROTOCOL_VERSION};
use serde::Deserialize;

const MAX_CAPTURE_BODY_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct CapturePayload {
    #[serde(alias = "captureId")]
    capture_id: String,
    #[serde(alias = "sourceApp")]
    source_app: String,
    #[serde(alias = "sourceCtx")]
    source_ctx: Option<String>,
    recipient: Option<String>,
    #[serde(alias = "rawMessage")]
    raw_message: String,
    #[serde(alias = "sentAt")]
    sent_at: i64,
    #[allow(dead_code)]
    bytes: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureDisposition {
    Committed,
    SiteDisabled,
}

/// Persists a capture envelope, then returns an acknowledgement.
///
/// Handshake, context, probe, and reconnect envelopes are acknowledged without
/// storing message bodies. Disabled-site captures receive a terminal discard
/// acknowledgement so the extension does not retain private content forever.
///
/// # Errors
///
/// Returns a string when payload validation, SQLite, or extraction persistence fails.
pub fn commit_envelope(db: &Database, envelope: Envelope) -> Result<Envelope, String> {
    let disposition = match envelope.kind {
        MessageKind::Capture => persist_capture(db, &envelope)?,
        MessageKind::Handshake
        | MessageKind::Context
        | MessageKind::Probe
        | MessageKind::Reconnect
        | MessageKind::Ack
        | MessageKind::Error => CaptureDisposition::Committed,
    };
    let (gmail, slack) = db.site_policy().map_err(|error| error.to_string())?;
    let payload = match disposition {
        CaptureDisposition::Committed => serde_json::json!({
            "committed": true,
            "site_policy": { "gmail": gmail, "slack": slack }
        }),
        CaptureDisposition::SiteDisabled => serde_json::json!({
            "committed": false,
            "discard": true,
            "reason": "site_disabled",
            "site_policy": { "gmail": gmail, "slack": slack }
        }),
    };
    Ok(Envelope {
        protocol_version: PROTOCOL_VERSION,
        kind: MessageKind::Ack,
        id: envelope.id,
        payload,
    })
}

fn persist_capture(db: &Database, envelope: &Envelope) -> Result<CaptureDisposition, String> {
    let payload: CapturePayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|_| "invalid capture payload".to_string())?;
    if payload.capture_id.trim().is_empty() || envelope.id != payload.capture_id {
        return Err("capture id is missing or does not match the envelope".into());
    }
    if payload.raw_message.trim().is_empty() {
        return Err("capture message is empty".into());
    }
    if payload.raw_message.len() > MAX_CAPTURE_BODY_BYTES {
        return Err("capture message exceeds the local outbox limit".into());
    }
    if payload.sent_at <= 0 {
        return Err("capture timestamp is invalid".into());
    }
    let source_app = match payload.source_app.as_str() {
        "gmail" => SourceApp::Gmail,
        "slack" => SourceApp::Slack,
        _ => return Err("capture source must be gmail or slack".into()),
    };
    if !db
        .site_enabled(source_app.as_str())
        .map_err(|error| error.to_string())?
    {
        return Ok(CaptureDisposition::SiteDisabled);
    }

    let sent_at = unix_seconds(payload.sent_at);
    chrono::DateTime::<chrono::Utc>::from_timestamp(sent_at, 0)
        .ok_or_else(|| "capture timestamp is outside the supported range".to_string())?;
    let timezone = db
        .get_setting("timezone")
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "UTC".into());
    timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| "configured timezone is invalid".to_string())?;

    ingest_message(
        db,
        &payload.capture_id,
        source_app,
        payload.source_ctx.as_deref(),
        payload.recipient.as_deref(),
        &payload.raw_message,
        sent_at,
        chrono::Utc::now(),
        0,
        &timezone,
    )
    .map_err(|error| error.to_string())?;
    Ok(CaptureDisposition::Committed)
}

fn unix_seconds(value: i64) -> i64 {
    if value > 10_000_000_000 {
        value / 1000
    } else {
        value
    }
}
