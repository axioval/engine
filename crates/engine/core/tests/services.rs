//! Typed host-service registry behavior.
#![allow(missing_docs)]
use axioval_engine::{ServiceRegistry, ServiceRegistryError};
#[derive(Debug, PartialEq)]
struct UnitScale(f64);
#[test]
fn services_are_type_safe_and_duplicate_registration_fails() {
    let mut services = ServiceRegistry::new();
    services.register(UnitScale(0.001)).unwrap();
    assert_eq!(services.get::<UnitScale>(), Some(&UnitScale(0.001)));
    assert_eq!(
        services.register(UnitScale(1.0)),
        Err(ServiceRegistryError::Duplicate)
    );
}

mod memo {
    use std::cell::Cell;

    use axioval_engine::{MeasuredMemo, ServiceRegistry};

    #[test]
    fn a_measurement_is_taken_once_per_key() {
        let memo = MeasuredMemo::default();
        let taken = Cell::new(0);
        let measure = |value: u32| {
            taken.set(taken.get() + 1);
            value
        };
        assert_eq!(memo.get_or_measure(("wall", 1_u8), || measure(10)), 10);
        assert_eq!(memo.get_or_measure(("wall", 1_u8), || measure(20)), 10);
        assert_eq!(memo.get_or_measure(("wall", 2_u8), || measure(30)), 30);
        assert_eq!(taken.get(), 2);
    }

    #[test]
    fn keys_and_values_of_other_types_never_meet() {
        let memo = MeasuredMemo::default();
        memo.insert("wall", 1_u32);
        memo.insert("wall", "one");
        assert_eq!(memo.get::<&str, u32>(&"wall"), Some(1));
        assert_eq!(memo.get::<&str, &str>(&"wall"), Some("one"));
        assert_eq!(memo.get::<&str, u64>(&"wall"), None);
        // An insert replaces what was memoized for its key.
        memo.insert("wall", 2_u32);
        assert_eq!(memo.get_with(&"wall", |value: &u32| value * 10), Some(20));
    }

    #[test]
    fn without_a_run_nothing_is_memoized() {
        let services = ServiceRegistry::new();
        let taken = Cell::new(0);
        for _ in 0..2 {
            MeasuredMemo::of(&services, "wall", || taken.set(taken.get() + 1));
        }
        assert_eq!(taken.get(), 2);
    }

    #[test]
    fn clones_share_what_they_memoized() {
        let memo = MeasuredMemo::default();
        let shared = memo.clone();
        memo.insert(1_u8, 'a');
        assert_eq!(shared.get::<u8, char>(&1), Some('a'));
    }
}
