//! Keyed limits on the unordered pair of space types on either face of a
//! door or window (`pair_key`), with wildcard rows and the reserved
//! `exterior` key.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, ElevationInterval, VerticalExtent, VerticalExtentError,
    VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension};
use axioval_rules::KeyedLimit;
use common::{Model, findings, id, kind, number, property, rule, source, string, strings};

const ID: &str = "axioval:capability.keyed-limit";
const ADJACENT: &str = "axioval:derived.adjacent-space";

/// An adjacency edge from `element` to `space` found on `side`.
fn edge(element: &str, space: &str, side: char) -> String {
    format!(
        "{ADJACENT};reach=1:{}->{}:side={side}(1.000000,0.000000):entered=0.000000",
        id(element),
        id(space)
    )
}

/// A face of `element` that enters no space.
fn outside(element: &str, side: char) -> String {
    format!(
        "{ADJACENT};reach=1:{}:side={side}(1.000000,0.000000):outside:reach=1",
        id(element)
    )
}

/// `element` of `kind` between `first` on its `+` face and `second` on its
/// `-` face, each a space or the outside (`None`).
fn joins(
    model: Model,
    element: &str,
    kind: &str,
    first: Option<&str>,
    second: Option<&str>,
) -> Model {
    let mut model = model.object(element, kind);
    for (side, space) in [('+', first), ('-', second)] {
        model = match space {
            Some(space) => model.edge(ADJACENT, element, space).cite(
                ADJACENT,
                element,
                &edge(element, space, side),
            ),
            None => model.cite(ADJACENT, element, &outside(element, side)),
        };
    }
    model
}

/// Offices `o1` and `o2`, corridor `k`, stair `s` and a space `x` stating
/// no use.
fn plan() -> Model {
    let mut model = Model::default();
    for (space, use_class) in [
        ("o1", Some("office")),
        ("o2", Some("office")),
        ("k", Some("corridor")),
        ("s", Some("stair")),
        ("x", None),
    ] {
        model = model.object(space, "space");
        if let Some(use_class) = use_class {
            model = model.text(space, "Pset", "Use", use_class);
        }
    }
    model
}

/// A row naming the two sides (blank when `None`) and a minimum.
fn row(first: Option<&str>, second: Option<&str>, minimum: f64) -> TableRow {
    let mut cells = TableRow::new();
    if let Some(first) = first {
        cells.insert("key_1".into(), string(first));
    }
    if let Some(second) = second {
        cells.insert("other_side".into(), string(second));
    }
    cells.insert("minimum".into(), number(minimum));
    cells
}

/// Clear widths per pair of spaces a door connects.
fn width_limits() -> Vec<TableRow> {
    vec![
        row(Some("office"), Some("corridor"), 0.9),
        row(Some("office"), Some("office"), 0.8),
        row(Some("office"), Some("*"), 1.0),
        row(Some("corridor"), Some("exterior"), 1.2),
    ]
}

fn width_keys(limits: Vec<TableRow>) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("limits", ParameterValue::Table { value: limits }),
        ("quantity", string("clear-width")),
        ("quantity_property", property(Some("Pset"), "ClearWidth")),
        ("key_1", property(Some("Pset"), "Use")),
        ("key_1_path", strings(&[ADJACENT])),
        ("pair_key", string("key_1")),
    ]
}

fn width(model: Model, door: &str, metres: f64) -> Model {
    model.value(
        door,
        "Pset",
        "ClearWidth",
        PropertyValue::Quantity {
            value: metres,
            dimension: QuantityDimension::Length,
        },
    )
}

fn judge(model: Model, parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    model.evaluate(&KeyedLimit, &rule(ID, kind("door"), parameters))
}

fn unevaluated(evaluation: &CapabilityEvaluation) -> Vec<(String, NotEvaluatedReason, String)> {
    evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| {
            (
                outcome
                    .object_id()
                    .map_or_else(|| "-".to_owned(), |object| object.local_id.clone()),
                outcome.reason().clone(),
                outcome.message().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_pair_row_applies_whichever_side_is_which() {
    // d1 has the corridor on its + face, d2 on its - face: both take the
    // office / corridor row, never the office / * one.
    let model = joins(plan(), "d1", "door", Some("k"), Some("o1"));
    let model = joins(model, "d2", "door", Some("o1"), Some("k"));
    let model = width(width(model, "d1", 0.85), "d2", 0.95);
    let evaluation = judge(model, width_keys(width_limits()));
    assert_eq!(
        findings(&evaluation),
        [(
            "d1".into(),
            "clear width (Pset.ClearWidth) is 0.85 m; required at least 0.9 m (limit row 0: \
             Pset.Use (via axioval:derived.adjacent-space) `corridor` and `office`)"
                .into()
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn the_row_naming_both_sides_wins_over_a_wildcard() {
    // Between two offices 0.85 m meets office / office (0.8 m), although
    // office / * asks 1 m; between an office and a stair only office / *
    // applies.
    let model = joins(plan(), "d1", "door", Some("o1"), Some("o2"));
    let model = joins(model, "d2", "door", Some("s"), Some("o2"));
    let model = width(width(model, "d1", 0.85), "d2", 0.85);
    let evaluation = judge(model, width_keys(width_limits()));
    assert_eq!(
        findings(&evaluation),
        [(
            "d2".into(),
            "clear width (Pset.ClearWidth) is 0.85 m; required at least 1 m (limit row 2: \
             Pset.Use (via axioval:derived.adjacent-space) `stair` and `office`)"
                .into()
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_face_to_the_outside_is_the_reserved_exterior_key() {
    // d1 leads from the corridor outside; d2 reaches no space at all, which
    // no row keys; d3 joins a stair and the outside, which no row keys.
    let model = joins(plan(), "d1", "door", Some("k"), None);
    let model = joins(model, "d2", "door", None, None);
    let model = joins(model, "d3", "door", None, Some("s"));
    let model = width(width(width(model, "d1", 1.0), "d2", 1.0), "d3", 1.0);
    let evaluation = judge(model, width_keys(width_limits()));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "d1".into(),
                "clear width (Pset.ClearWidth) is 1 m; required at least 1.2 m (limit row 3: \
                 Pset.Use (via axioval:derived.adjacent-space) `corridor` and `exterior`)"
                    .into()
            ),
            (
                "d2".into(),
                "no limit defined for Pset.Use (via axioval:derived.adjacent-space) \
                 `exterior` and `exterior`"
                    .into()
            ),
            (
                "d3".into(),
                "no limit defined for Pset.Use (via axioval:derived.adjacent-space) \
                 `exterior` and `stair`"
                    .into()
            ),
        ]
    );
    // A row keyed `exterior` on both sides takes a door reaching no space.
    let model = joins(plan(), "d1", "door", Some("k"), Some("o1"));
    let model = joins(model, "d2", "door", None, None);
    let model = width(width(model, "d1", 1.2), "d2", 1.0);
    let evaluation = judge(
        model,
        width_keys(vec![row(Some("exterior"), Some("exterior"), 1.1)]),
    );
    assert_eq!(
        findings(&evaluation)
            .iter()
            .map(|(object, _)| object.as_str())
            .collect::<Vec<_>>(),
        ["d1", "d2"],
        "{:?}",
        unevaluated(&evaluation)
    );
}

#[test]
fn equally_specific_rows_apply_only_when_they_agree() {
    // office / * and corridor / * both apply to an office-corridor door.
    let model = || {
        let model = joins(plan(), "d1", "door", Some("o1"), Some("k"));
        width(model, "d1", 0.85)
    };
    let disagree = vec![
        row(Some("office"), None, 0.9),
        row(Some("corridor"), None, 1.0),
    ];
    let evaluation = judge(model(), width_keys(disagree));
    assert!(findings(&evaluation).is_empty());
    let outcomes = unevaluated(&evaluation);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].0, "d1");
    assert_eq!(outcomes[0].1, NotEvaluatedReason::InvalidDeclaration);
    assert!(
        outcomes[0].2.contains("limit rows 0, 1 apply equally"),
        "{}",
        outcomes[0].2
    );
    let agree = vec![
        row(Some("office"), None, 0.9),
        row(None, Some("corridor"), 0.9),
    ];
    let evaluation = judge(model(), width_keys(agree));
    assert_eq!(
        findings(&evaluation),
        [(
            "d1".into(),
            "clear width (Pset.ClearWidth) is 0.85 m; required at least 0.9 m (limit row 0: \
             Pset.Use (via axioval:derived.adjacent-space) `office` and `corridor`)"
                .into()
        )]
    );
}

#[test]
fn an_unknown_side_is_not_evaluated_unless_no_row_could_apply() {
    // d1 joins an office and a space stating no use; d2 joins an office and
    // a space stating the reserved key itself.
    let model = joins(plan(), "d1", "door", Some("o1"), Some("x"));
    let model =
        joins(model, "d2", "door", Some("o1"), Some("s")).text("s", "Pset", "Use", "Exterior");
    let model = width(width(model, "d1", 0.85), "d2", 0.85);
    let evaluation = judge(model, width_keys(width_limits()));
    assert!(findings(&evaluation).is_empty());
    let outcomes = unevaluated(&evaluation);
    assert_eq!(
        outcomes
            .iter()
            .map(|(object, reason, _)| (object.as_str(), reason.clone()))
            .collect::<Vec<_>>(),
        [
            ("d1", NotEvaluatedReason::IncompleteEvidence),
            ("d2", NotEvaluatedReason::IncompleteEvidence),
        ]
    );
    assert!(
        outcomes[1].2.contains("reserved key `exterior`"),
        "{}",
        outcomes[1].2
    );
    // A row only stair doors could take is ruled out by the known office.
    let model = joins(plan(), "d1", "door", Some("o1"), Some("x"));
    let model = width(model, "d1", 0.85);
    let evaluation = judge(
        model,
        width_keys(vec![row(Some("stair"), Some("stair"), 1.0)]),
    );
    assert_eq!(findings(&evaluation).len(), 1);
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_pair_key_declaration_is_checked() {
    let model = || {
        let model = joins(plan(), "d1", "door", Some("o1"), Some("k"));
        width(model, "d1", 0.85)
    };
    let mut stated_path = width_keys(width_limits());
    stated_path.retain(|(name, _)| *name != "key_1_path");
    stated_path.push(("key_1_path", strings(&["adjacent"])));
    let mut undeclared = width_keys(width_limits());
    undeclared.retain(|(name, _)| *name != "pair_key");
    undeclared.push(("pair_key", string("key_2")));
    let mut unknown = width_keys(width_limits());
    unknown.retain(|(name, _)| *name != "pair_key");
    unknown.push(("pair_key", string("use")));
    let mut other_side_alone = width_keys(width_limits());
    other_side_alone.retain(|(name, _)| *name != "pair_key");
    for parameters in [stated_path, undeclared, unknown, other_side_alone] {
        let evaluation = judge(model(), parameters);
        assert_eq!(
            unevaluated(&evaluation)
                .iter()
                .map(|(object, reason, _)| (object.as_str(), reason.clone()))
                .collect::<Vec<_>>(),
            [("-", NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// Bottom elevations per object; every body is 1 m tall.
#[derive(Default)]
struct Bottoms(BTreeMap<ObjectId, f64>);

impl Bottoms {
    fn with(mut self, local: &str, bottom: f64) -> Self {
        self.0.insert(id(local), bottom);
        self
    }
}

impl VerticalExtentService for Bottoms {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let bottom = self
            .0
            .get(object)
            .copied()
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::try_new(bottom, bottom)?,
            ElevationInterval::try_new(bottom + 1.0, bottom + 1.0)?,
            Evidence::exact(source(), format!("extent:{}", object.local_id)),
        )
    }
}

#[test]
fn a_window_between_differently_typed_spaces_takes_its_pair_row() {
    // w1 lies 1.2 m above the floors of an office and a corridor, w2 0.9 m;
    // their pair row allows at most 1 m, an office alone any sill.
    let model = joins(plan(), "w1", "window", Some("o1"), Some("k"));
    let model = joins(model, "w2", "window", Some("k"), Some("o1"));
    let mut sill_row = TableRow::new();
    sill_row.insert("key_1".into(), string("corridor"));
    sill_row.insert("other_side".into(), string("office"));
    sill_row.insert("maximum".into(), number(1.0));
    let mut office = TableRow::new();
    office.insert("key_1".into(), string("office"));
    let parameters = vec![
        (
            "limits",
            ParameterValue::Table {
                value: vec![office, sill_row],
            },
        ),
        ("quantity", string("sill-height")),
        ("floor_path", strings(&[ADJACENT])),
        ("key_1", property(Some("Pset"), "Use")),
        ("key_1_path", strings(&[ADJACENT])),
        ("pair_key", string("key_1")),
    ];
    let bottoms = Bottoms::default()
        .with("o1", 0.0)
        .with("k", 0.0)
        .with("w1", 1.2)
        .with("w2", 0.9);
    let evaluation = model.evaluate_with(
        &KeyedLimit,
        &rule(ID, kind("window"), parameters),
        |services| {
            services
                .register(VerticalExtentServiceHandle::new(Arc::new(bottoms)))
                .unwrap();
        },
    );
    let found = findings(&evaluation);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "w1");
    assert!(
        found[0].1.ends_with(
            "required at most 1 m (limit row 1: Pset.Use (via axioval:derived.adjacent-space) \
             `office` and `corridor`)"
        ),
        "{}",
        found[0].1
    );
    assert!(unevaluated(&evaluation).is_empty());
}
