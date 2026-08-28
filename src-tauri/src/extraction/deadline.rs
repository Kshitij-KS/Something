use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveTime, Utc};
use regex::Regex;
use std::sync::OnceLock;

/// Precision retained from the expression that produced a deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlinePrecision {
    Minute,
    Hour,
    Day,
    Eod,
    Eow,
}

impl DeadlinePrecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Eod => "eod",
            Self::Eow => "eow",
        }
    }
}

/// Parsed deadline stored as UTC with the original local timezone label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDeadline {
    pub utc_ts: i64,
    pub tz_label: String,
    pub precision: DeadlinePrecision,
}

/// Explicit lexicon used to resolve relative deadlines.
#[derive(Debug, Clone)]
pub struct DeadlineLexicon {
    /// Local wall-clock used for EOD.
    pub eod: NaiveTime,
    /// Monday-based weekday index for EOW (Friday = 4).
    pub eow_weekday: u32,
}

impl Default for DeadlineLexicon {
    fn default() -> Self {
        Self {
            eod: NaiveTime::from_hms_opt(17, 0, 0).unwrap_or(NaiveTime::MIN),
            eow_weekday: 4,
        }
    }
}

/// Parses a temporal anchor against `now` in `offset_seconds`.
#[must_use]
pub fn parse_deadline(
    clause: &str,
    now_utc: DateTime<Utc>,
    offset_seconds: i32,
    tz_label: &str,
    lexicon: &DeadlineLexicon,
) -> Option<ParsedDeadline> {
    let offset = FixedOffset::east_opt(offset_seconds)?;
    let now_local = now_utc.with_timezone(&offset);
    let lower = clause.to_ascii_lowercase();
    let (local, precision) = if lower.contains("in an hour") {
        (now_local + Duration::hours(1), DeadlinePrecision::Minute)
    } else if lower.contains("tomorrow") {
        (
            at_eod(now_local.date_naive() + Duration::days(1), lexicon, offset)?,
            DeadlinePrecision::Eod,
        )
    } else if lower.contains("tonight") {
        (
            now_local
                .date_naive()
                .and_time(NaiveTime::from_hms_opt(21, 0, 0)?)
                .and_local_timezone(offset)
                .single()?,
            DeadlinePrecision::Hour,
        )
    } else if lower.contains("eod") || lower.contains("end of day") || has_word(&lower, "today") {
        (
            at_eod(now_local.date_naive(), lexicon, offset)?,
            DeadlinePrecision::Eod,
        )
    } else if lower.contains("eow")
        || lower.contains("end of week")
        || lower.contains("this week")
        || lower.contains("by friday")
    {
        (
            at_eow(now_local.date_naive(), lexicon, offset)?,
            DeadlinePrecision::Eow,
        )
    } else if lower.contains("next week") {
        (
            at_eow(now_local.date_naive() + Duration::weeks(1), lexicon, offset)?,
            DeadlinePrecision::Eow,
        )
    } else if let Some(day) = parse_ordinal_day(&lower) {
        (
            next_ordinal(now_local.date_naive(), day, lexicon, offset)?,
            DeadlinePrecision::Day,
        )
    } else {
        return None;
    };
    Some(ParsedDeadline {
        utc_ts: local.with_timezone(&Utc).timestamp(),
        tz_label: tz_label.to_owned(),
        precision,
    })
}

fn has_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .any(|token| token == word)
}

fn at_eod(
    date: NaiveDate,
    lexicon: &DeadlineLexicon,
    offset: FixedOffset,
) -> Option<DateTime<FixedOffset>> {
    date.and_time(lexicon.eod)
        .and_local_timezone(offset)
        .single()
}

fn at_eow(
    date: NaiveDate,
    lexicon: &DeadlineLexicon,
    offset: FixedOffset,
) -> Option<DateTime<FixedOffset>> {
    let current = date.weekday().num_days_from_monday();
    let delta = (i64::from(lexicon.eow_weekday) + 7 - i64::from(current)) % 7;
    at_eod(date + Duration::days(delta), lexicon, offset)
}

fn ordinal_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bby the (\d{1,2})(?:st|nd|rd|th)?\b").expect("ordinal regex"))
}

fn parse_ordinal_day(lower: &str) -> Option<u32> {
    let caps = ordinal_re().captures(lower)?;
    caps.get(1)?.as_str().parse().ok()
}

fn next_ordinal(
    today: NaiveDate,
    day: u32,
    lexicon: &DeadlineLexicon,
    offset: FixedOffset,
) -> Option<DateTime<FixedOffset>> {
    let mut year = today.year();
    let mut month = today.month();
    for _ in 0..3 {
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            if date >= today {
                return at_eod(date, lexicon, offset);
            }
        }
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    None
}
