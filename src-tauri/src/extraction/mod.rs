pub mod deadline;

use crate::extraction::deadline::{DeadlineLexicon, ParsedDeadline, parse_deadline};
use regex::Regex;
use std::sync::OnceLock;

/// Destination for a scored clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractRoute {
    Capture,
    Review,
    Discard,
}

/// One scored clause from an outgoing message.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedClause {
    pub ordinal: usize,
    pub original: String,
    pub normalized: String,
    pub score: i32,
    pub route: ExtractRoute,
    pub deadline: Option<ParsedDeadline>,
    pub kill_reason: Option<&'static str>,
}

/// Extraction input. Message bodies must never be written to logs.
#[derive(Debug, Clone)]
pub struct ExtractRequest<'a> {
    pub raw_message: &'a str,
    pub now_utc: chrono::DateTime<chrono::Utc>,
    pub offset_seconds: i32,
    pub tz_label: &'a str,
    pub blocklist: &'a [String],
}

/// Segments, scores, and routes clauses.
#[must_use]
pub fn extract(request: ExtractRequest<'_>) -> Vec<ExtractedClause> {
    segment(request.raw_message)
        .into_iter()
        .enumerate()
        .filter_map(|(ordinal, original)| score_clause(ordinal, &original, &request))
        .collect()
}

/// Builds a blocklist skeleton from a rejected clause.
#[must_use]
pub fn skeleton(clause: &str) -> String {
    let function_words = [
        "i", "will", "am", "going", "to", "be", "at", "the", "a", "an", "in", "on", "for", "of",
        "and", "or", "with", "you", "me", "my", "your", "this", "that", "by",
    ];
    normalize(clause)
        .split_whitespace()
        .map(|word| {
            if function_words.contains(&word) {
                word.to_owned()
            } else {
                "*".into()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn score_clause(
    ordinal: usize,
    original: &str,
    request: &ExtractRequest<'_>,
) -> Option<ExtractedClause> {
    let trimmed = original.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = normalize(trimmed);
    if let Some(reason) = hard_kill(trimmed, &normalized, request.blocklist) {
        tracing::info!(clause_ordinal = ordinal, kill = reason, "clause rejected");
        return Some(ExtractedClause {
            ordinal,
            original: trimmed.to_owned(),
            normalized,
            score: 0,
            route: ExtractRoute::Discard,
            deadline: None,
            kill_reason: Some(reason),
        });
    }
    let mut score = 0;
    if regex(COMMISSIVE).is_match(&normalized) {
        score += 3;
    }
    if regex(VERB).is_match(&normalized) {
        score += 2;
    }
    if regex(TEMPORAL).is_match(&normalized) {
        score += 2;
    }
    if regex(OBJECT).is_match(&normalized) {
        score += 1;
    }
    if regex(CONDITIONAL).is_match(&normalized) {
        score -= 2;
    }
    if regex(ATTENDANCE).is_match(&normalized) {
        score -= 3;
    }
    if word_count(&normalized) > 25 {
        score -= 1;
    }
    let route = if score >= 6 {
        ExtractRoute::Capture
    } else if score >= 4 {
        ExtractRoute::Review
    } else {
        ExtractRoute::Discard
    };
    let deadline = parse_deadline(
        &normalized,
        request.now_utc,
        request.offset_seconds,
        request.tz_label,
        &DeadlineLexicon::default(),
    );
    tracing::info!(clause_ordinal = ordinal, score, "clause scored");
    Some(ExtractedClause {
        ordinal,
        original: trimmed.to_owned(),
        normalized,
        score,
        route,
        deadline,
        kill_reason: None,
    })
}

fn segment(message: &str) -> Vec<String> {
    let mut clauses = Vec::new();
    let mut current = String::new();
    for token in split_keep(message) {
        if matches!(token.as_str(), "." | ";" | "\n") || token.eq_ignore_ascii_case(" and ") {
            if !current.trim().is_empty() {
                clauses.push(current.trim().to_owned());
            }
            current.clear();
        } else if token == "," {
            if current.split_whitespace().count() > 6 {
                clauses.push(current.trim().to_owned());
                current.clear();
            } else {
                current.push(',');
            }
        } else {
            current.push_str(&token);
        }
    }
    if !current.trim().is_empty() {
        clauses.push(current.trim().to_owned());
    }
    clauses
}

fn split_keep(message: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = message.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], '.' | ';' | ',' | '\n') {
            if !buf.is_empty() {
                out.push(std::mem::take(&mut buf));
            }
            out.push(chars[i].to_string());
            i += 1;
        } else if remaining_eq_ignore(&chars, i, " and ") {
            if !buf.is_empty() {
                out.push(std::mem::take(&mut buf));
            }
            out.push(" and ".into());
            i += 5;
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

fn remaining_eq_ignore(chars: &[char], index: usize, needle: &str) -> bool {
    let needle_chars: Vec<char> = needle.chars().collect();
    if index + needle_chars.len() > chars.len() {
        return false;
    }
    chars[index..index + needle_chars.len()]
        .iter()
        .map(|ch| ch.to_ascii_lowercase())
        .eq(needle_chars.iter().copied())
}

fn normalize(clause: &str) -> String {
    let mut text = clause.to_owned();
    for (from, to) in [
        ("I'll", "I will"),
        ("i'll", "i will"),
        ("I'm gonna", "I am going to"),
        ("i'm gonna", "i am going to"),
        ("I'm", "I am"),
        ("i'm", "i am"),
        ("gonna", "going to"),
        ("I'd", "I would"),
        ("i'd", "i would"),
        ("I've", "I have"),
        ("i've", "i have"),
        ("can't", "cannot"),
        ("won't", "will not"),
        ("let's", "let us"),
    ] {
        text = text.replace(from, to);
    }
    text.to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn hard_kill(original: &str, normalized: &str, blocklist: &[String]) -> Option<&'static str> {
    if original.trim_end().ends_with('?') {
        return Some("question");
    }
    if original
        .lines()
        .any(|line| line.trim_start().starts_with('>'))
    {
        return Some("quoted");
    }
    if regex(REQUEST).is_match(normalized) {
        return Some("second_person_request");
    }
    if regex(OPINION).is_match(normalized) {
        return Some("opinion");
    }
    if original.contains("I've") || original.contains("i've") || regex(PAST).is_match(normalized) {
        return Some("past");
    }
    let shape = skeleton(original);
    if blocklist.iter().any(|pattern| pattern == &shape) {
        return Some("blocklist");
    }
    None
}

fn word_count(normalized: &str) -> usize {
    normalized.split_whitespace().count()
}

const COMMISSIVE: &str = r"\b(i will|i am going to|let me|i can)\b";
const VERB: &str = r"\b(send|share|ship|push|draft|review|check|fix|update|email|ping|call|book|schedule|follow up|get back|look into|take care of|circle back)\b";
const TEMPORAL: &str = r"\b(today|tonight|tomorrow|eod|eow|by friday|this week|next week|in an hour|later|shortly|asap|by the \d)";
const OBJECT: &str =
    r"\b(invoice|doc|deck|link|file|pr|notes|draft|quote|contract|numbers|report)\b";
const CONDITIONAL: &str = r"\b(if|unless|in case|assuming)\b";
const ATTENDANCE: &str = r"\bi will be (there|late)\b|\bi am in\b";
const REQUEST: &str =
    r"\b(can you|could you|would you mind|please (send|share|review|check|fix))\b";
const OPINION: &str = r"\b(i think|i would say|i believe|i feel like)\b";
const PAST: &str = r"\b(i sent|i already|i did)\b";

fn regex(pattern: &'static str) -> &'static Regex {
    static COMMISSIVE_RE: OnceLock<Regex> = OnceLock::new();
    static VERB_RE: OnceLock<Regex> = OnceLock::new();
    static TEMPORAL_RE: OnceLock<Regex> = OnceLock::new();
    static OBJECT_RE: OnceLock<Regex> = OnceLock::new();
    static CONDITIONAL_RE: OnceLock<Regex> = OnceLock::new();
    static ATTENDANCE_RE: OnceLock<Regex> = OnceLock::new();
    static REQUEST_RE: OnceLock<Regex> = OnceLock::new();
    static OPINION_RE: OnceLock<Regex> = OnceLock::new();
    static PAST_RE: OnceLock<Regex> = OnceLock::new();
    let cell = match pattern {
        COMMISSIVE => &COMMISSIVE_RE,
        VERB => &VERB_RE,
        TEMPORAL => &TEMPORAL_RE,
        OBJECT => &OBJECT_RE,
        CONDITIONAL => &CONDITIONAL_RE,
        ATTENDANCE => &ATTENDANCE_RE,
        REQUEST => &REQUEST_RE,
        OPINION => &OPINION_RE,
        PAST => &PAST_RE,
        _ => unreachable!("unknown pattern"),
    };
    cell.get_or_init(|| Regex::new(pattern).expect("static regex"))
}
