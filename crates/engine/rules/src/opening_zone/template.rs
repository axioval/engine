//! `opening-zone` as a template, over the measured list `zone_checks`: each
//! placement of an opening in a host its path reaches within the host and
//! clear of its ends and edges, inside one of the allowed zones, each row
//! of the dimensioning table, each requirement on the host's supports, and
//! its least distance to another opening of the host, in the capability's
//! order.

use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Condition, Decision, Form, FormCheck, ItemCheck,
    ItemTest, ItemUnit, Items, Judge, OnNull, Operand, Range, Refusals, Requirement, Template,
    TemplateValue,
};
use serde_json::json;

use super::ROUNDING;

/// The list's arguments: the rule's declaration, and the openings it
/// selects.
const CHECKS: &str = concat!(
    "zone_checks",
    ";host_path=@host_path;host_selector=@host_selector;length_axis=@length_axis;\
             height_axis=@height_axis;end_distance=@end_distance;edge_distance=@edge_distance;\
             edge_distance_maximum=@edge_distance_maximum;maximum_edges=@maximum_edges;\
             zone=@zone;opening_spacing=@opening_spacing;zones=@zones;\
             minimum_opening_area=@minimum_opening_area;dimensions=@dimensions;\
             support_path=@support_path;support_selector=@support_selector;\
             support_gap=@support_gap;support_distance=@support_distance;\
             support_distance_ratio=@support_distance_ratio;\
             support_distance_reference=@support_distance_reference;\
             support_clearance=@support_clearance;openings=@selection"
);

/// The edges are judged where the rule bounds the distance from them, or
/// keeps the opening between the flanges.
const EDGES_JUDGED: Condition = Condition::Not {
    condition: &Condition::All {
        conditions: &[
            Condition::Not {
                condition: &Condition::Stated {
                    parameter: "edge_distance",
                },
            },
            Condition::Not {
                condition: &Condition::Equals {
                    parameter: "zone",
                    value: "web",
                },
            },
        ],
    },
};

/// A test of nothing more than the item's truth `field`, open with its
/// reason where it is undecided.
fn truth(field: &'static str, finding: bool, fail: &'static str) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge: Judge::Truth {
            value: field,
            finding,
        },
        fail,
        undecided: "{why}",
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("related"),
    }
}

/// A bound: the parameter `parameter`, or the item's number `value`.
fn bound(name: &'static str, bound: Bound) -> Vec<Requirement> {
    vec![Requirement {
        name,
        options: vec![Choice {
            when: Vec::new(),
            bound,
        }],
        words: "",
    }]
}

/// A graded range of the item's length `value`, within the capability's
/// rounding allowance.
#[allow(clippy::too_many_arguments)]
fn range(
    value: &'static str,
    at_least: Vec<Requirement>,
    at_most: Vec<Requirement>,
    allowance: Allowance,
    fail: &'static str,
    undecided: &'static str,
) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge: Judge::Range(Box::new(Range {
            value,
            unit: ItemUnit::Length,
            at_least,
            at_most,
            allowance,
            grade: true,
            null: OnNull::Skip,
            unmeasured: Some("{why}"),
        })),
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("related"),
    }
}

/// The rounding allowance every distance is judged within.
const ROUNDED: Allowance = Allowance::Fixed { value: ROUNDING };

/// The largest distance from one of the far edges: a finding where it is
/// surely farther, its doubt reported with the other edge's (`far_open`).
fn far(value: &'static str, fail: &'static str) -> ItemTest {
    let mut test = range(
        value,
        Vec::new(),
        bound(
            "maximum",
            Bound::Operand(Operand::Parameter("edge_distance_maximum")),
        ),
        ROUNDED,
        fail,
        "",
    );
    test.applies.when = &["edge_distance_maximum"];
    test.straddled = Some(Box::new(truth("placed", false, "")));
    test
}

/// The checks: each placement within the host, clear of its ends and
/// edges and near enough its far edges; inside an allowed zone; each row of
/// the dimensioning table; each requirement on the supports; and clear of
/// the other openings of the host.
fn checks() -> Vec<ItemCheck> {
    let mut edges = range(
        "edge",
        vec![Requirement {
            name: "required",
            options: vec![
                Choice {
                    when: Vec::new(),
                    bound: Bound::Operand(Operand::Parameter("edge_distance")),
                },
                Choice {
                    when: Vec::new(),
                    bound: Bound::Literal(0.0),
                },
            ],
            words: "",
        }],
        Vec::new(),
        ROUNDED,
        "opening {edge_words} of its host {host}; {required} clear required",
        "its distance from an edge of its host {host}'s outline may be under {required}: \
         where it lies in the host's section is known only within bounds",
    );
    edges.applies.condition = Some(EDGES_JUDGED);
    let mut ends = range(
        "end",
        bound(
            "required",
            Bound::Operand(Operand::Parameter("end_distance")),
        ),
        Vec::new(),
        ROUNDED,
        "opening is {end_shown} from an end of its host {host}; {end_distance:length} required",
        "its distance from an end of its host {host}'s outline may be under \
         {end_distance:length}: where it lies in the host's section is known only within \
         bounds",
    );
    ends.applies.when = &["end_distance"];
    let mut far_open = truth("far_open", true, "");
    far_open.applies.when = &["edge_distance_maximum"];
    let mut spacing = range(
        "spacing",
        bound(
            "required",
            Bound::Operand(Operand::Parameter("opening_spacing")),
        ),
        Vec::new(),
        ROUNDED,
        "opening is {spacing} clear of another opening in its host {host}; \
         {opening_spacing:length} required",
        "",
    );
    spacing.applies.when = &["opening_spacing"];
    let zone = range(
        "zone",
        bound("needed", Bound::Operand(Operand::Value("needed"))),
        Vec::new(),
        ROUNDED,
        "{words}",
        "{words}",
    );
    let dimension = range(
        "distance",
        bound("minimum", Bound::Operand(Operand::Value("minimum"))),
        bound("maximum", Bound::Operand(Operand::Value("maximum"))),
        Allowance::Stated {
            times: 1.0,
            value: "slack",
        },
        "{words}; {required} ({label})",
        "{open}",
    );
    [
        truth("placed", false, ""),
        truth(
            "inside",
            false,
            "opening lies partly outside its host {host}: {outside}",
        ),
        ends,
        edges,
        far(
            "bottom",
            "opening is {bottom_shown} from {bottom_name} of its host {host}; at most \
             {edge_distance_maximum:length} allowed",
        ),
        far(
            "top",
            "opening is {top_shown} from {top_name} of its host {host}; at most \
             {edge_distance_maximum:length} allowed",
        ),
        far_open,
        zone,
        dimension,
        truth("fails", true, "{message}"),
        spacing,
    ]
    .into_iter()
    .map(|test| ItemCheck::Test(Box::new(test)))
    .collect()
}

/// The checks over the items of the list, open with its refusal where the
/// opening's hosts cannot be told, each item's open outcomes for the
/// reason it states.
fn items() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: CHECKS,
            refused: Some("{why}"),
            checks: checks(),
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            at: None,
            once: false,
            combined: None,
            reason: Some("reason"),
        })),
        fail: "",
        undecided: "",
        related: None,
        grading: None,
        applies: None,
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// `opening-zone`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: super::ID,
        parameters: super::parameters(),
        grades: true,
        name: "opening-zone",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        // The declaration as the capability read it, in its order and
        // words.
        declaration: vec![Check::Arguments {
            when: &[],
            value: CHECKS,
        }],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            // The opening is judged by its checks.
            values: vec![TemplateValue {
                name: "judged",
                expression: serde_json::from_value(
                    json!({"kind": "literal", "value": {"type": "boolean", "value": true}}),
                )
                .expect("a literal"),
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            }],
            decision: Decision::Holds { value: "judged" },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: vec![items()],
            unless: Vec::new(),
            grading: None,
            once: Vec::new(),
            project: Vec::new(),
            joined: None,
        }],
    }
}
