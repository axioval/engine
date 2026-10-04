//! Calendar dates and date-times with a UTC offset, in ISO 8601.
//!
//! Both are validated on construction and on deserialization, so a value of
//! either type is always a real day of the proleptic Gregorian calendar in
//! the years `0000` to `9999`. The wire form is the ISO 8601 extended form,
//! as XML Schema writes `xs:date` and `xs:dateTime`:
//!
//! - a [`Date`] is `YYYY-MM-DD`, optionally followed by a time zone, `Z`
//!   or `±hh:mm` up to 14 hours, as the `xs:date` lexical space allows;
//! - a [`DateTime`] is `YYYY-MM-DDThh:mm:ss`, an optional fraction of up to
//!   nine digits, and an explicit offset: `Z` or `±hh:mm` up to 14 hours.
//!
//! A date without a time zone is the common case, and its wire form carries
//! none. A zoned date is a different value from the unzoned date naming the
//! same day: XML Schema orders the two only when they lie more than 14 hours
//! apart, and never finds them equal ([`Date::cmp_timeline`]).
//!
//! A date-time without an offset is refused. It names a wall-clock time in
//! an unknown zone, so it cannot be ordered against any other instant; an
//! adapter that meets one reports incomplete evidence instead of guessing a
//! zone. `-00:00` (offset unknown) is refused for the same reason; `24:00:00`
//! and leap seconds are refused as ambiguous.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Why a date or date-time literal was refused.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{literal:?} is not {expected}: {reason}")]
pub struct TemporalError {
    literal: String,
    expected: &'static str,
    reason: &'static str,
}

impl TemporalError {
    fn date(literal: &str, reason: &'static str) -> Self {
        Self {
            literal: literal.to_owned(),
            expected: "an ISO 8601 date (YYYY-MM-DD, optionally with a time zone Z or ±hh:mm)",
            reason,
        }
    }

    fn date_time(literal: &str, reason: &'static str) -> Self {
        Self {
            literal: literal.to_owned(),
            expected: "an ISO 8601 date-time with a UTC offset (YYYY-MM-DDThh:mm:ss±hh:mm)",
            reason,
        }
    }
}

/// How finely a comparison involving a date-time reads it.
///
/// Without a precision a date compares with a date and a date-time with a
/// date-time, as instants. `Day` reads every date-time as the calendar day
/// it states in its own offset (`2026-09-27T23:30:00-05:00` is on the 27th),
/// so a date-time can then be compared with a date, or two date-times by
/// day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum TemporalPrecision {
    /// Compare calendar days only.
    Day,
}

/// A calendar day of the proleptic Gregorian calendar, with the time zone
/// it was stated in, if any.
///
/// Equality and the derived order are structural: by day, then time zone,
/// an unzoned date first. They are not chronological across time zones;
/// [`Date::cmp_timeline`] is XML Schema's order of `xs:date` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
    /// Minutes east of UTC; `None` when the date states no time zone.
    offset_minutes: Option<i16>,
}

impl Date {
    /// The date without a time zone, when it is a real day in the years
    /// `0000` to `9999`.
    pub fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        (year <= 9999 && (1..=12).contains(&month) && day >= 1 && day <= days_in(year, month))
            .then_some(Self {
                year,
                month,
                day,
                offset_minutes: None,
            })
    }

    /// The same day stated in the time zone `offset_minutes` east of UTC,
    /// when the offset is at most 14 hours either way.
    #[must_use]
    pub fn with_offset(self, offset_minutes: i16) -> Option<Self> {
        (offset_minutes.unsigned_abs() <= 14 * 60).then_some(Self {
            offset_minutes: Some(offset_minutes),
            ..self
        })
    }

    /// The time zone the date was stated in, in minutes east of UTC; `None`
    /// when it states none.
    #[must_use]
    pub const fn offset_minutes(self) -> Option<i16> {
        self.offset_minutes
    }

    /// The calendar day the date states, without its time zone.
    #[must_use]
    pub const fn calendar_day(self) -> Self {
        Self {
            offset_minutes: None,
            ..self
        }
    }

    /// XML Schema's order of two `xs:date` values; `None` where it has none.
    ///
    /// Two unzoned dates order by day. Two zoned dates order by the instant
    /// each day begins, so `2026-09-28+12:00` equals `2026-09-27-12:00`. A
    /// zoned and an unzoned date are ordered only when the unzoned one,
    /// placed in any time zone from `-14:00` to `+14:00`, begins on the same
    /// side of the zoned one; otherwise the pair is incomparable, neither
    /// before, equal to nor after the other. They are never equal.
    #[must_use]
    pub fn cmp_timeline(self, other: Self) -> Option<Ordering> {
        const WIDEST: i64 = 14 * 60;
        let start = |date: Self| date.days_since_epoch() * 1440;
        match (self.offset_minutes, other.offset_minutes) {
            (None, None) => Some(start(self).cmp(&start(other))),
            (Some(left), Some(right)) => {
                Some((start(self) - i64::from(left)).cmp(&(start(other) - i64::from(right))))
            }
            (Some(offset), None) => {
                let zoned = start(self) - i64::from(offset);
                let unzoned = start(other);
                if zoned < unzoned - WIDEST {
                    Some(Ordering::Less)
                } else if zoned > unzoned + WIDEST {
                    Some(Ordering::Greater)
                } else {
                    None
                }
            }
            (None, Some(_)) => other.cmp_timeline(self).map(Ordering::reverse),
        }
    }

    #[must_use]
    pub const fn year(self) -> u16 {
        self.year
    }

    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }

    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }

    /// Days since 1970-01-01 (negative before it).
    #[must_use]
    pub fn days_since_epoch(self) -> i64 {
        // Howard Hinnant's `days_from_civil`, with March as the first month
        // so that the leap day ends the year.
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let month = i64::from(self.month);
        let shifted = if month > 2 { month - 3 } else { month + 9 };
        let day_of_year = (153 * shifted + 2) / 5 + i64::from(self.day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// The date `days` after 1970-01-01, when it lies in `0000` to `9999`.
    #[must_use]
    pub fn from_days_since_epoch(days: i64) -> Option<Self> {
        let days = days.checked_add(719_468)?;
        let era = days.div_euclid(146_097);
        let day_of_era = days - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * shifted + 2) / 5 + 1;
        let month = if shifted < 10 {
            shifted + 3
        } else {
            shifted - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        Self::new(
            u16::try_from(year).ok()?,
            u8::try_from(month).ok()?,
            u8::try_from(day).ok()?,
        )
    }
}

const fn leap(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

const fn days_in(year: u16, month: u8) -> u8 {
    match month {
        2 if leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Exactly `N` ASCII digits as a number.
fn digits<const N: usize>(text: &[u8]) -> Option<u32> {
    (text.len() == N && text.iter().all(u8::is_ascii_digit)).then(|| {
        text.iter()
            .fold(0, |total, digit| total * 10 + u32::from(digit - b'0'))
    })
}

/// A time zone `Z` or `±hh:mm` of at most 14 hours; `None` for no text.
///
/// `-00:00` is refused: it conventionally states that the offset is unknown.
fn parse_offset(text: &[u8]) -> Result<Option<i16>, &'static str> {
    match *text {
        [] => Ok(None),
        [b'Z'] => Ok(Some(0)),
        [sign @ (b'+' | b'-'), o0, o1, b':', p0, p1] => {
            let (Some(hours), Some(minutes)) = (digits::<2>(&[o0, o1]), digits::<2>(&[p0, p1]))
            else {
                return Err("expected an offset Z or ±hh:mm");
            };
            if minutes >= 60 || hours * 60 + minutes > 14 * 60 {
                return Err("an offset is at most 14:00");
            }
            if sign == b'-' && hours == 0 && minutes == 0 {
                return Err("-00:00 states no offset");
            }
            // At most 840.
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let minutes = (hours * 60 + minutes) as i16;
            Ok(Some(if sign == b'-' { -minutes } else { minutes }))
        }
        _ => Err("expected an offset Z or ±hh:mm"),
    }
}

/// An offset as written: `Z` for UTC, otherwise `±hh:mm`.
fn write_offset(f: &mut fmt::Formatter<'_>, offset_minutes: i16) -> fmt::Result {
    if offset_minutes == 0 {
        return f.write_str("Z");
    }
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let minutes = offset_minutes.unsigned_abs();
    write!(f, "{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The leading `YYYY-MM-DD` of `text`, and why it is not a date.
fn parse_date(text: &[u8]) -> Result<Date, &'static str> {
    let [y0, y1, y2, y3, b'-', m0, m1, b'-', d0, d1] = *text else {
        return Err("expected YYYY-MM-DD");
    };
    let (Some(year), Some(month), Some(day)) = (
        digits::<4>(&[y0, y1, y2, y3]),
        digits::<2>(&[m0, m1]),
        digits::<2>(&[d0, d1]),
    ) else {
        return Err("expected YYYY-MM-DD");
    };
    // Each fits: at most 9999 and 99.
    #[allow(clippy::cast_possible_truncation)]
    Date::new(year as u16, month as u8, day as u8).ok_or("no such day")
}

impl FromStr for Date {
    type Err = TemporalError;

    fn from_str(literal: &str) -> Result<Self, Self::Err> {
        let error = |reason| TemporalError::date(literal, reason);
        let bytes = literal.as_bytes();
        let date = parse_date(bytes.get(..10).unwrap_or(bytes)).map_err(error)?;
        match parse_offset(bytes.get(10..).unwrap_or_default()).map_err(error)? {
            None => Ok(date),
            Some(offset) => date
                .with_offset(offset)
                .ok_or_else(|| error("an offset is at most 14:00")),
        }
    }
}

impl fmt::Display for Date {
    /// `YYYY-MM-DD`, then the time zone if the date states one: `Z` for
    /// UTC, otherwise `±hh:mm`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)?;
        match self.offset_minutes {
            Some(offset) => write_offset(f, offset),
            None => Ok(()),
        }
    }
}

/// A date and time of day with its UTC offset, to the nanosecond.
///
/// Equality is structural: the same instant written in two offsets is two
/// different values. [`DateTime::cmp_instant`] orders instants, and
/// [`DateTime::date`] is the calendar day the value states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DateTime {
    date: Date,
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
    /// Minutes east of UTC.
    offset_minutes: i16,
}

impl DateTime {
    /// The date-time, when the time of day and the offset are valid.
    ///
    /// The offset is in minutes east of UTC, at most 14 hours either way.
    /// A time zone `date` states is ignored: `offset_minutes` is the one.
    #[must_use]
    pub fn new(
        date: Date,
        (hour, minute, second): (u8, u8, u8),
        nanosecond: u32,
        offset_minutes: i16,
    ) -> Option<Self> {
        (hour < 24
            && minute < 60
            && second < 60
            && nanosecond < 1_000_000_000
            && offset_minutes.unsigned_abs() <= 14 * 60)
            .then_some(Self {
                date: date.calendar_day(),
                hour,
                minute,
                second,
                nanosecond,
                offset_minutes,
            })
    }

    /// The UTC date-time `seconds` after 1970-01-01T00:00:00Z, when it lies
    /// in the years `0000` to `9999`.
    #[must_use]
    pub fn from_unix_seconds(seconds: i64) -> Option<Self> {
        let date = Date::from_days_since_epoch(seconds.div_euclid(86_400))?;
        let of_day = seconds.rem_euclid(86_400);
        // Each is below 24, 60 and 60.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Self::new(
            date,
            (
                (of_day / 3600) as u8,
                (of_day / 60 % 60) as u8,
                (of_day % 60) as u8,
            ),
            0,
            0,
        )
    }

    /// The calendar day the value states, in its own offset, without a time
    /// zone.
    #[must_use]
    pub const fn date(self) -> Date {
        self.date
    }

    /// Hour, minute and second, in its own offset.
    #[must_use]
    pub const fn time(self) -> (u8, u8, u8) {
        (self.hour, self.minute, self.second)
    }

    #[must_use]
    pub const fn nanosecond(self) -> u32 {
        self.nanosecond
    }

    /// Minutes east of UTC.
    #[must_use]
    pub const fn offset_minutes(self) -> i16 {
        self.offset_minutes
    }

    /// Seconds since 1970-01-01T00:00:00Z, and the nanoseconds past them.
    #[must_use]
    pub fn unix_instant(self) -> (i64, u32) {
        let local = self.date.days_since_epoch() * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second);
        (local - i64::from(self.offset_minutes) * 60, self.nanosecond)
    }

    /// The chronological order of the two instants, whatever their offsets.
    #[must_use]
    pub fn cmp_instant(self, other: Self) -> Ordering {
        self.unix_instant().cmp(&other.unix_instant())
    }
}

impl FromStr for DateTime {
    type Err = TemporalError;

    fn from_str(literal: &str) -> Result<Self, Self::Err> {
        let error = |reason| TemporalError::date_time(literal, reason);
        let bytes = literal.as_bytes();
        if bytes.len() < 19 || bytes[10] != b'T' {
            return Err(error("expected YYYY-MM-DDThh:mm:ss"));
        }
        let date = parse_date(&bytes[..10]).map_err(error)?;
        let [h0, h1, b':', i0, i1, b':', s0, s1] = bytes[11..19] else {
            return Err(error("expected hh:mm:ss"));
        };
        let (Some(hour), Some(minute), Some(second)) = (
            digits::<2>(&[h0, h1]),
            digits::<2>(&[i0, i1]),
            digits::<2>(&[s0, s1]),
        ) else {
            return Err(error("expected hh:mm:ss"));
        };
        let mut rest = &bytes[19..];
        let mut nanosecond = 0;
        if let Some(fraction) = rest.strip_prefix(b".") {
            let length = fraction.iter().take_while(|b| b.is_ascii_digit()).count();
            if length == 0 || length > 9 {
                return Err(error("a fraction has one to nine digits"));
            }
            nanosecond = fraction[..length]
                .iter()
                .fold(0, |total, digit| total * 10 + u32::from(digit - b'0'))
                * 10_u32.pow(9 - u32::try_from(length).unwrap_or(9));
            rest = &fraction[length..];
        }
        let Some(offset) = parse_offset(rest).map_err(error)? else {
            return Err(error("the UTC offset is missing"));
        };
        if hour == 24 {
            return Err(error("24:00 is refused; write 00:00 of the next day"));
        }
        if second == 60 {
            return Err(error("a leap second cannot be ordered"));
        }
        // Each is two digits.
        #[allow(clippy::cast_possible_truncation)]
        Self::new(
            date,
            (hour as u8, minute as u8, second as u8),
            nanosecond,
            offset,
        )
        .ok_or_else(|| error("no such time of day"))
    }
}

impl fmt::Display for DateTime {
    /// The canonical form: fraction without trailing zeros, `Z` for UTC.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}T{:02}:{:02}:{:02}",
            self.date, self.hour, self.minute, self.second
        )?;
        if self.nanosecond != 0 {
            let fraction = format!("{:09}", self.nanosecond);
            write!(f, ".{}", fraction.trim_end_matches('0'))?;
        }
        write_offset(f, self.offset_minutes)
    }
}

macro_rules! lexical_serde {
    ($type:ty, $expecting:literal) => {
        impl Serialize for $type {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct Visitor;
                impl serde::de::Visitor<'_> for Visitor {
                    type Value = $type;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str($expecting)
                    }
                    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<$type, E> {
                        text.parse().map_err(E::custom)
                    }
                }
                deserializer.deserialize_str(Visitor)
            }
        }
    };
}

lexical_serde!(Date, "an ISO 8601 date string, optionally with a time zone");
lexical_serde!(DateTime, "an ISO 8601 date-time string with a UTC offset");

/// A UTC offset as the readers accept it: `Z`, or `±hh:mm` up to 14 hours.
#[cfg(feature = "schema")]
const OFFSET_PATTERN: &str = "(Z|[+-](0[0-9]|1[0-3]):[0-5][0-9]|[+-]14:00)";
/// `YYYY-MM-DD`, months and days in range; whether the day exists in that
/// month is checked when the package is read.
#[cfg(feature = "schema")]
const DATE_PATTERN: &str = "[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])";

/// The wire form is the lexical one the custom `Serialize` writes.
#[cfg(feature = "schema")]
impl schemars::JsonSchema for Date {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Date".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "An ISO 8601 calendar date, `YYYY-MM-DD`, optionally with a time zone `Z` or `±hh:mm`.",
            "type": "string",
            "pattern": format!("^{DATE_PATTERN}{OFFSET_PATTERN}?$"),
        })
    }
}

/// The wire form is the lexical one the custom `Serialize` writes.
#[cfg(feature = "schema")]
impl schemars::JsonSchema for DateTime {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DateTime".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "An ISO 8601 date-time with an explicit UTC offset, `YYYY-MM-DDThh:mm:ss[.f](Z|±hh:mm)`.",
            "type": "string",
            "pattern": format!(
                "^{DATE_PATTERN}T([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\\.[0-9]{{1,9}})?{OFFSET_PATTERN}$"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Date, DateTime};
    use std::cmp::Ordering;

    #[test]
    fn epoch_days_round_trip_across_the_calendar() {
        assert_eq!(Date::new(1970, 1, 1).unwrap().days_since_epoch(), 0);
        assert_eq!(Date::new(2000, 3, 1).unwrap().days_since_epoch(), 11_017);
        for days in [-719_528, -1, 0, 59, 60, 11_016, 20_000, 2_932_896] {
            let date = Date::from_days_since_epoch(days).unwrap();
            assert_eq!(date.days_since_epoch(), days, "{date}");
        }
        assert_eq!(Date::from_days_since_epoch(-719_529), None);
        assert_eq!(Date::from_days_since_epoch(2_932_897), None);
    }

    #[test]
    fn unix_seconds_read_as_utc() {
        let stamp = DateTime::from_unix_seconds(1_700_000_000).unwrap();
        assert_eq!(stamp.to_string(), "2023-11-14T22:13:20Z");
        assert_eq!(stamp.unix_instant(), (1_700_000_000, 0));
        let before = DateTime::from_unix_seconds(-1).unwrap();
        assert_eq!(before.to_string(), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn instants_order_across_offsets() {
        let berlin: DateTime = "2026-09-27T10:00:00+02:00".parse().unwrap();
        let utc: DateTime = "2026-09-27T08:00:00Z".parse().unwrap();
        assert_ne!(berlin, utc);
        assert_eq!(berlin.cmp_instant(utc), Ordering::Equal);
        let later: DateTime = "2026-09-27T08:00:00.000000001Z".parse().unwrap();
        assert_eq!(berlin.cmp_instant(later), Ordering::Less);
    }
}
