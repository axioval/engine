//! Items of a measured member list judged one by one ([`Items`]): the
//! risers of a flight, the landings at a ramp's run ends, the rails along
//! a stretch, each a member a provider measured (or a search answered,
//! three-valued) for the object a rule checks.
//!
//! A form's check whose decision is [`Decision::Items`](super::Decision)
//! reads the list once per object, its `@` references bound as any
//! measured value's are, and judges every item with its [`ItemCheck`]s:
//! each failing test is its own finding or not-evaluated outcome on the
//! object (several findings per object, D1), or, [`Together`], one outcome
//! names every failing item. Item fields are read as the members state
//! them: a number interval, a truth, a text, objects, `null`, or undecided
//! with why. Nothing here measures: the provider measured, the template
//! judges the measurements against the rule's parameters.
//!
//! Messages render placeholders over the item: a text field or objects
//! field by name (objects joined `, `), a number field with a format
//! (`{rise:length}`, `{slope:ratio}`, `{angle:degrees}`, `{count:count}`,
//! `{area:area}`),
//! a parameter (`{landing_depth_minimum:length}`), a requirement's chosen
//! operand (`{stated:length}`), `{bound}` (the judgement's bounds as
//! declared, `0.15 m to 0.19 m`, `at least 0.26 m`), `{bound:plain}` (the
//! bound a range failed or straddled, its number as declared and without a
//! unit, as a range judge words it: `at least 4`), `{why}` (an undecided
//! field's reason), `{index}` and `{count}` (an item's place among the
//! items judged together), and the [`ItemText`]s by name.

use axioval_ir::contract::{AggregateFunction, AggregateSource, Expression, ExpressionComparison};
use serde::Serialize;

use super::{Applies, Operand};

/// Items of a measured member list, each judged by `checks`, or judged
/// `together` in one outcome.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Items {
    /// When the rule declares this check at all.
    pub applies: Applies,
    /// The measured member list, written as a measured value is: its name
    /// and arguments, `@name` the rule's parameter `name` and `@anchor`
    /// the object checked, bound for each object.
    pub list: &'static str,
    /// A list that cannot be measured leaves the check open with this
    /// message (`{why}` the list's refusal), or, `None`, reports nothing:
    /// another check reports why the object could not be measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<&'static str>,
    /// The checks of each item, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<ItemCheck>,
    /// One outcome over every item, instead of each item's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub together: Option<Together>,
    /// Every item passing, the check is still open where this holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passing: Option<Passing>,
    /// Named message parts, each rendered where its conditions hold; of
    /// several of one name the first that holds.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<ItemText>,
    /// Whether the findings of several items worded alike are one finding
    /// on the object, relating the objects each related (sorted, each
    /// once): one finding per distinct defect, not one per item.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub merged: bool,
    /// The objects field naming the object each item's outcomes are on (its
    /// first object), where not the object checked: a storey's region of
    /// floor judged in a check of the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<&'static str>,
    /// Whether the object is left open at most once: of this check's open
    /// outcomes only the first is reported, and none where an earlier
    /// check of the form already left the object open (a capability that
    /// reported one doubt per object).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub once: bool,
    /// Every item's outcome one outcome on the object ([`Combined`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combined: Option<Combined>,
}

/// The items' outcomes as one: where any item fails, one finding whose
/// message (`fail`) reads `{findings}`, the failing items' messages joined
/// by `separator`, graded by the worst, relating every object they relate;
/// otherwise, where any is open, one open outcome (`open`, `{opens}` their
/// messages joined), for the first one's reason; otherwise a pass. Its
/// messages read the first item's fields (`{row}`), which every item
/// shares. A capability judging a window against each floor beside it and
/// reporting every failing floor in one finding.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Combined {
    /// What joins the items' messages.
    pub separator: &'static str,
    /// The finding's message, `{findings}` the failing items' messages.
    pub fail: &'static str,
    /// The open outcome's message, `{opens}` the open items' messages.
    pub open: &'static str,
    /// The items as alternatives of what each group stands for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternatives: Option<Alternatives>,
}

/// Items as the alternatives of one unknown per group: the items of a
/// group (a text field) are the candidates for what it stands for (the
/// floor a door's side steps onto), those whose truth `sure` holds surely
/// among them. A group fails where a sure item fails, or where it has no
/// sure item and every item surely fails (each failing item's finding
/// standing); it passes where every item passes; otherwise it is open with
/// its items' open outcomes, or with `open` (worded over its first item)
/// where none is open.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alternatives {
    /// The text field naming an item's group.
    pub group: &'static str,
    /// The truth field holding where an item surely stands for its group.
    pub sure: &'static str,
    /// A group's open message where none of its items is open.
    pub open: &'static str,
}

/// A named message part of [`Items`].
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemText {
    pub name: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<When>,
    pub text: &'static str,
}

/// A condition on the rule or the item.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum When {
    /// The item's truth field is `value` (an undecided truth is neither).
    Field { field: &'static str, value: bool },
    /// The item's field is `null`.
    Null { field: &'static str },
    /// The item's field is not `null` (it may be undecided).
    Stated { field: &'static str },
    /// The rule states the parameter (a boolean, or its default, true).
    Declared { parameter: &'static str },
    /// The rule leaves the parameter unstated (a boolean false).
    Undeclared { parameter: &'static str },
    /// The rule's selector parameter leaves objects undecided: one of them
    /// may be what the item looked for.
    Undecided { selector: &'static str },
    /// The string parameter (or its default) is `value`.
    Equals {
        parameter: &'static str,
        value: &'static str,
    },
    /// The item's number field is exactly `value`.
    Is { field: &'static str, value: f64 },
    /// The item's number field is surely below `value`.
    Below { field: &'static str, value: f64 },
    /// The item's objects field names none.
    Empty { field: &'static str },
    /// The item's field is undecided: the measurement could not tell.
    Unknown { field: &'static str },
}

/// One check of an item: a test, or tests behind shared guards.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ItemCheck {
    /// A test, its own outcome.
    Test(Box<ItemTest>),
    /// Checks applying together: a guard that does not hold is one outcome
    /// for them all.
    Group(Box<Group>),
}

/// Checks behind guards.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    #[serde(skip_serializing_if = "Applies::is_empty")]
    pub applies: Applies,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<When>,
    /// Fields read first, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<Guard>,
    pub checks: Vec<ItemCheck>,
}

/// A field a group reads before its checks.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Guard {
    pub field: &'static str,
    /// Where the guard applies.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<When>,
    /// An undecided field leaves the item open with this message (`{why}`
    /// the field's reason), or with its reason alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undecided: Option<&'static str>,
    /// What a `null` field comes to.
    pub null: OnNull,
}

/// What a `null` field or value comes to.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum OnNull {
    /// Judged on: a test reading it decides what it means.
    Judge,
    /// Nothing more is judged of the item here.
    Skip,
    /// The item is open with the message.
    Open(&'static str),
    /// The item is a finding with the message.
    Fail(&'static str),
    /// The value does not meet a test reading it, however the test bounds
    /// it: the test fails, worded by its own message, its `otherwise` in
    /// its place (no barrier covers the edge, so it is not covered). A
    /// guard reading it judges on.
    Unmet,
}

/// One test of an item.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemTest {
    #[serde(skip_serializing_if = "Applies::is_empty")]
    pub applies: Applies,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<When>,
    pub judge: Judge,
    /// The finding's message.
    pub fail: &'static str,
    /// The message of a value straddling a bound.
    pub undecided: &'static str,
    /// Outcomes turned open where their conditions hold, in order; the
    /// first that applies.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// A test judged only where this one passes, in its place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub then: Option<Box<ItemTest>>,
    /// A test judged where this one fails, in its place where it applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otherwise: Option<Box<ItemTest>>,
    /// A test judged where this one is open, in its place where it
    /// applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub straddled: Option<Box<ItemTest>>,
    /// The objects field a finding relates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<&'static str>,
}

/// How a test judges an item.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Judge {
    /// A number within bounds.
    Range(Box<Range>),
    /// A truth: `finding` is a finding, the other value passes, an
    /// undecided truth leaves the item open with its reason.
    Truth { value: &'static str, finding: bool },
    /// Some row of a table parameter holds: each row bounds item numbers
    /// from above.
    Rows(Box<Rows>),
    /// A finding wherever the test applies: its conditions are the
    /// requirement (no handrail along a side where one is required).
    Fails,
}

/// A number of the item within bounds, each the largest (lower) or
/// smallest (upper) of its requirements, widened by the allowance.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub value: &'static str,
    pub unit: ItemUnit,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub at_least: Vec<Requirement>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub at_most: Vec<Requirement>,
    pub allowance: Allowance,
    /// Whether a finding states how far it misses the bound it fails.
    pub grade: bool,
    /// What a `null` value comes to. A finding (`OnNull::Fail`) stands only
    /// where a lower bound applies: a value that reaches nowhere misses
    /// only a least one.
    pub null: OnNull,
    /// How an undecided value leaves the item open (`{why}` its reason);
    /// without it, its reason alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmeasured: Option<&'static str>,
}

/// A bound: the first option whose conditions hold and whose operand is
/// known (a stated parameter, an item number).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    /// The placeholder naming the operand chosen.
    pub name: &'static str,
    pub options: Vec<Choice>,
    /// How `{requirements}` words it where chosen (`the {noun}'s width
    /// ({walking:length})`); the chosen ones are joined by ` and `.
    pub words: &'static str,
}

/// One option of a [`Requirement`].
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<When>,
    pub bound: Bound,
}

/// What a [`Choice`] bounds by.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Bound {
    /// A stated parameter, or an item's number.
    Operand(Operand),
    /// A fixed number in the value's unit: the rounding a rule never
    /// declares (a rail's top level within a micrometre).
    Literal(f64),
}

/// How a value is shown and what it measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemUnit {
    /// Metres, rounded to the micrometre: `0.3 m`.
    Length,
    /// Degrees of a plane angle in radians: `12.5°`.
    Degrees,
    /// A plain ratio to six decimals: `0.0833`.
    Ratio,
    /// An integer count: `12`.
    Count,
    /// Square metres as the area capabilities showed them, rounded to
    /// 1e-4: `26`, or `between 24 and 26` for an interval.
    Area,
}

/// The rounding allowance a bound is widened by.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Allowance {
    /// None: the bound as declared.
    None,
    /// `times` eight units in the last place of the magnitude (at least
    /// one): the binary rounding of decimal coordinates. Judged together,
    /// the largest magnitude of any item.
    Slack {
        times: f64,
        magnitude: super::Magnitude,
    },
    /// `times` the upper end of an item number the measurement states as
    /// its allowance. Judged together, the largest of any item.
    Stated { times: f64, value: &'static str },
    /// A fixed allowance in the value's unit.
    Fixed { value: f64 },
    /// A fixed allowance added to the value rather than to the bound: the
    /// value raised against a lower bound (`value + allowance ≥ bound`) and
    /// lowered against an upper one, as capabilities widening a measured
    /// length by its rounding compared it. Only a test's range reads it.
    Raised { value: f64 },
}

/// An outcome turned open, worded by `message`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<When>,
    pub on: On,
    pub message: &'static str,
}

/// Which outcome an [`Effect`] turns open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum On {
    /// A pass.
    Pass,
    /// Any finding.
    Fail,
    /// A finding of a value below its lower bound.
    FailBelow,
    /// A `null` value judged a pass or skipped.
    Null,
}

/// Rows of a table parameter bounding item numbers from above: an item
/// passes where some row holds, fails where every row fails (graded by
/// the row it misses least, none where a row fails only within its
/// allowance), and is open otherwise. No row judges nothing.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rows {
    pub table: &'static str,
    /// The columns, the first required: each bounds a number of the item.
    pub columns: Vec<RowColumn>,
    /// How rows are joined in `{rows}`: `; or `.
    pub joiner: &'static str,
}

/// One column of [`Rows`]: the item number it bounds, and how the row
/// words it (`over at most {}`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowColumn {
    pub column: &'static str,
    pub value: &'static str,
    pub unit: ItemUnit,
    pub allowance: Allowance,
    pub words: &'static str,
}

/// Every item judged in one outcome.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Together {
    /// Only items whose field is not `null` are judged, numbered among
    /// themselves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub present: Option<&'static str>,
    /// Only items where these hold are judged (a flight's winders only
    /// where it turns).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<When>,
    /// How an item is named: `riser {index} of {count}`.
    pub name: &'static str,
    pub judge: TogetherJudge,
}

/// How [`Together`] judges its items.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TogetherJudge {
    /// Each item's number within the range; one finding names every item
    /// failing (`{failing}`, each `item`, joined `, `), graded by the worst.
    Every(Box<Every>),
    /// Each item's truth: `finding` fails (`{failing}`), undecided ones
    /// are open together.
    Truths(Box<Truths>),
    /// The spread of the items' numbers (the largest less the smallest)
    /// at most a tolerance.
    Spread(Box<Spread>),
    /// How many items there are, within bounds (`{count}`), never open.
    Count(Box<Count>),
    /// The least of the items' numbers at least a bound: the least lies
    /// between the least lower end and the least upper end, at the item of
    /// least upper end; an item not measured can only lower it.
    Least(Box<Least>),
    /// At least one item meets a requirement: an item holding passes,
    /// otherwise the first item leaving it open does, otherwise the items
    /// surely failing it are one finding.
    Any(Box<Any>),
}

/// [`TogetherJudge::Any`]: one item holding is enough. Where none holds,
/// the first item (in item order) matching one of `open` leaves the check
/// open, worded by the first case it matches; where none is open either,
/// the items matching `fails` are one finding (`{failing}`, each `item`,
/// joined `, `), relating each one's `related` objects; and where none
/// fails, it passes.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Any {
    /// What an item meeting the requirement holds.
    pub holds: Vec<When>,
    /// What an item surely failing it holds.
    pub fails: Vec<When>,
    /// What leaves an item open, in order: its conditions, and how the
    /// check is worded open (`{why}` the field `why`'s reason).
    pub open: Vec<OpenCase>,
    /// How a failing item is named: `{space}`.
    pub item: &'static str,
    /// The finding.
    pub fail: &'static str,
    /// The objects field each failing item relates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<&'static str>,
}

/// One way an item leaves an [`Any`] open.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenCase {
    pub when: Vec<When>,
    /// The undecided field whose reason is `{why}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<&'static str>,
    pub message: &'static str,
}

/// [`TogetherJudge::Count`].
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Count {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub at_least: Vec<Requirement>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub at_most: Vec<Requirement>,
    /// The finding: `{count}` the items, `{bound}` the bounds as counts.
    pub fail: &'static str,
}

/// [`TogetherJudge::Least`]. Messages read `{least}` (the least as an
/// interval), `{at}` (the narrowest item, worded by `at`), `{unknown}`
/// (why items are not measured, joined `; `) and `{some}` (`a width` or
/// `widths`, by how many are not measured).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Least {
    pub value: &'static str,
    pub unit: ItemUnit,
    pub at_least: Requirement,
    /// The allowance, times eight units in the last place of the least's
    /// upper end (at least one).
    pub times: f64,
    /// How the narrowest item is named.
    pub at: &'static str,
    /// The objects fields a finding relates, of the narrowest item.
    pub related: Vec<&'static str>,
    pub fail: &'static str,
    pub undecided: &'static str,
    /// Every measured item passing, the check is open with this where its
    /// conditions hold (an undecided obstacle may narrow it).
    pub pending: Effect,
    /// Every measured item passing but some not measured.
    pub partial: &'static str,
    /// No item measured.
    pub unmeasured: &'static str,
    /// Further reasons items may be missing, a text field of the first item
    /// (`;`-separated), counted with the unmeasured ones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<&'static str>,
}

/// [`TogetherJudge::Every`].
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Every {
    pub range: Range,
    /// A number surely within this of zero is no item, and one possibly
    /// that close is open (`straddling`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zero: Option<f64>,
    /// An item as a failing or straddling one is named: `{name} is
    /// {riser:length}`.
    pub item: &'static str,
    /// The finding: `{failing}; {bound} required`.
    pub fail: &'static str,
    /// How open items are worded.
    pub open: OpenItems,
    /// Any item whose number is undecided leaves the check open with this
    /// message instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmeasured_any: Option<&'static str>,
}

/// How [`Every`] words its open items.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenItems {
    /// Straddling items together (`straddling`, `{items}` their `item`
    /// joined `, `), then unmeasured ones (`unmeasured`, `{items}` their
    /// names joined `, `), the parts joined `; `.
    Grouped {
        straddling: &'static str,
        unmeasured: &'static str,
    },
    /// Each open item on its own, in item order, joined `; `: a
    /// straddling one as `straddling`, an unmeasured one as `unmeasured`.
    Each {
        straddling: &'static str,
        unmeasured: &'static str,
    },
}

/// [`TogetherJudge::Truths`].
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Truths {
    pub value: &'static str,
    pub finding: bool,
    /// A failing item: `{name} is open`.
    pub item: &'static str,
    /// The finding: `{failing}; closed risers required`.
    pub fail: &'static str,
    /// Undecided items, `{items}` their names joined by `joiner`.
    pub undecided: &'static str,
    pub joiner: &'static str,
}

/// [`TogetherJudge::Spread`].
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spread {
    pub value: &'static str,
    pub unit: ItemUnit,
    pub tolerance: Operand,
    pub allowance: Allowance,
    /// The finding: `{spread}` the spread, `{values}` every item's number
    /// (`a..b` for an interval), `{tolerance}` the tolerance.
    pub fail: &'static str,
    pub undecided: &'static str,
}

/// Every item passing, the check is open where `when` holds: once, or,
/// with `groups`, for each item of another list whose items (of `key`
/// alike) all pass, worded over that item.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Passing {
    pub when: Vec<When>,
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Groups>,
}

/// The items of another list, each grouping the judged items whose number
/// field `key` is its own; one whose `key` is not a number groups none and
/// is never open.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Groups {
    pub list: &'static str,
    pub key: &'static str,
}

impl Applies {
    /// Whether it states no condition.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.when.is_empty() && self.any.is_empty() && self.condition.is_none()
    }
}

/// Every test of `checks`, those of groups included, in order.
fn collect<'a>(checks: &'a [ItemCheck], tests: &mut Vec<&'a ItemTest>) {
    for check in checks {
        match check {
            ItemCheck::Test(test) => tests.push(test),
            ItemCheck::Group(group) => collect(&group.checks, tests),
        }
    }
}

impl Items {
    /// The check as a block editor shows it: every item of the list
    /// satisfying each test's bounds (a truth test, its truth), as one
    /// aggregate. It states what the template judges, not how it words or
    /// grades it, and a rule is never forked from it.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn expression(&self) -> Expression {
        let field = |name: &str| Expression::Property {
            property_set: Some(axioval_ir::MEMBER_SET.to_owned()),
            property: name.to_owned(),
            of: None,
            label: None,
        };
        let parameter = |operand: &Operand| match operand {
            Operand::Parameter(name) => Expression::Parameter {
                name: (*name).to_owned(),
                label: None,
            },
            Operand::Value(name) => field(name),
        };
        let compare = |operator, left: Expression, right: Expression| Expression::Compare {
            operator,
            left: Box::new(left),
            right: Box::new(right),
            case_sensitive: true,
            label: None,
        };
        let mut tests: Vec<&ItemTest> = Vec::new();
        collect(&self.checks, &mut tests);
        let bound = |bound: &Bound| match bound {
            Bound::Operand(operand) => parameter(operand),
            Bound::Literal(value) => Expression::Literal {
                value: axioval_ir::contract::ScalarValue::Number { value: *value },
                label: None,
            },
        };
        let range = |range: &Range| -> Vec<Expression> {
            let mut operands = Vec::new();
            for requirement in &range.at_least {
                if let Some(choice) = requirement.options.first() {
                    operands.push(compare(
                        ExpressionComparison::GreaterThanOrEquals,
                        field(range.value),
                        bound(&choice.bound),
                    ));
                }
            }
            for requirement in &range.at_most {
                if let Some(choice) = requirement.options.first() {
                    operands.push(compare(
                        ExpressionComparison::LessThanOrEquals,
                        field(range.value),
                        bound(&choice.bound),
                    ));
                }
            }
            operands
        };
        let truth = |value: &str, finding: bool| {
            if finding {
                Expression::Not {
                    operand: Box::new(field(value)),
                    label: None,
                }
            } else {
                field(value)
            }
        };
        let mut operands = Vec::new();
        for test in tests {
            match &test.judge {
                Judge::Range(bounds) => operands.extend(range(bounds)),
                Judge::Truth { value, finding } => operands.push(truth(value, *finding)),
                Judge::Fails => {}
                Judge::Rows(rows) => {
                    for column in &rows.columns {
                        operands.push(compare(
                            ExpressionComparison::LessThanOrEquals,
                            field(column.value),
                            Expression::Parameter {
                                name: format!("{}.{}", rows.table, column.column),
                                label: None,
                            },
                        ));
                    }
                }
            }
        }
        if let Some(together) = &self.together {
            match &together.judge {
                TogetherJudge::Every(every) => operands.extend(range(&every.range)),
                TogetherJudge::Truths(truths) => {
                    operands.push(truth(truths.value, truths.finding));
                }
                TogetherJudge::Count(_) | TogetherJudge::Least(_) | TogetherJudge::Any(_) => {}
                TogetherJudge::Spread(spread) => operands.push(compare(
                    ExpressionComparison::LessThanOrEquals,
                    Expression::Subtract {
                        left: Box::new(Expression::Aggregate {
                            function: AggregateFunction::Max,
                            over: AggregateSource::Measured {
                                name: self.list.to_owned(),
                            },
                            filter: None,
                            value: Some(Box::new(field(spread.value))),
                            label: None,
                        }),
                        right: Box::new(Expression::Aggregate {
                            function: AggregateFunction::Min,
                            over: AggregateSource::Measured {
                                name: self.list.to_owned(),
                            },
                            filter: None,
                            value: Some(Box::new(field(spread.value))),
                            label: None,
                        }),
                        label: Some("spread".into()),
                    },
                    parameter(&spread.tolerance),
                )),
            }
        }
        if operands.is_empty() {
            operands.push(Expression::Literal {
                value: axioval_ir::contract::ScalarValue::Boolean { value: true },
                label: None,
            });
        }
        Expression::Aggregate {
            function: AggregateFunction::None,
            over: AggregateSource::Measured {
                name: self.list.to_owned(),
            },
            filter: None,
            value: Some(Box::new(Expression::Not {
                operand: Box::new(Expression::And {
                    operands,
                    label: None,
                }),
                label: None,
            })),
            label: Some("no item fails".into()),
        }
    }
}
