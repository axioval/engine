#![allow(missing_docs)]
use super::{Expression, ParameterValue};
use crate::{Discipline, TemporalPrecision};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Selector {
    All,
    EntityType {
        #[serde(rename = "objectType")]
        object_type: String,
        #[serde(rename = "includeSubtypes", default = "yes")]
        include_subtypes: bool,
    },
    Property {
        #[serde(rename = "propertySet")]
        property_set: Option<String>,
        property: String,
        operator: ComparisonOperator,
        value: Option<ParameterValue>,
        /// Whether text comparisons respect case; `false` folds both sides.
        #[serde(
            rename = "caseSensitive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        case_sensitive: bool,
        /// Whether a text value is trimmed of surrounding whitespace first.
        #[serde(default, skip_serializing_if = "is_false")]
        trim: bool,
        /// How a list value is compared: `any` element or `all` of them
        /// must satisfy the operator. A scalar value counts as a list of
        /// one. Without it a list value is not evaluated, never compared as
        /// a whole; `exists` takes none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantifier: Option<Quantifier>,
        /// How finely dates and date-times compare. `day` reads a date-time
        /// as the calendar day it states, so it compares with a date; without
        /// it a date-time compared with a date is not evaluated. Applies to
        /// a `date` or `dateTime` value only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        precision: Option<TemporalPrecision>,
    },
    /// Objects by the properties whose set and name match XML Schema
    /// patterns, as IDS names them (`Pset_.*Common`).
    ///
    /// A pattern matches the whole name the source states; it names no
    /// concept and is never bound through the package vocabulary. Without
    /// `propertySetPattern` every property set is searched. The selector
    /// holds when at least one property matches and the comparison holds
    /// for `matched` of them (`any` or `all`); no matching property is no
    /// match. The other fields compare each matched property's value as a
    /// `property` selector does.
    PropertyPattern {
        #[serde(
            rename = "propertySetPattern",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        property_set_pattern: Option<String>,
        #[serde(rename = "propertyPattern")]
        property_pattern: String,
        /// Which matched properties must satisfy the comparison.
        matched: Quantifier,
        operator: ComparisonOperator,
        value: Option<ParameterValue>,
        #[serde(
            rename = "caseSensitive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        case_sensitive: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        trim: bool,
        /// How each matched list, bounded or table value is compared.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantifier: Option<Quantifier>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        precision: Option<TemporalPrecision>,
    },
    /// Objects carrying a classification in `system`, as their source
    /// states it.
    ///
    /// `code` names one code exactly and `codePattern` matches codes by an
    /// XML Schema pattern over the whole code (`Ss_25_.*`), as IDS writes
    /// classification patterns; at most one of them is given. With neither,
    /// any classification in `system` matches. `includeDescendants` also
    /// matches the codes an assignment's ancestors carry, and needs a code or
    /// a pattern.
    Classification {
        system: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(
            rename = "codePattern",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        code_pattern: Option<String>,
        #[serde(rename = "includeDescendants", default)]
        include_descendants: bool,
    },
    /// Objects a classification of the ruleset (`classifications`)
    /// assigns `class`, or, with `includeDescendants`, `class` or any class
    /// below it in the classification's tree.
    ///
    /// An all-match classification selects an object when any class it
    /// assigns does. An unclassified object is not selected; one whose class
    /// cannot be derived is not evaluated. `class` is a declared class of a
    /// hierarchical classification, or a class a flat one's rows assign,
    /// which has no descendants.
    DerivedClass {
        classification: String,
        class: String,
        #[serde(
            rename = "includeDescendants",
            default,
            skip_serializing_if = "is_false"
        )]
        include_descendants: bool,
    },
    /// The groups a grouping of the ruleset (`groupings`) derives: derived
    /// objects, one per group, never a model object. Only this selector
    /// reaches them, as only an `entityType` reaches resource objects.
    DerivedGroup {
        grouping: String,
    },
    AllOf {
        operands: Vec<Selector>,
    },
    AnyOf {
        operands: Vec<Selector>,
    },
    Not {
        operand: Box<Selector>,
    },
    /// Objects by the objects a relationship `path` reaches from them.
    ///
    /// Each step is `Relationship` or `Relationship:direction` (`forward`,
    /// the default, `backward` or `either`), walked one after another; a step
    /// ending in `+` is taken one or more times, reaching every object along
    /// the relationship's chain. A step may name several relationships
    /// separated by `|`, with one direction after the last applying to all
    /// (`IfcRelFillsElement|IfcRelVoidsElement:backward+`); it takes any of
    /// them, mixing them along a chain. The reached objects are tested
    /// against `selector` under `quantifier`.
    Related {
        path: Vec<String>,
        /// Which reached objects must match; `any` is omitted when
        /// serialized.
        #[serde(default, skip_serializing_if = "RelatedQuantifier::is_any")]
        quantifier: RelatedQuantifier,
        selector: Box<Selector>,
    },
    /// Objects of the sources the host declared to play `value`, such as
    /// `structure`.
    ///
    /// A discipline is source metadata, not an object fact: every object of
    /// a source matches or none does. An object whose source declares no
    /// discipline is not evaluated, never a non-match, so a discipline-scoped
    /// rule cannot pass vacuously over a model nobody classified.
    Discipline {
        value: Discipline,
    },
    /// Objects of the sources whose metadata `field` satisfies the
    /// comparison, such as the application that wrote a model.
    ///
    /// Source metadata is not an object fact: every object of a source
    /// matches or none does. A field compares as a property selector's value
    /// does: one holding several values (a model written by two
    /// applications) needs `quantifier`, and one the source states it lacks
    /// matches nothing, as an absent property does. A source whose field was
    /// never read is not evaluated, never a non-match.
    Source {
        field: SourceField,
        operator: ComparisonOperator,
        value: Option<ParameterValue>,
        #[serde(
            rename = "caseSensitive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        case_sensitive: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        trim: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantifier: Option<Quantifier>,
    },
    /// Objects by how another rule of the same ruleset judged them.
    ///
    /// `passed` holds for an object the rule `rule` selected and reported
    /// nothing about; `failed` for an object it reported a finding about.
    /// An object the other rule left not evaluated, or could not decide
    /// whether it selected, is not evaluated, never a match or a non-match;
    /// so is every object of a source or project the other rule reported
    /// about as a whole. The plan runs `rule` first; a cycle of such
    /// references fails compilation.
    RuleOutcome {
        rule: String,
        outcome: RuleOutcomeKind,
    },
    /// Objects for which `expression`, a truth, holds.
    ///
    /// It is evaluated per candidate object with three-valued truth: true
    /// selects, false and `null` do not, and a value that cannot be read
    /// or decided (a measured interval straddling a bound) leaves the object
    /// not evaluated, never skipped. It reads properties, measured and
    /// derived values, never a rule's parameters; it is type checked when
    /// the ruleset is compiled.
    Expression {
        expression: Box<Expression>,
    },
    /// Exactly these objects: an engine-internal narrowing (a rule run per
    /// group of objects whose computed parameters agree). It is never read
    /// from or written to a package.
    #[serde(skip)]
    Objects {
        objects: std::collections::BTreeSet<crate::ObjectId>,
    },
}
/// Which judgement of another rule a `ruleOutcome` selector selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum RuleOutcomeKind {
    /// Selected by the rule, and nothing found or left open about it.
    Passed,
    /// The subject of at least one of the rule's findings.
    Failed,
}
/// A fact about a whole source that a `source` selector compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum SourceField {
    /// The file name the host read the source from.
    FileName,
    /// The name of every application the source states wrote it.
    Application,
    /// The schema the source declares, such as `IFC4`.
    Schema,
    /// The name of the project the source describes.
    Project,
    /// When the source states it was written, as written: for IFC the
    /// header's `FILE_NAME.time_stamp`.
    Timestamp,
}
impl SourceField {
    /// The field's spelling in a package.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FileName => "fileName",
            Self::Application => "application",
            Self::Schema => "schema",
            Self::Project => "project",
            Self::Timestamp => "timestamp",
        }
    }
}
impl Default for Selector {
    fn default() -> Self {
        Self::All
    }
}
impl Selector {
    /// Every expression this selector holds, in its own `expression`
    /// operands and those of its nested selectors, in written order.
    #[must_use]
    pub fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Expression { expression } => vec![expression],
            Self::AllOf { operands } | Self::AnyOf { operands } => {
                operands.iter().flat_map(Self::expressions).collect()
            }
            Self::Not { operand } => operand.expressions(),
            Self::Related { selector, .. } => selector.expressions(),
            _ => Vec::new(),
        }
    }

    /// Every rule whose outcomes this selector reads: its `ruleOutcome`
    /// selectors and the rule reads of its expressions.
    #[must_use]
    pub fn rule_references(&self) -> Vec<&str> {
        match self {
            Self::RuleOutcome { rule, .. } => vec![rule],
            Self::Expression { expression } => expression.rule_references(),
            Self::AllOf { operands } | Self::AnyOf { operands } => {
                operands.iter().flat_map(Self::rule_references).collect()
            }
            Self::Not { operand } => operand.rule_references(),
            Self::Related { selector, .. } => selector.rule_references(),
            _ => Vec::new(),
        }
    }

    /// Renames every rule [`Selector::rule_references`] lists.
    pub fn rename_rules(&mut self, rename: &dyn Fn(&str) -> String) {
        match self {
            Self::RuleOutcome { rule, .. } => *rule = rename(rule),
            Self::Expression { expression } => expression.rename_rules(rename),
            Self::AllOf { operands } | Self::AnyOf { operands } => {
                operands
                    .iter_mut()
                    .for_each(|operand| operand.rename_rules(rename));
            }
            Self::Not { operand } => operand.rename_rules(rename),
            Self::Related { selector, .. } => selector.rename_rules(rename),
            _ => {}
        }
    }

    /// A property selector with case-sensitive, untrimmed text comparison.
    #[must_use]
    pub fn property(
        property_set: Option<String>,
        property: impl Into<String>,
        operator: ComparisonOperator,
        value: Option<ParameterValue>,
    ) -> Self {
        Self::Property {
            property_set,
            property: property.into(),
            operator,
            value,
            case_sensitive: true,
            trim: false,
            quantifier: None,
            precision: None,
        }
    }
}
/// Which elements of a list value a property selector must hold for.
///
/// `all` never holds vacuously: an empty list satisfies neither quantifier,
/// as an absent value satisfies no comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Quantifier {
    /// At least one element satisfies the comparison.
    Any,
    /// Every element satisfies it, and there is at least one.
    All,
}
/// Which of the objects a `related` selector reaches must match its selector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum RelatedQuantifier {
    /// At least one reached object matches.
    #[default]
    Any,
    /// Every reached object matches, and at least one is reached.
    All,
    /// No reached object matches; holds when none is reached.
    None,
}
impl RelatedQuantifier {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }
}
/// How a property selector compares the resolved value with its `value`.
///
/// `matches` is a regular expression and `like` a wildcard pattern (`*` any
/// run, `?` one character, `\` escapes); both must match the whole value.
/// `contains` takes a string, `oneOf` and `noneOf` a string list. The ordered
/// operators also take a `date` or `dateTime`, compared chronologically.
/// `exists`, `isEmpty` and `isNotEmpty` judge presence and take no value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum ComparisonOperator {
    Equals,
    NotEquals,
    LessThan,
    LessThanOrEquals,
    GreaterThan,
    GreaterThanOrEquals,
    Matches,
    Like,
    Contains,
    OneOf,
    NoneOf,
    Exists,
    /// Present, but null, blank text or a list of nothing else: the `empty`
    /// presence of `property-requirements`. An absent value is not empty.
    IsEmpty,
    /// Present with a value: the `not-empty` presence of
    /// `property-requirements`. An absent value is not a value.
    IsNotEmpty,
}
const fn yes() -> bool {
    true
}
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_true(value: &bool) -> bool {
    *value
}
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}
