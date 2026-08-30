use callback_lib::extraction::{ExtractRequest, ExtractRoute, extract};
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};

const MINIMUM_CORPUS_SIZE: u32 = 300;
const PHASE_TWO_MINIMUM_PRECISION: f64 = 0.70;
const RELEASE_TARGET_PRECISION: f64 = 0.80;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusRecord {
    id: String,
    source: String,
    text: String,
    label: String,
}

#[derive(Default)]
struct InvalidCounts {
    rows: u32,
    malformed: u32,
    id: u32,
    duplicate_id: u32,
    source: u32,
    text: u32,
    label: u32,
}

#[derive(Default)]
#[allow(clippy::struct_field_names)] // Matrix coordinates are clearer with explicit route names.
struct RouteMatrix {
    capture_promise: u32,
    capture_not_promise: u32,
    review_promise: u32,
    review_not_promise: u32,
    discard_promise: u32,
    discard_not_promise: u32,
}

impl RouteMatrix {
    fn record(&mut self, route: ExtractRoute, is_promise: bool) {
        let count = match (route, is_promise) {
            (ExtractRoute::Capture, true) => &mut self.capture_promise,
            (ExtractRoute::Capture, false) => &mut self.capture_not_promise,
            (ExtractRoute::Review, true) => &mut self.review_promise,
            (ExtractRoute::Review, false) => &mut self.review_not_promise,
            (ExtractRoute::Discard, true) => &mut self.discard_promise,
            (ExtractRoute::Discard, false) => &mut self.discard_not_promise,
        };
        *count = count.saturating_add(1);
    }

    fn valid_count(&self) -> u32 {
        self.capture_promise
            .saturating_add(self.capture_not_promise)
            .saturating_add(self.review_promise)
            .saturating_add(self.review_not_promise)
            .saturating_add(self.discard_promise)
            .saturating_add(self.discard_not_promise)
    }

    fn automatic_precision(&self) -> Option<f64> {
        ratio(
            self.capture_promise,
            self.capture_promise
                .saturating_add(self.capture_not_promise),
        )
    }

    fn automatic_recall(&self) -> Option<f64> {
        ratio(
            self.capture_promise,
            self.capture_promise
                .saturating_add(self.review_promise)
                .saturating_add(self.discard_promise),
        )
    }

    fn candidate_precision(&self) -> Option<f64> {
        ratio(
            self.capture_promise.saturating_add(self.review_promise),
            self.capture_promise
                .saturating_add(self.capture_not_promise)
                .saturating_add(self.review_promise)
                .saturating_add(self.review_not_promise),
        )
    }
}

fn ratio(numerator: u32, denominator: u32) -> Option<f64> {
    (denominator != 0).then(|| f64::from(numerator) / f64::from(denominator))
}

fn message_route(text: &str) -> ExtractRoute {
    let now_utc = Utc
        .with_ymd_and_hms(2026, 1, 15, 12, 0, 0)
        .single()
        .expect("fixed evaluator timestamp");
    let clauses = extract(ExtractRequest {
        raw_message: text,
        now_utc,
        offset_seconds: 0,
        tz_label: "UTC",
        blocklist: &[],
    });
    if clauses
        .iter()
        .any(|clause| clause.route == ExtractRoute::Capture)
    {
        ExtractRoute::Capture
    } else if clauses
        .iter()
        .any(|clause| clause.route == ExtractRoute::Review)
    {
        ExtractRoute::Review
    } else {
        ExtractRoute::Discard
    }
}

#[test]
#[ignore = "requires the local CALLBACK_PRIVATE_CORPUS JSONL file"]
fn evaluates_private_message_corpus_without_emitting_content() {
    let path = std::env::var_os("CALLBACK_PRIVATE_CORPUS")
        .unwrap_or_else(|| panic!("CALLBACK_PRIVATE_CORPUS is required"));
    let file = File::open(path).unwrap_or_else(|_| panic!("failed to open private corpus"));
    let reader = BufReader::new(file);
    let mut ids = HashSet::new();
    let mut invalid = InvalidCounts::default();
    let mut matrix = RouteMatrix::default();

    for line in reader.lines() {
        let line = line.unwrap_or_else(|_| panic!("failed to read private corpus"));
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<CorpusRecord>(&line) else {
            invalid.rows = invalid.rows.saturating_add(1);
            invalid.malformed = invalid.malformed.saturating_add(1);
            continue;
        };

        let valid_id = !record.id.is_empty() && record.id.trim() == record.id;
        let valid_source = matches!(record.source.as_str(), "gmail" | "slack");
        let valid_text = !record.text.trim().is_empty();
        let valid_label = matches!(record.label.as_str(), "promise" | "not_promise");
        if !(valid_id && valid_source && valid_text && valid_label) {
            invalid.rows = invalid.rows.saturating_add(1);
            if !valid_id {
                invalid.id = invalid.id.saturating_add(1);
            }
            if !valid_source {
                invalid.source = invalid.source.saturating_add(1);
            }
            if !valid_text {
                invalid.text = invalid.text.saturating_add(1);
            }
            if !valid_label {
                invalid.label = invalid.label.saturating_add(1);
            }
            continue;
        }
        if !ids.insert(record.id) {
            invalid.rows = invalid.rows.saturating_add(1);
            invalid.duplicate_id = invalid.duplicate_id.saturating_add(1);
            continue;
        }

        matrix.record(message_route(&record.text), record.label == "promise");
    }

    let valid = matrix.valid_count();
    let automatic_precision = matrix.automatic_precision();
    let automatic_recall = matrix.automatic_recall();
    let candidate_precision = matrix.candidate_precision();
    println!(
        "private-corpus aggregate: valid={valid} invalid={} malformed={} invalid_id={} duplicate_id={} invalid_source={} invalid_text={} invalid_label={}",
        invalid.rows,
        invalid.malformed,
        invalid.id,
        invalid.duplicate_id,
        invalid.source,
        invalid.text,
        invalid.label
    );
    println!(
        "route matrix: capture[promise={},not_promise={}] review[promise={},not_promise={}] discard[promise={},not_promise={}]",
        matrix.capture_promise,
        matrix.capture_not_promise,
        matrix.review_promise,
        matrix.review_not_promise,
        matrix.discard_promise,
        matrix.discard_not_promise
    );
    println!(
        "metrics: automatic_precision={} automatic_recall={} candidate_precision={} phase2_threshold={} release_target={}",
        format_metric(automatic_precision),
        format_metric(automatic_recall),
        format_metric(candidate_precision),
        threshold_result(valid, automatic_precision, PHASE_TWO_MINIMUM_PRECISION),
        threshold_result(valid, automatic_precision, RELEASE_TARGET_PRECISION)
    );

    assert_eq!(invalid.rows, 0, "private corpus contains invalid rows");
    assert!(
        valid >= MINIMUM_CORPUS_SIZE,
        "private corpus has fewer than 300 valid unique records"
    );
    let precision = automatic_precision
        .unwrap_or_else(|| panic!("automatic precision is undefined: no Capture predictions"));
    assert!(
        precision >= PHASE_TWO_MINIMUM_PRECISION,
        "automatic-capture precision is below the 70% Phase 2 gate"
    );
}

fn format_metric(metric: Option<f64>) -> String {
    metric.map_or_else(
        || "undefined".to_owned(),
        |value| format!("{:.2}%", value * 100.0),
    )
}

fn threshold_result(valid: u32, precision: Option<f64>, threshold: f64) -> &'static str {
    if valid >= MINIMUM_CORPUS_SIZE && precision.is_some_and(|value| value >= threshold) {
        "met"
    } else {
        "not_met"
    }
}
