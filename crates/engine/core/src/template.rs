//! Built-in capabilities as preconfigured compositions ("templates").
//!
//! A template keeps a capability's outside contract (its id, parameter
//! descriptor, defaults, finding wording and three-valued outcomes) and
//! states, as data, how a rule bound to it is decided from the shared
//! parts: measured and stated values read by expressions the engine's one
//! evaluator evaluates, and a decision over them. Packages never supply a
//! template; templates are engine-owned and built in, and a rule bound to
//! one is still exported as that capability with its parameters.
//!
//! A template is [`Template`]: its descriptor, declaration [`Check`]s run
//! once per rule, the host [`Service`]s it needs, named message
//! [`Text`]s, and one or more [`Form`]s. The first form whose `when`
//! parameters are all stated applies; it lists its [`TemplateValue`]s (each an
//! expression, read in order, the first that cannot be read leaving the
//! object open) and its [`Decision`]. Messages are written with
//! placeholders: a [`Text`]'s name, a parameter's name (its value as
//! stated: a string, a property reference as `set.name`), a value's name
//! with a format (`{extent:length}`: `0.3 m` or `between 0.48 m and
//! 0.52 m`; `{target:stated}`: a stated property's value as the source
//! states it), `{bound}` (the bound a [`Decision::Within`] failed or
//! straddled, `at least 0.26 m`) and `{why}` (the refusal of a value that
//! cannot be read). The runner, in the rules crate, binds a rule's
//! parameters into the expressions as constants (slots `{parameter}`,
//! `{parameter.set}` and `{parameter.name}` in a string field, and
//! `parameter` reads), evaluates the values for each selected object with
//! the shared evaluator, decides and words the outcome.
//!
//! [`Form::requirement`] states a form as one expression, the decision's
//! expression form with every value inlined: what the catalogue shows a
//! block editor, and what a rule forked from the template starts from.

use axioval_ir::contract::{Expression, ExpressionComparison, ScalarValue};
use serde::Serialize;

use crate::ParameterDescriptor;

/// Four units in the last place of the largest magnitude a comparison
/// involves: decimal coordinates and lengths read in binary differ from
/// what was meant by that much, and no more. A [`Decision::Within`]
/// widens its bounds by it.
pub const ROUNDING_ULPS: f64 = 4.0 * f64::EPSILON;

/// A built-in capability as a composition of shared parts.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    /// The capability id rules bind to.
    #[serde(skip)]
    pub id: &'static str,
    /// The capability's parameter descriptor, unchanged by the template.
    #[serde(skip)]
    pub parameters: Vec<ParameterDescriptor>,
    /// Whether a finding states how far its value misses the bound it
    /// fails (the descriptor's `grades_deviation`), measured from the
    /// declared bound, never the widened one.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub grades: bool,
    /// How messages about the rule as a whole name the capability, such as
    /// `body-extent` in the message ``body-extent: `minimum` exceeds `maximum` ``.
    pub name: &'static str,
    /// Where a declaration the template refuses, or a host missing its
    /// services, is reported: the rule as a whole, or each selected object.
    #[serde(skip_serializing_if = "Refusals::is_rule")]
    pub refusals: Refusals,
    /// Values an optional parameter takes when a rule leaves it unstated,
    /// applied after the declaration checks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub defaults: Vec<ParameterDefault>,
    /// What a rule's parameters must satisfy, in order; the first that
    /// fails leaves the rule not evaluated as an invalid declaration.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub declaration: Vec<Check>,
    /// The host services every value needs. Without any of them the rule
    /// as a whole is left not evaluated (`MissingService`), before any
    /// object is selected, as the capability always did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<Services>,
    /// Named message parts, which placeholders name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<Text>,
    /// The compositions, the first whose `when` parameters are all stated
    /// applying.
    pub forms: Vec<Form>,
}

/// Where a template reports a declaration it refuses and a host missing
/// its services.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Refusals {
    /// Once, for the rule as a whole, before anything is selected, worded
    /// after the template's name (`body-extent: …`).
    #[default]
    Rule,
    /// For each selected object, after the selection, worded as the check
    /// states it: as capabilities that judged the declaration per object
    /// reported it.
    Objects,
}

impl Refusals {
    /// Whether refusals are the rule's.
    #[must_use]
    pub fn is_rule(&self) -> bool {
        *self == Self::Rule
    }
}

/// A value an optional parameter takes when unstated.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterDefault {
    pub parameter: &'static str,
    pub value: ScalarValue,
}

/// One check of a rule's parameters. Each names the parameters it reads;
/// its message is the rule's not-evaluated message after `<name>: `.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Check {
    /// A string parameter is one of `options`: otherwise
    /// `` <parameter> `<value>` is unsupported; use `a`, `b` or `c` ``.
    Choice {
        parameter: &'static str,
        options: &'static [&'static str],
    },
    /// A quantity parameter, where stated, is a length of at least zero:
    /// otherwise `` `<parameter>` is negative `` or
    /// `` `<parameter>` is not a length ``.
    Length { parameter: &'static str },
    /// An integer parameter is stated where the descriptor requires it,
    /// and is at least zero: otherwise `` parameter `<parameter>` is
    /// required `` or `` `<parameter>` is negative ``.
    Count { parameter: &'static str },
    /// The parameter, where stated, is of its descriptor's kind, refused
    /// as the parameter reader words a wrong one: a check placed where the
    /// capability read the parameter, so refusals keep their order.
    Kind { parameter: &'static str },
    /// Every number of `parameters` that is stated is at least zero: each
    /// is read first (a wrong type refused as the parameter reader words
    /// it), then any below zero fails with `message`.
    NonNegative {
        parameters: &'static [&'static str],
        message: &'static str,
    },
    /// The traversal the rule declares (`relationship`, `direction`,
    /// `follow_chain`, `path`, `skip_absent_relationship_ends`) is valid,
    /// as the traversal reader words a refusal, and is declared only
    /// together with one of `with`.
    Traversal {
        with: &'static [&'static str],
        message: &'static str,
    },
    /// No parameter of `one` is stated together with one of `other`.
    Exclusive {
        one: &'static [&'static str],
        other: &'static [&'static str],
        message: &'static str,
    },
    /// At least one of `parameters` is stated.
    AnyOf {
        parameters: &'static [&'static str],
        message: &'static str,
    },
    /// `parameter` is stated only together with one of `with`.
    Requires {
        parameter: &'static str,
        with: &'static [&'static str],
        message: &'static str,
    },
    /// Where both are stated, `low` is at most `high`.
    Ordered {
        low: &'static str,
        high: &'static str,
        message: &'static str,
    },
    /// Every one of `parameters` is stated as a finite `number` (no other
    /// kind of value), above `above` and at least `at_least` where given:
    /// otherwise `message`.
    Finite {
        parameters: &'static [&'static str],
        #[serde(skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        at_least: Option<f64>,
        message: &'static str,
    },
    /// Every one of `parameters` stated as a `number` is at most `value`:
    /// otherwise `message` (a share no greater than the whole).
    AtMost {
        parameters: &'static [&'static str],
        value: f64,
        message: &'static str,
    },
    /// Both stated as numbers, `low` is below `high`: otherwise `message`.
    Increasing {
        low: &'static str,
        high: &'static str,
        message: &'static str,
    },
    /// A string-list parameter, where stated, lists at least one valid
    /// discipline: otherwise `` `<parameter>` is empty `` or the
    /// discipline's refusal.
    Disciplines { parameter: &'static str },
    /// A string-list parameter, where stated, is a valid relationship path
    /// (the step grammar every path shares), refused as the path reader
    /// words it.
    Path { parameter: &'static str },
    /// The rule's tolerance parameters (`tolerance`, `relative_tolerance`,
    /// `decimals`) are valid together, refused as the tolerance reader
    /// words it.
    Tolerance,
    /// The parameter is stated, and of its descriptor's kind: otherwise
    /// `` parameter `<parameter>` is required `` or the reader's refusal.
    Required { parameter: &'static str },
    /// A quantity parameter, where stated, is a length of at least zero:
    /// otherwise `message` (a wrong type refused as the reader words it).
    NonNegativeLength {
        parameter: &'static str,
        message: &'static str,
    },
    /// The parameters are stated all together or not at all.
    Together {
        parameters: &'static [&'static str],
        message: &'static str,
    },
    /// A string parameter, where stated, is one of `options`: otherwise
    /// `message`, `{value}` the stated string.
    Among {
        parameter: &'static str,
        options: &'static [&'static str],
        message: &'static str,
    },
    /// At least one of `parameters` is declared: stated and, for a
    /// boolean, true.
    Declares {
        parameters: &'static [&'static str],
        message: &'static str,
    },
    /// Where the boolean `flag` is stated false, one of `with` is stated.
    FalseRequires {
        flag: &'static str,
        with: &'static [&'static str],
        message: &'static str,
    },
    /// Where `when` is stated, no string parameter of `parameters` states
    /// `value`: a mode that does not combine with one of their options.
    Excludes {
        when: &'static str,
        parameters: &'static [&'static str],
        value: &'static str,
        message: &'static str,
    },
    /// Where every parameter of `when` is stated, the rule parameters the
    /// measured value `value` names (`@name`) are checked as stated, under
    /// the value's keys, by the value's own argument check (one its
    /// provider declares, `measured_kinds::argument_check`): a declaration
    /// only the measurement knows how to read, such as a table of rows,
    /// refused once for the rule in the measurement's words.
    Arguments {
        when: &'static [&'static str],
        value: &'static str,
    },
}

/// The host services a template's values need, and the message leaving
/// the rule open without them.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Services {
    pub needs: Vec<Service>,
    pub message: &'static str,
}

/// A host service a template may need, by the name the measured-value
/// registry lists it under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Service {
    ObjectFrame,
    VerticalExtent,
    TriangleCount,
}

impl Service {
    /// Whether `services` registers it.
    #[must_use]
    pub fn registered(self, services: &crate::ServiceRegistry) -> bool {
        match self {
            Self::ObjectFrame => services.get::<crate::ObjectFrameServiceHandle>().is_some(),
            Self::VerticalExtent => services
                .get::<crate::VerticalExtentServiceHandle>()
                .is_some(),
            Self::TriangleCount => services
                .get::<crate::TriangleCountServiceHandle>()
                .is_some(),
        }
    }
}

/// A named message part. `{name}` in a message renders it, or nothing
/// where its condition does not hold.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Text {
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    pub text: &'static str,
}

/// When a [`Text`] is rendered.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Condition {
    /// The parameter (or its default) is a number above zero.
    Positive { parameter: &'static str },
    /// The value was read from evidence that is not exact: a count of a
    /// tessellation, a measurement within a chord deviation.
    Inexact { value: &'static str },
    /// The string parameter (or its default) is `value`. Several texts of
    /// one name may each hold under another condition; the first that
    /// holds is rendered.
    Equals {
        parameter: &'static str,
        value: &'static str,
    },
    /// The value's lower end is zero: nothing surely counted.
    Zero { value: &'static str },
    /// The string parameter (or its default) is one of `values`.
    OneOf {
        parameter: &'static str,
        values: &'static [&'static str],
    },
    /// Every one of `conditions` holds.
    All { conditions: &'static [Condition] },
    /// `condition` does not hold.
    Not { condition: &'static Condition },
}

/// One composition of a template.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Form {
    /// The parameters whose statement selects this form.
    pub when: &'static [&'static str],
    /// The values read for each selected object, in order.
    pub values: Vec<TemplateValue>,
    /// How the values decide.
    pub decision: Decision,
    /// The finding's message where the decision fails.
    pub fail: &'static str,
    /// The not-evaluated message where the values cannot decide.
    pub undecided: &'static str,
    /// The members each selected object (an anchor) is judged through,
    /// where the form judges anchors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Members>,
    /// The report table the form fills with what it read, passing or not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<Table>,
    /// Where the form decides, when not for each selected object: each
    /// source, or the project as a whole, over the objects the rule
    /// selects there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scopes>,
    /// Values derived from the values read, after them and before the
    /// decision, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub derived: Vec<Derived>,
    /// The value whose measured reads' cited objects ([`Citation`]) a
    /// finding relates (the doors a shelf length was measured with), or
    /// `members:<selector>`, the members one population surely picked;
    /// none relates the decided members of every population, if any.
    ///
    /// [`Citation`]: crate::Citation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<&'static str>,
    /// Further decisions, each judged on its own once `values` are read
    /// and before the form's own decision, each its own finding or
    /// not-evaluated outcome: a capability reporting one per failed check.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<FormCheck>,
}

/// A further decision of a [`Form`]: its own values, read after the form's
/// (one that cannot be read leaves only this check open, worded as it was
/// refused), its decision over them and the form's, and its messages.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormCheck {
    /// The values read for this check, in order.
    pub values: Vec<TemplateValue>,
    /// How it decides.
    pub decision: Decision,
    /// The finding's message where it fails.
    pub fail: &'static str,
    /// The not-evaluated message where it cannot decide.
    pub undecided: &'static str,
    /// The value whose measured reads' cited objects its finding relates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<&'static str>,
}

/// A value derived from values already read, in plain binary arithmetic
/// over their intervals, as the capabilities computed it.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Derived {
    /// `minuend − subtrahend`.
    Difference(Difference),
    /// `numerator / denominator` of two values of at least zero (areas,
    /// counts). A denominator that may be zero gives a ratio without an
    /// upper bound (infinity) where the evaluator's division refuses; one
    /// that is surely zero (no upper end above zero) leaves the object
    /// open with `zero`.
    Ratio {
        name: &'static str,
        numerator: &'static str,
        denominator: &'static str,
        zero: &'static str,
    },
}

/// A form deciding once per source of the session (an empty source
/// included), or once for the whole project, over the objects the rule
/// selects in it: an existence or cardinality check an object rule cannot
/// make, since an object rule over an empty selection reports nothing.
///
/// A value reads the scope's objects as an aggregate over
/// [`Scopes::source`]: the objects surely selected are its members, those
/// whose selection is undecided possible members (a count widens over
/// them). A finding is scoped to the source or the project, relating the
/// objects surely selected and citing what selected them; a scope the
/// possible members leave undecided is not evaluated, each of them too,
/// for its own reason. Messages name the scope as `{place}` and the
/// possible members' count as `{undecided}`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scopes {
    /// The boolean parameter that, true, makes the project one scope;
    /// each source is one otherwise.
    pub across: &'static str,
    /// The string-list parameter naming the disciplines whose sources
    /// count, where the template takes one. A source of another
    /// discipline is left out; one declaring none is not evaluated per
    /// source (`undeclared`) and its selected objects are possible members
    /// of the project (`undeclared_member`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disciplines: Option<&'static str>,
    /// The scope's messages.
    pub messages: ScopeMessages,
}

/// The messages of a [`Scopes`] form. `{source}` names a source,
/// `{disciplines}` the declared disciplines (`` `mep` or `hvac` ``),
/// `{why}` a refusal.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeMessages {
    /// `{place}` for a source scope (``in source `{source}` ``).
    pub source: &'static str,
    /// `{place}` for the project.
    pub project: &'static str,
    /// The rule left open where there is no source to judge.
    pub no_source: &'static str,
    /// The rule left open where no source plays a declared discipline.
    pub no_discipline: &'static str,
    /// A source declaring no discipline, judged per source.
    pub undeclared: &'static str,
    /// A selected object of a source declaring no discipline, judged
    /// across sources.
    pub undeclared_member: &'static str,
    /// A source whose resource objects cannot be listed.
    pub unlisted: &'static str,
    /// The rule left open where disciplines are declared and the run
    /// states none.
    pub no_disciplines: &'static str,
}

impl Scopes {
    /// The aggregate source a value reads a scope's objects through, as a
    /// block editor shows it: the objects the rule selects there.
    #[must_use]
    pub fn source() -> axioval_ir::contract::AggregateSource {
        axioval_ir::contract::AggregateSource::Selector {
            selector: Box::new(axioval_ir::contract::Selector::Expression {
                expression: Box::new(Expression::Parameter {
                    name: SELECTION.to_owned(),
                    label: Some("the objects the rule selects in the scope".to_owned()),
                }),
            }),
        }
    }
}

/// The name a template reads the rule's own selection under: no parameter
/// of any capability, only a block editor's name for it.
pub const SELECTION: &str = "selection";

/// The members of an anchor: the objects a selector parameter picks that
/// the rule's traversal (`relationship` or `path`) reaches from the
/// anchor, or, without one, every such object of the anchor's own source
/// but the anchor. A value reads them as an aggregate over
/// [`Members::source`]; the runner supplies the anchor's members in its
/// place. Findings relate the members surely picked.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Members {
    /// The selector parameter picking members.
    pub selector: &'static str,
    /// What members the selector cannot decide leave.
    pub undecided: UndecidedMembers,
    /// Whether an unstated selector parameter picks every object, rather
    /// than leaving the rule open as required.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub every_when_unstated: bool,
    /// The string-list parameter naming a path that, where stated, keeps
    /// only the members from which it reaches the same objects as from
    /// the anchor (a revolving door's swing door between the same two
    /// spaces). A member whose ends cannot be read is a possible member;
    /// an anchor whose ends cannot be read, or reach nothing, is open.
    /// `{relation}` then ends `with the same ends via <path>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_ends: Option<&'static str>,
    /// Further selector parameters picking members of the same anchor
    /// along the same traversal, each read as an aggregate over
    /// [`Members::source`] of its own name: two populations, such as a
    /// ratio's numerator and denominator. Their undecided members count
    /// with the first population's for `undecided`.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub more: &'static [&'static str],
    /// Judgements of each member of the first population on its own, made
    /// just before the anchor reads the value each names.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<MemberCheck>,
}

/// A judgement of each member of an anchor's first population on its own,
/// the member in scope: a finding on the member relating the anchor (once
/// per member, however many anchors reach it), or the member open.
///
/// It is made just before the anchor reads its value `before`, and only
/// while the anchor is judged that far; once that value is read, an anchor
/// with a member it found is left open as invalid evidence (`failed`:
/// `{failed}` how many, `{first}` the first). A member whose first value
/// is `null` or cannot be read is not judged (the value the anchor reads
/// refuses it where it must); a later value that cannot be read leaves the
/// member open (`open`, `{why}` its refusal).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberCheck {
    /// The anchor's value before which members are judged.
    pub before: &'static str,
    /// The values read for each member, in order.
    pub values: Vec<TemplateValue>,
    /// How they decide: a `Within`.
    pub decision: Decision,
    /// The finding on a member where it fails.
    pub fail: &'static str,
    /// The member open where it cannot decide.
    pub undecided: &'static str,
    /// The member open where a value after the first cannot be read.
    pub open: &'static str,
    /// The anchor open where a member failed.
    pub failed: &'static str,
}

impl Members {
    /// The aggregate source a value reads an anchor's members through:
    /// the objects the member selector picks, as a block editor shows it.
    /// Run, it is the anchor's members.
    #[must_use]
    pub fn source(selector: &str) -> axioval_ir::contract::AggregateSource {
        axioval_ir::contract::AggregateSource::Selector {
            selector: Box::new(axioval_ir::contract::Selector::Expression {
                expression: Box::new(Expression::Parameter {
                    name: selector.to_owned(),
                    label: None,
                }),
            }),
        }
    }
}

/// How an anchor with members the selector cannot decide is judged.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UndecidedMembers {
    /// Undecided members can only add to the value (a sum of areas): only
    /// a value surely above the maximum stands, and the anchor is otherwise
    /// left not evaluated with `message` (`{undecided}` the count of
    /// undecided members, `{relation}` how they are reached). The anchor
    /// has no row in the form's table: its value is known only from below.
    OnlyExcess { message: &'static str },
    /// Undecided members are possible members of the aggregate: a count
    /// widens over them (from the members surely picked to every member
    /// that may be), as the evaluator widens any aggregate, and the
    /// decision judges the widened value, a verdict standing only where
    /// they cannot change it. `{undecided}` is their count.
    Widen,
    /// Any object the selector cannot decide, anywhere, leaves every
    /// anchor open (members are ordered among each other): with
    /// `message`, `{why}` the first such object's refusal, for its reason.
    Refuse { message: &'static str },
    /// Any member, of any population, the selector cannot decide leaves
    /// the anchor open with `message` (`{undecided}` their count,
    /// `{relation}` how they are reached) before any value is read: a value
    /// over members that may be there is never judged.
    Open { message: &'static str },
}

/// A report table a form fills: one row per selected object whose values
/// were read, keyed by the object.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    /// The table's name, a report contract.
    pub name: &'static str,
    /// Its columns, in order.
    pub columns: Vec<Column>,
}

/// The dimension of a [`Column`] holding a plain number (a ratio, a
/// count): no SI base unit, which names no quantity's dimension.
pub const NUMBER: axioval_ir::QuantityDimension =
    axioval_ir::QuantityDimension::Other { exponents: [0; 7] };

/// One column of a [`Table`]: a value of the form, under an id that may
/// name a [`Text`] (`{column}`), in a quantity's dimension, or a plain
/// number ([`NUMBER`]).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: &'static str,
    pub value: &'static str,
    pub dimension: axioval_ir::QuantityDimension,
}

/// One value of a form: an expression the shared evaluator evaluates for
/// the object. A value that cannot be read leaves the object not
/// evaluated with the reason and the refusal of whatever it read
/// (`{why}`); a `null` one is a missing-information finding, `absent`
/// wording it; one not of the `expect`ed kind leaves the object not
/// evaluated as invalid evidence, `mismatch` wording it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateValue {
    pub name: &'static str,
    pub expression: Expression,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expect: Option<Expect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absent: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mismatch: Option<&'static str>,
}

/// The kind of value a [`TemplateValue`] must have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Expect {
    /// A finite length: a stated property must state a length quantity.
    Length,
}

/// How a form's values decide.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Decision {
    /// The generic range judge: the value `value` lies within `minimum`
    /// and `maximum` (each inclusive, either absent where a parameter it
    /// sums is unstated), both widened by [`ROUNDING_ULPS`] times the
    /// largest of the `rounding` magnitudes. A verdict needs the whole
    /// interval on one side of a bound; one straddling it is undecided,
    /// naming the bound (`{bound}`: `at least 0.26 m`).
    Within {
        value: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        minimum: Option<Vec<Term>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        maximum: Option<Vec<Term>>,
        rounding: Vec<Magnitude>,
    },
    /// The generic comparison judge: the stated value `value` against the
    /// target a rule states, by the operator it names ([`Comparison`]),
    /// decided by the one comparison every rule uses
    /// ([`crate::comparison`]).
    Compare {
        value: &'static str,
        comparison: Comparison,
    },
    /// A group decision: the stated value `value` of each selected object
    /// compared with those of the other objects of its group, a finding on
    /// every object sharing it with another, relating them ([`Unique`]).
    Unique { value: &'static str, unique: Unique },
    /// The anchor's members ([`Form::members`]) read and judged one by
    /// one, against their neighbours, a reference prevailing among them,
    /// and their own nested members ([`Each`]). Findings are on the
    /// members (or nested members), several per member where several
    /// judgements fail.
    Each(Box<Each>),
    /// A value agrees with a reference within a tolerance: a finding where
    /// the whole interval lies beyond it, the subject open where part of
    /// it does. Only a judgement of [`Each`] uses it.
    Near {
        value: &'static str,
        reference: Reference,
        tolerance: Operand,
    },
}

/// Members read and judged one by one: what [`Decision::Each`] decides.
///
/// Each member of an anchor (decided, as [`UndecidedMembers::Refuse`]
/// requires) reads `values` with itself in scope; `order` orders them,
/// lowest first and ties by identity, a member whose `order` is not a
/// stated length leaving the anchor open (`unordered`, `{member}`). The
/// members' `rise` is the difference of the next member's `order` and the
/// member's own, in plain binary arithmetic as the capabilities computed
/// it; the last member's comes from `last`, or leaves it open (`open`),
/// and `skip_first`/`skip_last` (boolean parameters) leave the first or
/// last member without one. A rise is read only where an applicable
/// judgement reads it. `checks` judge each member having every value they
/// read; `nested` reads further populations per member. `table` holds one
/// row per member, its values or unknown.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Each {
    pub values: Vec<TemplateValue>,
    pub order: &'static str,
    pub unordered: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_first: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_last: Option<&'static str>,
    pub rise: Rise,
    pub checks: Vec<Judgement>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nested: Vec<Nested>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<Table>,
}

/// The difference of the next member's `order` and a member's own.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rise {
    /// The name the rise is read under.
    pub name: &'static str,
    /// The last member's rise: the highest upper end of a value of a
    /// nested population, less the member's `order`, where that population
    /// applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<Highest>,
    /// The last member left open where no `last` applies.
    pub open: &'static str,
}

/// The highest `value` among the members of the nested population
/// `nested`: lower and upper ends each the greatest.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Highest {
    pub nested: &'static str,
    pub value: &'static str,
}

/// When a judgement or nested population applies: every parameter of
/// `when` stated (a boolean, or its default, true), one of `any` where it
/// names some, and `condition` holding.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applies {
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub when: &'static [&'static str],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub any: &'static [&'static str],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
}

/// One judgement of each subject (a member, or a nested member): its own
/// outcome, so a subject failing several has several findings.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Judgement {
    pub applies: Applies,
    /// A `Within` or `Near` decision over the subject's values.
    pub decision: Decision,
    pub fail: &'static str,
    pub undecided: &'static str,
    /// Fewer subjects with the values it reads: nothing judged (a
    /// prevailing value needs two).
    pub least: usize,
}

/// The reference a [`Decision::Near`] compares with.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Reference {
    /// A value of the subject, or (`member:<name>`) of its member.
    Value(&'static str),
    /// The prevailing exact value `value` among the judgement's subjects:
    /// the one most share within the tolerance, the lowest among equally
    /// common ones. Without one each subject is open with `missing`, or
    /// nothing is judged without a message.
    Prevailing(Prevailing),
}

/// The prevailing exact value `value` among a judgement's subjects
/// ([`Reference::Prevailing`]), and the message leaving each subject open
/// without one.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prevailing {
    pub value: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<&'static str>,
}

/// Objects a path reaches from each member (a storey's spaces, a level's
/// contents), read and judged per member.
///
/// They are the objects a selector parameter picks (every object where
/// unstated) that the path parameter reaches; any whose selection is
/// undecided leaves the member open (`undecided`, `{undecided}` their
/// count, `{relation}` the path). Fewer than `least` nested members judge
/// nothing, or leave the member open with `fewer`. Without `services` the
/// member is open with their message, checked first where
/// `services_first`. A value of a nested member that cannot be read leaves
/// it open, or, `errors_open_member`, the member. `differences` derive
/// values in plain binary arithmetic (`[a.lower − b.upper, a.upper −
/// b.lower]`); `table` holds a row per nested member read.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Nested {
    pub name: &'static str,
    pub applies: Applies,
    pub path: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<&'static str>,
    pub undecided: &'static str,
    pub least: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fewer: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<Services>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub services_first: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub errors_open_member: bool,
    /// Only members whose value of this name is known have the population.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members_with: Option<&'static str>,
    pub values: Vec<TemplateValue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub differences: Vec<Difference>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<Judgement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<NestedTable>,
}

impl Nested {
    /// The aggregate source a value reads a member's nested population
    /// through, as a block editor shows it.
    #[must_use]
    pub fn source(&self) -> axioval_ir::contract::AggregateSource {
        axioval_ir::contract::AggregateSource::Path {
            path: vec![format!("{{{}}}", self.path)],
        }
    }
}

/// A value derived as `minuend − subtrahend`, both intervals.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    pub name: &'static str,
    pub minuend: &'static str,
    pub subtrahend: &'static str,
}

/// A report table of nested members: the member's identity as text under
/// `member`, then columns of the nested member's values or
/// (`member:<name>`) its member's.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NestedTable {
    pub name: &'static str,
    pub member: &'static str,
    pub columns: Vec<Column>,
}

/// What [`Decision::Unique`] reads of the rule.
///
/// Objects are grouped per source, or across the project where the
/// boolean parameter `across` is true, and narrowed by the rule's
/// traversal to the objects reaching the same related objects (the spaces
/// of one storey). Within a group, text is compared trimmed (`trim`) and
/// folded (unless `case_sensitive`), and, where the rule declares a
/// tolerance (`tolerance`, `relative_tolerance`, `decimals`), numbers and
/// quantities of one dimension pair by pair within it, or by their
/// rounding. Each object sharing its value gets one finding relating the
/// others (`{others}` their count, `{value:stated}` the value,
/// `{tolerance:suffix}` the tolerance). A value the source states absent,
/// `null` or blank is a finding worded `missing` where the boolean
/// parameter `require` holds, and is not compared otherwise.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unique {
    pub across: &'static str,
    pub trim: &'static str,
    pub case_sensitive: &'static str,
    pub require: &'static str,
    pub missing: &'static str,
}

/// A comparison a rule states as an operator word and at most one target
/// parameter: what [`Decision::Compare`] judges.
///
/// Binding checks the statement in the order of `targets`: the operator
/// (`operator`) takes exactly one stated target, none for a `presence`
/// word (``operator `x` takes 1 target value(s); 0 given``); the stated
/// target's kind admits the operator (``operator `x` does not apply to a
/// number``); `precision` (read before the first target that is not a
/// number) applies to a date or date-time target only; a `matches`
/// pattern compiles; and a declared tolerance (`tolerance`,
/// `relative_tolerance` or `decimals`, where `tolerance` holds) applies
/// to a numeric target only.
///
/// The judge compares what the source states. A comparison presupposes a
/// value: an absent property, `null` or a value of another kind than the
/// target fails every operator but a presence test, which reads blank
/// text as undefined. A quantity is compared only with a quantity target
/// of its dimension: against a unit-less target, or a unit-less number
/// against a quantity target, it leaves the object not evaluated, as an
/// integer beyond 2^53 compared under a tolerance does.
///
/// Messages read `{target}`, the stated target as the rule declares it
/// after a space (nothing for a presence test), and `{tolerance:suffix}`,
/// the declared tolerance (`within tolerance 0.01` in parentheses, nothing
/// when exact).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// The string parameter naming the operator.
    pub operator: &'static str,
    /// The operator words that test presence and take no target.
    pub presence: &'static [Presence],
    /// The target parameters, in the order a statement is checked.
    pub targets: Vec<ComparisonTarget>,
    /// The boolean parameter that, `false`, folds the case of text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_sensitive: Option<&'static str>,
    /// The string parameter stating a date comparison's precision (`day`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<&'static str>,
    /// Whether the rule's tolerance parameters apply to numeric targets.
    pub tolerance: bool,
}

/// An operator word testing presence: whether the value must be defined.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub word: &'static str,
    pub defined: bool,
}

/// A parameter a comparison may take its target from, its kind and the
/// operator words that apply to it.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonTarget {
    pub parameter: &'static str,
    pub kind: TargetKind,
    pub operators: &'static [Operation],
}

/// The kind of value a comparison target states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetKind {
    Integer,
    Number,
    Quantity,
    Date,
    DateTime,
    Boolean,
    Texts,
    Text,
}

impl TargetKind {
    /// How a message names the kind: `operator `x` does not apply to …`.
    #[must_use]
    pub fn noun(self) -> &'static str {
        match self {
            Self::Integer => "an integer",
            Self::Number => "a number",
            Self::Quantity => "a quantity",
            Self::Date | Self::DateTime => "a date",
            Self::Boolean => "a boolean",
            Self::Texts => "a text list",
            Self::Text => "text",
        }
    }

    /// Whether a tolerance applies to the kind.
    #[must_use]
    pub fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Number | Self::Quantity)
    }

    /// Whether a precision applies to the kind.
    #[must_use]
    pub fn is_temporal(self) -> bool {
        matches!(self, Self::Date | Self::DateTime)
    }
}

/// An operator word and the test it names.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub word: &'static str,
    pub test: Test,
}

/// What an operator tests of a value against its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "order", rename_all = "camelCase")]
pub enum Test {
    /// An order or equality.
    Order(crate::comparison::Order),
    /// The text contains the target.
    Contains,
    /// The whole text matches the target regular expression.
    Matches,
    /// The text is one of the target list.
    OneOf,
    /// The text is none of the target list.
    NoneOf,
}

/// One term of a bound: an operand added or subtracted, left to right.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    pub sign: Sign,
    pub operand: Operand,
}

impl Term {
    #[must_use]
    pub const fn plus(operand: Operand) -> Self {
        Self {
            sign: Sign::Plus,
            operand,
        }
    }

    #[must_use]
    pub const fn minus(operand: Operand) -> Self {
        Self {
            sign: Sign::Minus,
            operand,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Sign {
    Plus,
    Minus,
}

/// A value of the form or a parameter of the rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "camelCase")]
pub enum Operand {
    Value(&'static str),
    Parameter(&'static str),
}

/// A magnitude the rounding allowance scales with: one end of an
/// operand's interval.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Magnitude {
    pub end: End,
    pub operand: Operand,
}

/// An end of an interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum End {
    Lower,
    Upper,
}

impl Form {
    /// The form as one truth expression: the decision's expression form
    /// with each value's expression in place of its name, and with its
    /// checks' before it, all of them required. It still holds the
    /// template's slots (`{axis}`) and `parameter` reads until a rule's
    /// parameters are bound into it.
    #[must_use]
    pub fn requirement(&self) -> Expression {
        self.requirement_with(&|_, expression| expression.clone())
    }

    /// [`Self::requirement`], each value's expression passed through
    /// `value` (its name and expression) first.
    #[must_use]
    pub fn requirement_with(&self, value: &dyn Fn(&str, &Expression) -> Expression) -> Expression {
        let inline = |values: &[&TemplateValue], name: &str| self.inlined(values, name, value);
        let own: Vec<&TemplateValue> = self.values.iter().collect();
        let decided = self.decision.expression(&|name| inline(&own, name));
        if self.checks.is_empty() {
            return decided;
        }
        let mut operands: Vec<Expression> = self
            .checks
            .iter()
            .map(|check| {
                let values: Vec<&TemplateValue> = self.values.iter().chain(&check.values).collect();
                check.decision.expression(&|name| inline(&values, name))
            })
            .collect();
        operands.push(decided);
        Expression::And {
            operands,
            label: None,
        }
    }

    /// The value `name` as an expression: a value read, inlined; a value
    /// derived from them, as the arithmetic it states (a ratio as a
    /// division, which a block editor shows, though the runner divides by
    /// a denominator that may be zero where the evaluator refuses); any
    /// other name as a derived value of that name.
    fn inlined(
        &self,
        values: &[&TemplateValue],
        name: &str,
        value: &dyn Fn(&str, &Expression) -> Expression,
    ) -> Expression {
        if let Some(step) = values.iter().find(|step| step.name == name) {
            return value(step.name, &step.expression);
        }
        let derived = self.derived.iter().find(|derived| match derived {
            Derived::Difference(difference) => difference.name == name,
            Derived::Ratio { name: ratio, .. } => *ratio == name,
        });
        match derived {
            Some(Derived::Difference(difference)) => Expression::Subtract {
                left: boxed(self.inlined(values, difference.minuend, value)),
                right: boxed(self.inlined(values, difference.subtrahend, value)),
                label: Some(name.to_owned()),
            },
            Some(Derived::Ratio {
                numerator,
                denominator,
                ..
            }) => Expression::Divide {
                left: boxed(self.inlined(values, numerator, value)),
                right: boxed(self.inlined(values, denominator, value)),
                label: Some(name.to_owned()),
            },
            None => Expression::Derived {
                name: name.to_owned(),
                label: None,
            },
        }
    }
}

fn boxed(expression: Expression) -> Box<Expression> {
    Box::new(expression)
}

impl Decision {
    /// The decision as a truth expression, `value` giving each value's
    /// expression and parameters read as `parameter`. A
    /// [`Decision::Within`] is `value ≥ minimum − allowance` and
    /// `value ≤ maximum + allowance`, the allowance
    /// `4ε · max(|m|…)` over its magnitudes, none without one. Evaluated by the expression
    /// evaluator's sound interval arithmetic, it reaches the judge's
    /// verdicts except where a value lies within a unit in the last place
    /// of the widened bound, which the expression leaves undecided.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn expression(&self, value: &dyn Fn(&str) -> Expression) -> Expression {
        match self {
            Self::Within {
                value: subject,
                minimum,
                maximum,
                rounding,
            } => {
                let operand = |operand: &Operand| match operand {
                    Operand::Value(name) => value(name),
                    Operand::Parameter(name) => Expression::Parameter {
                        name: (*name).to_owned(),
                        label: None,
                    },
                };
                let sum = |terms: &[Term]| {
                    let mut terms = terms.iter();
                    let first = terms
                        .next()
                        .map_or(Expression::Null { label: None }, |term| {
                            let first = operand(&term.operand);
                            match term.sign {
                                Sign::Plus => first,
                                Sign::Minus => Expression::Negate {
                                    operand: boxed(first),
                                    label: None,
                                },
                            }
                        });
                    terms.fold(first, |sum, term| match term.sign {
                        Sign::Plus => Expression::Add {
                            left: boxed(sum),
                            right: boxed(operand(&term.operand)),
                            label: None,
                        },
                        Sign::Minus => Expression::Subtract {
                            left: boxed(sum),
                            right: boxed(operand(&term.operand)),
                            label: None,
                        },
                    })
                };
                // Without a magnitude to scale with there is no allowance:
                // the bound is compared as it is.
                let allowance = (!rounding.is_empty()).then(|| Expression::Multiply {
                    left: boxed(Expression::Literal {
                        value: ScalarValue::Number {
                            value: ROUNDING_ULPS,
                        },
                        label: None,
                    }),
                    right: boxed(Expression::Max {
                        operands: rounding
                            .iter()
                            .map(|magnitude| Expression::Abs {
                                operand: boxed(operand(&magnitude.operand)),
                                label: None,
                            })
                            .collect(),
                        label: None,
                    }),
                    label: Some("rounding allowance".into()),
                });
                let widened = |bound: Expression, up: bool| match allowance.clone() {
                    None => bound,
                    Some(allowance) if up => Expression::Add {
                        left: boxed(bound),
                        right: boxed(allowance),
                        label: None,
                    },
                    Some(allowance) => Expression::Subtract {
                        left: boxed(bound),
                        right: boxed(allowance),
                        label: None,
                    },
                };
                let mut operands = Vec::new();
                if let Some(terms) = minimum {
                    operands.push(Expression::Compare {
                        operator: ExpressionComparison::GreaterThanOrEquals,
                        left: boxed(value(subject)),
                        right: boxed(widened(sum(terms), false)),
                        case_sensitive: true,
                        label: Some("at least the lower bound".into()),
                    });
                }
                if let Some(terms) = maximum {
                    operands.push(Expression::Compare {
                        operator: ExpressionComparison::LessThanOrEquals,
                        left: boxed(value(subject)),
                        right: boxed(widened(sum(terms), true)),
                        case_sensitive: true,
                        label: Some("at most the upper bound".into()),
                    });
                }
                Expression::And {
                    operands,
                    label: None,
                }
            }
            Self::Compare {
                value: subject,
                comparison,
            } => comparison.expression(&value(subject)),
            // Every judgement of every member, as a block editor shows them.
            Self::Each(each) => Expression::And {
                operands: each
                    .checks
                    .iter()
                    .chain(each.nested.iter().flat_map(|nested| &nested.checks))
                    .map(|judgement| judgement.decision.expression(value))
                    .collect(),
                label: Some("each member".into()),
            },
            Self::Near {
                value: subject,
                reference,
                tolerance,
            } => {
                let reference = match reference {
                    Reference::Value(name) => value(name),
                    Reference::Prevailing(Prevailing { value: name, .. }) => Expression::Derived {
                        name: format!("prevailing {name}"),
                        label: None,
                    },
                };
                let tolerance = match tolerance {
                    Operand::Value(name) => value(name),
                    Operand::Parameter(name) => Expression::Parameter {
                        name: (*name).to_owned(),
                        label: None,
                    },
                };
                Expression::Compare {
                    operator: ExpressionComparison::LessThanOrEquals,
                    left: boxed(Expression::Abs {
                        operand: boxed(Expression::Subtract {
                            left: boxed(value(subject)),
                            right: boxed(reference),
                            label: None,
                        }),
                        label: None,
                    }),
                    right: boxed(tolerance),
                    case_sensitive: true,
                    label: Some("within the tolerance".into()),
                }
            }
            Self::Unique { value: subject, .. } => {
                // No other object the rule selects in the scope states the
                // checked object's value: at most one, itself, does.
                let member = value(subject);
                let own = match member.clone() {
                    Expression::Property {
                        property_set,
                        property,
                        label,
                        ..
                    } => Expression::Property {
                        property_set,
                        property,
                        of: Some(axioval_ir::contract::PropertyScope::Subject),
                        label,
                    },
                    other => other,
                };
                Expression::Compare {
                    operator: ExpressionComparison::LessThanOrEquals,
                    left: boxed(Expression::Aggregate {
                        function: axioval_ir::contract::AggregateFunction::Count,
                        over: Scopes::source(),
                        filter: Some(Box::new(axioval_ir::contract::Selector::Expression {
                            expression: boxed(Expression::Compare {
                                operator: ExpressionComparison::Equals,
                                left: boxed(member),
                                right: boxed(own),
                                case_sensitive: false,
                                label: None,
                            }),
                        })),
                        value: None,
                        label: Some("objects stating the same value".into()),
                    }),
                    right: boxed(Expression::Literal {
                        value: ScalarValue::Integer { value: 1 },
                        label: None,
                    }),
                    case_sensitive: true,
                    label: None,
                }
            }
        }
    }
}

impl Test {
    /// The expression operator of a test that compares two operands.
    fn operator(self) -> Option<ExpressionComparison> {
        use crate::comparison::Order;
        Some(match self {
            Self::Order(Order::Equal) => ExpressionComparison::Equals,
            Self::Order(Order::NotEqual) => ExpressionComparison::NotEquals,
            Self::Order(Order::Less) => ExpressionComparison::LessThan,
            Self::Order(Order::LessOrEqual) => ExpressionComparison::LessThanOrEquals,
            Self::Order(Order::Greater) => ExpressionComparison::GreaterThan,
            Self::Order(Order::GreaterOrEqual) => ExpressionComparison::GreaterThanOrEquals,
            Self::Contains => ExpressionComparison::Contains,
            Self::Matches => ExpressionComparison::Matches,
            Self::OneOf | Self::NoneOf => return None,
        })
    }

    /// `subject` tested against `target`.
    #[must_use]
    pub fn expression(
        self,
        subject: Expression,
        target: Expression,
        case_sensitive: bool,
    ) -> Expression {
        match self.operator() {
            Some(operator) => Expression::Compare {
                operator,
                left: boxed(subject),
                right: boxed(target),
                case_sensitive,
                label: None,
            },
            None if self == Self::OneOf => Expression::OneOf {
                operand: boxed(subject),
                values: vec![target],
                case_sensitive,
                label: None,
            },
            None => Expression::NoneOf {
                operand: boxed(subject),
                values: vec![target],
                case_sensitive,
                label: None,
            },
        }
    }
}

impl Comparison {
    /// The comparison as one expression over `subject`, for a block editor:
    /// an `if` choosing, by the operator word and the stated target, the
    /// test it names (`isDefined`, `isUndefined`, `compare`, `oneOf`,
    /// `noneOf`), each reading the target as a `parameter`. A rule forked
    /// from the template states only its own test, its target a literal.
    #[must_use]
    pub fn expression(&self, subject: &Expression) -> Expression {
        let parameter = |name: &str| Expression::Parameter {
            name: name.to_owned(),
            label: None,
        };
        let names = |word: &str| Expression::Compare {
            operator: ExpressionComparison::Equals,
            left: boxed(parameter(self.operator)),
            right: boxed(Expression::Literal {
                value: ScalarValue::String {
                    value: word.to_owned(),
                },
                label: None,
            }),
            case_sensitive: true,
            label: Some(format!("`{}` is `{word}`", self.operator)),
        };
        let mut branches: Vec<axioval_ir::contract::Branch> = self
            .presence
            .iter()
            .map(|presence| axioval_ir::contract::Branch {
                when: names(presence.word),
                then: if presence.defined {
                    Expression::IsDefined {
                        operand: boxed(subject.clone()),
                        label: None,
                    }
                } else {
                    Expression::IsUndefined {
                        operand: boxed(subject.clone()),
                        label: None,
                    }
                },
            })
            .collect();
        for target in &self.targets {
            for operation in target.operators {
                branches.push(axioval_ir::contract::Branch {
                    when: Expression::And {
                        operands: vec![
                            Expression::IsDefined {
                                operand: boxed(parameter(target.parameter)),
                                label: Some(format!("`{}` is stated", target.parameter)),
                            },
                            names(operation.word),
                        ],
                        label: None,
                    },
                    then: operation.test.expression(
                        subject.clone(),
                        parameter(target.parameter),
                        true,
                    ),
                });
            }
        }
        Expression::If {
            branches,
            otherwise: boxed(Expression::Null { label: None }),
            label: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn within(rounding: Vec<Magnitude>) -> Decision {
        Decision::Within {
            value: "count",
            minimum: None,
            maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
            rounding,
        }
    }

    /// A form's checks are required beside its own decision, each reading
    /// the form's values and its own.
    #[test]
    fn a_form_with_checks_requires_them_all() {
        let value = |name: &'static str| TemplateValue {
            name,
            expression: Expression::Parameter {
                name: format!("read_{name}"),
                label: None,
            },
            expect: None,
            absent: None,
            mismatch: None,
        };
        let form = Form {
            when: &[],
            values: vec![value("count")],
            decision: within(Vec::new()),
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: Some("count"),
            checks: vec![FormCheck {
                values: vec![value("height")],
                decision: Decision::Within {
                    value: "height",
                    minimum: Some(vec![Term::plus(Operand::Value("count"))]),
                    maximum: None,
                    rounding: Vec::new(),
                },
                fail: "",
                undecided: "",
                related: None,
            }],
        };
        let Expression::And { operands, .. } = form.requirement() else {
            panic!("an `and` of the checks and the form's decision");
        };
        assert_eq!(operands.len(), 2);
        let check = serde_json::to_string(&operands[0]).unwrap();
        assert!(
            check.contains("read_height") && check.contains("read_count"),
            "{check}"
        );
    }

    /// Without a magnitude to scale with, a bound is compared as it is;
    /// with one, it is widened by the rounding allowance.
    #[test]
    fn a_bound_is_widened_only_by_a_magnitude() {
        let value = |name: &str| Expression::Derived {
            name: name.to_owned(),
            label: None,
        };
        let plain = serde_json::to_string(&within(Vec::new()).expression(&value)).unwrap();
        assert!(!plain.contains("rounding allowance"), "{plain}");
        assert!(
            plain.contains("\"right\":{\"kind\":\"parameter\",\"name\":\"maximum\"}"),
            "{plain}"
        );
        let widened = serde_json::to_string(
            &within(vec![Magnitude {
                end: End::Upper,
                operand: Operand::Value("count"),
            }])
            .expression(&value),
        )
        .unwrap();
        assert!(widened.contains("rounding allowance"), "{widened}");
    }
}
