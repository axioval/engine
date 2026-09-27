//! `axioval:derived.same-level`: levels of several sources matched as one.
#![allow(missing_docs)]

use axioval_engine::{LevelFacts, LevelMatch, RelationshipSelectionError};

fn at(elevation: f64) -> LevelFacts {
    LevelFacts {
        elevation_metres: Some(elevation),
        name: None,
    }
}

fn named(name: &str) -> LevelFacts {
    LevelFacts {
        elevation_metres: None,
        name: Some(name.into()),
    }
}

#[test]
fn the_identity_parses_with_defaults_and_prints_canonically() {
    let parsed = LevelMatch::parse("axioval:derived.same-level").unwrap();
    assert_eq!(
        parsed,
        Some(LevelMatch::Elevation {
            tolerance_metres: 0.0
        })
    );
    assert_eq!(
        parsed.unwrap().to_string(),
        "axioval:derived.same-level;by=elevation;tolerance=0"
    );
    assert_eq!(
        LevelMatch::parse("axioval:derived.same-level;tolerance=0.05").unwrap(),
        Some(LevelMatch::Elevation {
            tolerance_metres: 0.05
        })
    );
    assert_eq!(
        LevelMatch::parse("axioval:derived.same-level;by=name").unwrap(),
        Some(LevelMatch::Name)
    );
    for other in [
        "IfcRelContainedInSpatialStructure",
        "axioval:derived.contained-in-space",
    ] {
        assert_eq!(LevelMatch::parse(other).unwrap(), None, "{other}");
    }
    for invalid in [
        "axioval:derived.same-level;tolerance=-1",
        "axioval:derived.same-level;tolerance=x",
        "axioval:derived.same-level;by=name;tolerance=0.1",
        "axioval:derived.same-level;by=height",
        "axioval:derived.same-level;by=name;by=name",
        "axioval:derived.same-level;reach=1",
        "axioval:derived.same-level;by",
    ] {
        assert_eq!(
            LevelMatch::parse(invalid),
            Err(RelationshipSelectionError::InvalidRequest),
            "{invalid}"
        );
    }
}

#[test]
fn levels_match_by_elevation_within_the_tolerance_or_by_name() {
    let exact = LevelMatch::Elevation {
        tolerance_metres: 0.0,
    };
    assert_eq!(exact.same(&at(3.0), &at(3.000_000_000_1)), Some(true));
    assert_eq!(exact.same(&at(3.0), &at(3.01)), Some(false));
    let loose = LevelMatch::Elevation {
        tolerance_metres: 0.05,
    };
    assert_eq!(loose.same(&at(3.0), &at(3.05)), Some(true));
    assert_eq!(loose.same(&at(3.0), &at(3.06)), Some(false));
    assert_eq!(loose.same(&at(3.0), &named("Level 1")), None);
    assert_eq!(
        LevelMatch::Name.same(&named("Level 1"), &named("Level 1")),
        Some(true)
    );
    assert_eq!(
        LevelMatch::Name.same(&named("Level 1"), &named("level 1")),
        Some(false)
    );
    assert_eq!(LevelMatch::Name.same(&named("Level 1"), &at(3.0)), None);
}
