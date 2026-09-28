//! Dates and date-times: strict ISO 8601 literals, wire forms and ordering.
#![allow(missing_docs)]

use std::cmp::Ordering;

use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Date, DateTime, PropertyValue, TemporalPrecision};
use serde_json::json;

fn date(text: &str) -> Date {
    text.parse().unwrap()
}

fn date_time(text: &str) -> DateTime {
    text.parse().unwrap()
}

#[test]
fn a_date_is_a_real_calendar_day_in_extended_form() {
    let parsed = date("2026-09-27");
    assert_eq!((parsed.year(), parsed.month(), parsed.day()), (2026, 9, 27));
    assert_eq!(parsed.to_string(), "2026-09-27");
    assert_eq!(date("2024-02-29").to_string(), "2024-02-29");
    assert_eq!(date("2000-02-29").to_string(), "2000-02-29");
    assert_eq!(date("0000-01-01").to_string(), "0000-01-01");
    for bad in [
        "2026-02-29",
        "1900-02-29",
        "2026-04-31",
        "2026-13-01",
        "2026-00-10",
        "2026-01-00",
        "20260927",
        "2026-9-27",
        "26-09-27",
        "+2026-09-27",
        "2026-09-27z",
        "2026-09-27+0200",
        "2026-09-27+14:30",
        "2026-09-27-00:00",
        "2026-09-27T00:00:00Z",
        " 2026-09-27",
        "２０２６-09-27",
        "",
    ] {
        assert!(bad.parse::<Date>().is_err(), "{bad:?}");
    }
}

#[test]
fn a_date_may_state_a_time_zone() {
    let zoned = date("2022-01-01+00:00");
    assert_eq!((zoned.year(), zoned.month(), zoned.day()), (2022, 1, 1));
    assert_eq!(zoned.offset_minutes(), Some(0));
    // UTC is written `Z`, as for a date-time.
    assert_eq!(zoned.to_string(), "2022-01-01Z");
    assert_eq!(date("2022-01-01Z"), zoned);
    assert_eq!(date("2022-01-01-09:30").offset_minutes(), Some(-570));
    assert_eq!(date("2022-01-01+14:00").to_string(), "2022-01-01+14:00");
    assert_eq!(date("2022-01-01").offset_minutes(), None);
    assert_eq!(zoned.calendar_day(), date("2022-01-01"));
    assert_ne!(zoned, date("2022-01-01"), "a zoned date is another value");
    assert_eq!(Date::new(2022, 1, 1).unwrap().with_offset(841), None);
    // A date-time's day carries no zone of its own.
    assert_eq!(
        DateTime::new(zoned, (1, 0, 0), 0, 60).unwrap().date(),
        date("2022-01-01")
    );
}

#[test]
fn zoned_and_unzoned_dates_order_as_xml_schema_orders_them() {
    let order = |left: &str, right: &str| date(left).cmp_timeline(date(right));
    assert_eq!(order("2022-01-01", "2022-01-02"), Some(Ordering::Less));
    assert_eq!(order("2022-01-01", "2022-01-01"), Some(Ordering::Equal));
    // Zoned dates compare the instants their days begin.
    assert_eq!(
        order("2022-01-02+12:00", "2022-01-01-12:00"),
        Some(Ordering::Equal)
    );
    assert_eq!(
        order("2022-01-01+01:00", "2022-01-01Z"),
        Some(Ordering::Less)
    );
    // Within 14 hours either way a zoned and an unzoned date are
    // incomparable, so never equal.
    assert_eq!(order("2022-01-01Z", "2022-01-01"), None);
    assert_eq!(order("2022-01-01", "2022-01-01+14:00"), None);
    assert_eq!(order("2022-01-01-14:00", "2022-01-01"), None);
    assert_eq!(order("2022-01-02Z", "2022-01-01"), Some(Ordering::Greater));
    assert_eq!(order("2022-01-01", "2022-01-02Z"), Some(Ordering::Less));
    assert_eq!(
        order("2022-01-01-09:00", "2022-01-02"),
        Some(Ordering::Less)
    );
    // Exactly 14 hours apart is still indeterminate: the order is strict.
    assert_eq!(order("2022-01-01-10:00", "2022-01-02"), None);
}

#[test]
fn a_date_time_states_its_offset() {
    let parsed = date_time("2026-09-27T10:30:05.250+02:00");
    assert_eq!(parsed.date(), date("2026-09-27"));
    assert_eq!(parsed.time(), (10, 30, 5));
    assert_eq!(parsed.nanosecond(), 250_000_000);
    assert_eq!(parsed.offset_minutes(), 120);
    // The canonical form drops trailing fraction zeros and writes UTC as `Z`.
    assert_eq!(parsed.to_string(), "2026-09-27T10:30:05.25+02:00");
    assert_eq!(
        date_time("2026-09-27T10:30:05+00:00").to_string(),
        "2026-09-27T10:30:05Z"
    );
    assert_eq!(
        date_time("2026-09-27T10:30:05-09:30").offset_minutes(),
        -570
    );
    assert_eq!(
        date_time("2026-09-27T10:30:05.123456789+14:00").to_string(),
        "2026-09-27T10:30:05.123456789+14:00"
    );
    for (bad, why) in [
        ("2026-09-27T10:30:05", "the UTC offset is missing"),
        ("2026-09-27T10:30:05-00:00", "-00:00 states no offset"),
        ("2026-09-27T24:00:00Z", "24:00 is refused"),
        ("2026-09-27T23:59:60Z", "leap second"),
        ("2026-09-27T10:30Z", "expected YYYY-MM-DDThh:mm:ss"),
        ("2026-09-27 10:30:05Z", "expected YYYY-MM-DDThh:mm:ss"),
        ("2026-09-27T10:30:05+14:30", "at most 14:00"),
        ("2026-09-27T10:30:05+02:60", "at most 14:00"),
        ("2026-09-27T10:30:05+0200", "expected an offset"),
        ("2026-09-27T10:30:05z", "expected an offset"),
        ("2026-09-27T10:30:05.Z", "one to nine digits"),
        ("2026-09-27T10:30:05.1234567890Z", "one to nine digits"),
        ("2026-09-27T10:61:05Z", "no such time of day"),
        ("2026-02-30T10:30:05Z", "no such day"),
    ] {
        let error = bad.parse::<DateTime>().unwrap_err().to_string();
        assert!(error.contains(why), "{bad:?}: {error}");
    }
}

#[test]
fn wire_forms_are_iso_8601_strings() {
    let value = PropertyValue::Date(date("2026-09-27"));
    let wire = json!({"type": "date", "value": "2026-09-27"});
    assert_eq!(serde_json::to_value(&value).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<PropertyValue>(wire).unwrap(),
        value
    );

    let value = PropertyValue::DateTime(date_time("2026-09-27T10:30:00+02:00"));
    let wire = json!({"type": "dateTime", "value": "2026-09-27T10:30:00+02:00"});
    assert_eq!(serde_json::to_value(&value).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<PropertyValue>(wire).unwrap(),
        value
    );

    // A zoned date keeps its zone on the wire; an unzoned one gains none.
    let value = PropertyValue::Date(date("2022-01-01+00:00"));
    let wire = json!({"type": "date", "value": "2022-01-01Z"});
    assert_eq!(serde_json::to_value(&value).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<PropertyValue>(wire).unwrap(),
        value
    );

    for bad in [
        json!({"type": "date", "value": "2026-02-30"}),
        json!({"type": "date", "value": 20_260_927}),
        json!({"type": "dateTime", "value": "2026-09-27T10:30:00"}),
    ] {
        assert!(
            serde_json::from_value::<PropertyValue>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn package_literals_are_validated_when_read() {
    let literal: ParameterValue =
        serde_json::from_value(json!({"type": "date", "value": "2026-09-27"})).unwrap();
    assert_eq!(
        literal,
        ParameterValue::Date {
            value: date("2026-09-27")
        }
    );
    let literal: ParameterValue =
        serde_json::from_value(json!({"type": "dateTime", "value": "2026-09-27T08:00:00Z"}))
            .unwrap();
    assert_eq!(
        literal,
        ParameterValue::DateTime {
            value: date_time("2026-09-27T08:00:00Z")
        }
    );
    for bad in [
        json!({"type": "date", "value": "27.09.2026"}),
        json!({"type": "dateTime", "value": "2026-09-27T08:00:00"}),
    ] {
        assert!(
            serde_json::from_value::<ParameterValue>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn a_selector_states_day_precision_and_omits_it_when_unset() {
    let wire = json!({
        "kind": "property",
        "propertySet": "Pset_Inspection",
        "property": "InspectedOn",
        "operator": "greaterThanOrEquals",
        "value": {"type": "date", "value": "2026-01-01"},
        "precision": "day",
    });
    let selector: Selector = serde_json::from_value(wire.clone()).unwrap();
    let Selector::Property { precision, .. } = &selector else {
        panic!("{selector:?}");
    };
    assert_eq!(*precision, Some(TemporalPrecision::Day));
    assert_eq!(serde_json::to_value(&selector).unwrap(), wire);

    let mut unset = wire;
    unset.as_object_mut().unwrap().remove("precision");
    let selector: Selector = serde_json::from_value(unset.clone()).unwrap();
    assert_eq!(serde_json::to_value(&selector).unwrap(), unset);
}

#[test]
fn date_times_order_as_instants_and_dates_by_day() {
    let berlin = date_time("2026-09-27T10:00:00+02:00");
    let utc = date_time("2026-09-27T08:00:00Z");
    let new_york = date_time("2026-09-27T04:00:00-04:00");
    assert_eq!(berlin.cmp_instant(utc), Ordering::Equal);
    assert_eq!(berlin.cmp_instant(new_york), Ordering::Equal);
    assert_ne!(berlin, utc, "equality is structural");
    assert_eq!(
        date_time("2026-09-27T23:30:00-05:00").cmp_instant(date_time("2026-09-28T01:00:00Z")),
        Ordering::Greater
    );
    // The stated day, not the UTC day.
    assert_eq!(
        date_time("2026-09-27T23:30:00-05:00").date(),
        date("2026-09-27")
    );
    assert!(date("2026-09-27") < date("2026-10-01"));
    assert!(date("2025-12-31") < date("2026-01-01"));
}
