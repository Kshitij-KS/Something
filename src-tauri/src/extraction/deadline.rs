use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveTime, TimeZone, Utc};
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

/// Parses a temporal anchor against `now` in an IANA timezone when possible.
/// The fixed offset is retained as a compatibility fallback for older callers.
#[must_use]
pub fn parse_deadline(
    clause: &str,
    now_utc: DateTime<Utc>,
    offset_seconds: i32,
    tz_label: &str,
    lexicon: &DeadlineLexicon,
) -> Option<ParsedDeadline> {
    if let Ok(timezone) = tz_label.parse::<chrono_tz::Tz>() {
        return parse_in_zone(clause, now_utc, timezone, tz_label, lexicon);
    }
    let offset = FixedOffset::east_opt(offset_seconds)?;
    parse_in_zone(clause, now_utc, offset, tz_label, lexicon)
}

fn parse_in_zone<Tz>(
    clause: &str,
    now_utc: DateTime<Utc>,
    timezone: Tz,
    tz_label: &str,
    lexicon: &DeadlineLexicon,
) -> Option<ParsedDeadline>
where
    Tz: TimeZone + Copy,
{
    let now_local = now_utc.with_timezone(&timezone);
    let lower = clause.to_ascii_lowercase();
    let (utc_ts, precision) = if lower.contains("in an hour") {
        (
            (now_local + Duration::hours(1))
                .with_timezone(&Utc)
                .timestamp(),
            DeadlinePrecision::Minute,
        )
    } else if lower.contains("tomorrow") {
        (
            at_eod(
                now_local.date_naive() + Duration::days(1),
                lexicon,
                timezone,
            )?
            .with_timezone(&Utc)
            .timestamp(),
            DeadlinePrecision::Eod,
        )
    } else if lower.contains("tonight") {
        (
            local_datetime(
                now_local.date_naive(),
                NaiveTime::from_hms_opt(21, 0, 0)?,
                timezone,
            )?
            .with_timezone(&Utc)
            .timestamp(),
            DeadlinePrecision::Hour,
        )
    } else if lower.contains("eod") || lower.contains("end of day") || has_word(&lower, "today") {
        (
            at_eod(now_local.date_naive(), lexicon, timezone)?
                .with_timezone(&Utc)
                .timestamp(),
            DeadlinePrecision::Eod,
        )
    } else if lower.contains("eow")
        || lower.contains("end of week")
        || lower.contains("this week")
        || lower.contains("by friday")
    {
        (
            at_eow(now_local.date_naive(), lexicon, timezone)?
                .with_timezone(&Utc)
                .timestamp(),
            DeadlinePrecision::Eow,
        )
    } else if lower.contains("next week") {
        (
            at_eow(
                now_local.date_naive() + Duration::weeks(1),
                lexicon,
                timezone,
            )?
            .with_timezone(&Utc)
            .timestamp(),
            DeadlinePrecision::Eow,
        )
    } else if let Some(day) = parse_ordinal_day(&lower) {
        (
            next_ordinal(now_local.date_naive(), day, lexicon, timezone)?
                .with_timezone(&Utc)
                .timestamp(),
            DeadlinePrecision::Day,
        )
    } else {
        return None;
    };
    Some(ParsedDeadline {
        utc_ts,
        tz_label: tz_label.to_owned(),
        precision,
    })
}

fn has_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .any(|token| token == word)
}

fn local_datetime<Tz: TimeZone>(
    date: NaiveDate,
    time: NaiveTime,
    timezone: Tz,
) -> Option<DateTime<Tz>> {
    date.and_time(time).and_local_timezone(timezone).single()
}

fn at_eod<Tz: TimeZone>(
    date: NaiveDate,
    lexicon: &DeadlineLexicon,
    timezone: Tz,
) -> Option<DateTime<Tz>> {
    local_datetime(date, lexicon.eod, timezone)
}

fn at_eow<Tz: TimeZone + Copy>(
    date: NaiveDate,
    lexicon: &DeadlineLexicon,
    timezone: Tz,
) -> Option<DateTime<Tz>> {
    let current = date.weekday().num_days_from_monday();
    let delta = (i64::from(lexicon.eow_weekday) + 7 - i64::from(current)) % 7;
    at_eod(date + Duration::days(delta), lexicon, timezone)
}

fn ordinal_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bby the (\d{1,2})(?:st|nd|rd|th)?\b").expect("ordinal regex"))
}

fn parse_ordinal_day(lower: &str) -> Option<u32> {
    let caps = ordinal_re().captures(lower)?;
    caps.get(1)?.as_str().parse().ok()
}

fn next_ordinal<Tz: TimeZone + Copy>(
    today: NaiveDate,
    day: u32,
    lexicon: &DeadlineLexicon,
    timezone: Tz,
) -> Option<DateTime<Tz>> {
    let mut year = today.year();
    let mut month = today.month();
    for _ in 0..3 {
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            if date >= today {
                return at_eod(date, lexicon, timezone);
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
