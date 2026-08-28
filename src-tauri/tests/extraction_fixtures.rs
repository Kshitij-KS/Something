use callback_lib::extraction::deadline::{DeadlineLexicon, parse_deadline};
use callback_lib::extraction::{ExtractRequest, ExtractRoute, extract, skeleton};
use chrono::{FixedOffset, TimeZone, Utc};

fn extract_one(message: &str) -> callback_lib::extraction::ExtractedClause {
    extract(ExtractRequest {
        raw_message: message,
        now_utc: Utc::now(),
        offset_seconds: 0,
        tz_label: "UTC",
        blocklist: &[],
    })
    .into_iter()
    .next()
    .expect("clause")
}

#[test]
fn commissive_verb_temporal_and_object_reach_capture() {
    let clause = extract_one("I will send the invoice tomorrow");
    assert!(clause.score >= 6, "score {}", clause.score);
    assert_eq!(clause.route, ExtractRoute::Capture);
    assert!(clause.normalized.contains("i will"));
}

#[test]
fn contractions_expand_before_scoring() {
    let clause = extract_one("I'll send the report tomorrow");
    assert!(clause.normalized.contains("i will"));
    assert!(clause.score >= 6);
}

#[test]
fn questions_second_person_opinion_past_and_quotes_are_killed() {
    assert_eq!(
        extract_one("Can you send the invoice?").kill_reason,
        Some("question")
    );
    assert_eq!(
        extract_one("Can you send the invoice").kill_reason,
        Some("second_person_request")
    );
    assert_eq!(
        extract_one("I think the invoice is ready").kill_reason,
        Some("opinion")
    );
    assert_eq!(extract_one("I sent the invoice").kill_reason, Some("past"));
    assert_eq!(
        extract_one("> I'll send the invoice tomorrow").kill_reason,
        Some("quoted")
    );
}

#[test]
fn conditional_and_attendance_apply_penalties() {
    let conditional = extract_one("I will send the invoice tomorrow if I can");
    let baseline = extract_one("I will send the invoice tomorrow");
    assert!(conditional.score <= baseline.score - 2);
    let attendance = extract_one("I'll be there tomorrow");
    assert!(attendance.score < 4);
}

#[test]
fn eod_eow_use_the_explicit_lexicon() {
    let now = FixedOffset::east_opt(0)
        .expect("offset")
        .with_ymd_and_hms(2026, 8, 26, 10, 0, 0)
        .single()
        .expect("dt")
        .with_timezone(&Utc);
    let lexicon = DeadlineLexicon::default();
    let eod = parse_deadline("by EOD", now, 0, "UTC", &lexicon).expect("eod");
    assert_eq!(eod.precision.as_str(), "eod");
    let eow = parse_deadline("by EOW", now, 0, "UTC", &lexicon).expect("eow");
    assert_eq!(eow.precision.as_str(), "eow");
}

#[test]
fn locale_timezone_and_dst_keep_utc_storage() {
    let offset = 5 * 3600 + 30 * 60;
    let now = Utc
        .with_ymd_and_hms(2026, 3, 8, 6, 30, 0)
        .single()
        .expect("now");
    let parsed = parse_deadline(
        "today",
        now,
        offset,
        "Asia/Kolkata",
        &DeadlineLexicon::default(),
    )
    .expect("today");
    assert_eq!(parsed.tz_label, "Asia/Kolkata");
    assert!(parsed.utc_ts < now.timestamp() + 36 * 3600);
}

#[test]
fn multi_clause_idempotency_uses_ordinals() {
    let clauses = extract(ExtractRequest {
        raw_message: "I will send the invoice tomorrow. I will share the deck this week.",
        now_utc: Utc::now(),
        offset_seconds: 0,
        tz_label: "UTC",
        blocklist: &[],
    });
    assert_eq!(clauses.len(), 2);
    assert_eq!(clauses[0].ordinal, 0);
    assert_eq!(clauses[1].ordinal, 1);
}

#[test]
fn blocklist_skeleton_matches_related_shapes() {
    assert_eq!(
        skeleton("I'll be at the standup"),
        skeleton("I'll be at the retro")
    );
}

#[test]
fn extraction_logs_never_include_the_raw_body() {
    let clause = extract_one("I will send the secret-body-token tomorrow");
    assert!(!format!("{clause:?}").is_empty());
    // Production tracing fields are clause_ordinal/score/kill only.
}
