//! IFC date and time defined types as IR dates and date-times.
//!
//! - `IfcDate` (a `STRING`, ISO 8601 `YYYY-MM-DD`) is a [`Date`].
//! - `IfcDateTime` (a `STRING`, ISO 8601 `YYYY-MM-DDThh:mm:ss`) is a
//!   [`DateTime`] when it states a UTC offset. Without one its instant is
//!   unknown, and the value is refused as incomplete; no zone is guessed.
//! - `IfcTimeStamp` (an `INTEGER`, seconds since 1970-01-01 UTC) is a UTC
//!   [`DateTime`].
//!
//! Text that is not the type's ISO 8601 form is an invalid value, never text.

use axioval_engine::PropertyResolutionError;
use axioval_ir::{Date, DateTime, PropertyValue};

/// A stated value, as far as the date and time types read it.
pub(crate) enum Raw<'a> {
    Text(&'a str),
    Integer(i64),
    Other,
}

/// The value of `type_name` when it is a date or time type; `None` otherwise.
pub(crate) fn read(
    type_name: &str,
    raw: &Raw<'_>,
) -> Option<Result<PropertyValue, PropertyResolutionError>> {
    let name = type_name.to_ascii_uppercase();
    Some(match (name.as_str(), raw) {
        ("IFCDATE", Raw::Text(text)) => text
            .parse::<Date>()
            .map(PropertyValue::Date)
            .map_err(|_| PropertyResolutionError::InvalidValue),
        ("IFCDATETIME", Raw::Text(text)) => match text.parse::<DateTime>() {
            Ok(value) => Ok(PropertyValue::DateTime(value)),
            Err(_) if format!("{text}Z").parse::<DateTime>().is_ok() => {
                Err(PropertyResolutionError::Incomplete(format!(
                    "IFCDATETIME '{text}' states no UTC offset, so its instant is unknown"
                )))
            }
            Err(_) => Err(PropertyResolutionError::InvalidValue),
        },
        ("IFCTIMESTAMP", Raw::Integer(seconds)) => DateTime::from_unix_seconds(*seconds)
            .map(PropertyValue::DateTime)
            .ok_or(PropertyResolutionError::InvalidValue),
        ("IFCDATE" | "IFCDATETIME" | "IFCTIMESTAMP", _) => {
            Err(PropertyResolutionError::InvalidValue)
        }
        _ => return None,
    })
}
