//! Information takeoff: the selection counted, and stated, measured and
//! related values aggregated per group of objects.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    BoundaryCoverage, BoundaryCoverageRequest, BoundaryCoverageServiceHandle, BoundaryPlacement,
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::{CategoryLevel, Selector};
use axioval_ir::{
    ColumnExactness, MEASURED_AREA, MEASURED_SET, MEASURED_VOLUME, Object, ObjectId, PropertyValue,
    QuantityDimension, ReportColumn, ReportTable, ReportValue, Scope,
};

use crate::allowed_profile::{dimension_columns, dimension_value, read_profile};
use crate::body_facts::BodyFacts;
use crate::selection::{NameSpec, Selection, enumerate, object_by_id, selector_matches};
use crate::space_boundary_coverage::coverage_error;
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, category_headings, display, exact_f64,
    invalid, resolve, undefined,
};

mod expression;

use expression::{Expression, Failure, Interval, Unit};

/// The name of the table a takeoff reports.
pub const TAKEOFF_TABLE: &str = "takeoff";
/// How many group keys a takeoff declares at most (`group_1` ...).
const GROUPS: usize = 3;
/// How many columns a takeoff declares at most (`measure_1` ...).
const MEASURES: usize = 8;
/// The column kinds a measure may declare (`measure_<n>_kind`).
const KINDS: &str = "property, related, boundary_area, property_set, profile or computed";

/// Counts the rule's selection per group and aggregates values of each
/// group into the report table `takeoff`.
///
/// Groups are keyed by up to three properties (`group_1` to `group_3`),
/// each read on the object or, with `group_<n>_path`, on the objects the
/// path reaches (the storey), as a rule's categories read them: distinct
/// values join in one text, no value is `-`. A derived classification is
/// the property `<id>` in `axioval:classification`, and a level of a
/// hierarchical one `<id>;level=<n>`. Each row counts its group (`count`)
/// and aggregates up to eight columns (`measure_1` to `measure_8`) by
/// `sum`, `min`, `max`, `mean` or `values` (the distinct values, listed).
///
/// A column is of one kind (`measure_<n>_kind`): a `property` of the
/// object (the default), stated or measured; a property of the objects a
/// relationship path reaches (`related`, summed over them when numeric,
/// listed otherwise); the area of the space boundaries whose bounding
/// element a selector selects (`boundary_area`); every property of a
/// property set (`property_set`), one column per property found; or the
/// swept profile of the body (`profile`), its type, name and every
/// dimension found; or `computed`, a checked arithmetic expression over
/// other columns of the member (`area × 42.5 EUR/m²`), parsed and
/// unit-checked when the rule is bound and evaluated over intervals. Each
/// column states its exactness.
///
/// Values are intervals sure to hold the exact value, so aggregates are
/// too. An object whose selection cannot be decided may or may not belong
/// to its group, and one whose group cannot be read may belong to any group
/// of its scope: either widens the count and the aggregates of every group
/// it may belong to, and is reported not evaluated. A member whose value is
/// absent or unreadable makes its group's numeric aggregate unknown. The
/// takeoff raises no finding.
pub struct QuantityTakeoff;

impl RuleCapability for QuantityTakeoff {
    fn id(&self) -> &'static str {
        "axioval:capability.quantity-takeoff"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = Vec::new();
        for n in 1..=GROUPS {
            parameters.push(ParameterDescriptor::optional(
                format!("group_{n}"),
                ParameterType::PropertyReference,
            ));
            parameters.push(ParameterDescriptor::optional(
                format!("group_{n}_path"),
                ParameterType::StringList,
            ));
            parameters.push(ParameterDescriptor::optional(
                format!("group_{n}_name"),
                ParameterType::String,
            ));
        }
        for n in 1..=MEASURES {
            for (suffix, kind) in [
                ("", ParameterType::PropertyReference),
                ("_aggregates", ParameterType::StringList),
                ("_name", ParameterType::String),
                ("_kind", ParameterType::String),
                ("_path", ParameterType::StringList),
                ("_bounding", ParameterType::Selector),
                ("_property_set", ParameterType::String),
                ("_expression", ParameterType::String),
                ("_unit", ParameterType::String),
            ] {
                parameters.push(ParameterDescriptor::optional(
                    format!("measure_{n}{suffix}"),
                    kind,
                ));
            }
        }
        parameters.push(ParameterDescriptor::optional(
            "across_sources",
            ParameterType::Boolean,
        ));
        parameters.push(ParameterDescriptor::optional(
            "boundary_plane_tolerance",
            ParameterType::Quantity,
        ));
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declaration = match Declaration::parse(rule) {
            Ok(declaration) => declaration,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("quantity-takeoff: {message}"),
                );
            }
        };
        let mut evaluation = CapabilityEvaluation::default();
        let universe: Vec<&Object> = context.project.objects().collect();
        let mut members = Vec::new();
        let mut scopes: BTreeMap<Scope, Vec<usize>> = BTreeMap::new();
        for object in context.project.objects() {
            let Some(member) = declaration.member(context, rule, object, &universe) else {
                continue;
            };
            let scope = if declaration.across_sources {
                Scope::Project
            } else {
                Scope::Source(object.id.source.clone())
            };
            scopes.entry(scope).or_default().push(members.len());
            members.push(member);
        }
        let fields = declaration.fields(&members, &mut evaluation);
        for member in &members {
            member.report(&fields, &mut evaluation);
        }
        let table = match declaration.table(rule, &fields, &members, &scopes) {
            Ok(table) => table,
            Err(error) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidEvidence,
                    format!("quantity-takeoff: {error}"),
                );
            }
        };
        evaluation.push_table(table);
        evaluation
    }
}

/// How a column's values are combined per group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Aggregate {
    Sum,
    Min,
    Max,
    Mean,
    /// The distinct values, listed as text.
    Values,
}

impl Aggregate {
    fn parse(name: &str) -> Result<Self, Unavailable> {
        Ok(match name {
            "sum" => Self::Sum,
            "min" => Self::Min,
            "max" => Self::Max,
            "mean" => Self::Mean,
            "values" => Self::Values,
            other => {
                return Err(invalid(format!(
                    "aggregate `{other}` is unsupported; use sum, min, max, mean or values"
                )));
            }
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Min => "min",
            Self::Max => "max",
            Self::Mean => "mean",
            Self::Values => "values",
        }
    }

    /// Whether it combines numbers (otherwise it lists values).
    fn numeric(self) -> bool {
        self != Self::Values
    }
}

/// Where a column's values come from.
enum Source<'a> {
    /// A property of the object.
    Property(PropertyRef<'a>),
    /// A property of the objects a path reaches.
    Related {
        property: PropertyRef<'a>,
        path: Traversal,
    },
    /// The area of the boundaries whose element the selector selects.
    BoundaryArea(&'a Selector),
    /// Every property of a set, one column each.
    PropertySet(&'a str),
    /// The body's swept profile: type, name and dimensions.
    Profile,
    /// An expression over other columns of the member, as written and,
    /// once every column is declared, bound.
    Computed {
        text: &'a str,
        bound: Option<Box<Computed>>,
    },
}

/// A bound computed column: its expression and the column each input of
/// it reads, in the expression's order.
struct Computed {
    expression: Expression,
    inputs: Vec<Input>,
}

/// A column an expression reads: a measure or one part of a profile, the
/// unit its values must be stated in, and its name in the expression.
struct Input {
    measure: usize,
    part: Option<String>,
    unit: Unit,
    name: String,
}

impl Computed {
    /// The expression's value for a member that read `reads`: no value
    /// when an input has none, never zero; unreadable when one cannot be
    /// read, is no number of its unit, or a divisor may be zero.
    fn cell(&self, reads: &[Read]) -> Cell {
        let mut values = Vec::with_capacity(self.inputs.len());
        let mut all_exact = true;
        let mut absent = false;
        for input in &self.inputs {
            let cell = match (&reads[input.measure], &input.part) {
                (Read::Single(cell), _) => cell.clone(),
                (Read::Parts(Ok(parts)), Some(part)) => {
                    parts.get(part).cloned().unwrap_or(Cell::Absent)
                }
                (Read::Parts(Ok(_)), None) => Cell::Absent,
                (Read::Parts(Err((reason, message))), _) => {
                    Cell::Unreadable(reason.clone(), format!("cannot be read ({message})"))
                }
            };
            match cell {
                Cell::Number {
                    lower,
                    upper,
                    kind,
                    exact,
                } => {
                    let fits = match kind {
                        Some(dimension) => input.unit.takes(dimension),
                        None => input.unit.takes_plain_numbers(),
                    };
                    if !fits {
                        return Cell::Unreadable(
                            NotEvaluatedReason::InvalidEvidence,
                            format!(
                                "cannot be computed: `{}` is stated as {}, not in {}",
                                input.name,
                                kind_text(kind),
                                input.unit
                            ),
                        );
                    }
                    all_exact &= exact;
                    values.push(Interval { lower, upper });
                }
                Cell::Absent => {
                    absent = true;
                    values.push(Interval {
                        lower: 0.0,
                        upper: 0.0,
                    });
                }
                Cell::Text(texts) => {
                    return Cell::Unreadable(
                        NotEvaluatedReason::InvalidEvidence,
                        format!(
                            "cannot be computed: `{}` states `{}`, not a number",
                            input.name,
                            texts.join(", ")
                        ),
                    );
                }
                Cell::Unreadable(reason, why) => {
                    return Cell::Unreadable(
                        reason,
                        format!("cannot be computed: `{}` {why}", input.name),
                    );
                }
            }
        }
        if absent {
            return Cell::Absent;
        }
        match self.expression.evaluate(&values) {
            Ok(value) => Cell::number(
                value.lower,
                value.upper,
                self.expression.unit().dimension().ok().flatten(),
                all_exact,
            ),
            Err(Failure::ZeroDivisor) => Cell::Unreadable(
                NotEvaluatedReason::IncompleteEvidence,
                "cannot be computed: it divides by an interval that holds zero".into(),
            ),
            Err(Failure::Overflow) => {
                Cell::Unreadable(NotEvaluatedReason::InvalidEvidence, "is not finite".into())
            }
        }
    }
}

impl Source<'_> {
    /// Whether it expands to one column per part found.
    fn expands(&self) -> bool {
        matches!(self, Self::PropertySet(_) | Self::Profile)
    }

    /// The kind a numeric column of it answers in when no value says.
    fn fallback_kind(&self) -> Kind {
        match self {
            Self::Property(property) => measured_kind(*property),
            Self::BoundaryArea(_) => Some(QuantityDimension::Area),
            Self::Related { .. } | Self::PropertySet(_) | Self::Profile | Self::Computed { .. } => {
                None
            }
        }
    }

    /// How exact its values are when no value says.
    fn fallback_exactness(&self) -> ColumnExactness {
        match self {
            Self::Property(property) if property.set == Some(MEASURED_SET) => {
                ColumnExactness::Bounded
            }
            Self::BoundaryArea(_) => ColumnExactness::Bounded,
            _ => ColumnExactness::Exact,
        }
    }
}

/// One declared column (`measure_<n>`).
struct Measure<'a> {
    source: Source<'a>,
    /// The declared aggregates, if any.
    aggregates: Option<Vec<Aggregate>>,
    /// The column name after the aggregate (`sum_<name>`); an expanding
    /// column's prefix before each part (`<name>_<part>`), none when empty.
    name: String,
    /// How messages name the values read.
    what: String,
    /// The unit its values are stated in, with its scale to the coherent
    /// unit: declared (`measure_<n>_unit`), or a computed column's.
    unit: Option<(f64, Unit)>,
}

impl Measure<'_> {
    /// `cell` in the declared unit: a quantity of its dimension, or a
    /// plain number scaled from it where the unit counts a currency or is
    /// a plain number; any other number is unreadable.
    fn in_unit(&self, cell: Cell) -> Cell {
        let Some((scale, unit)) = &self.unit else {
            return cell;
        };
        match cell {
            Cell::Number {
                kind: Some(dimension),
                ..
            } if !unit.takes(dimension) => Cell::Unreadable(
                NotEvaluatedReason::InvalidEvidence,
                format!("is stated in {}, not {unit}", dimension.unit_symbol()),
            ),
            Cell::Number {
                lower,
                upper,
                kind: None,
                exact,
            } => {
                if unit.takes_plain_numbers() {
                    Cell::number(
                        lower * scale,
                        upper * scale,
                        unit.dimension().ok().flatten(),
                        exact,
                    )
                } else {
                    Cell::Unreadable(
                        NotEvaluatedReason::InvalidEvidence,
                        format!("is a plain number, not {unit}"),
                    )
                }
            }
            cell => cell,
        }
    }

    /// The aggregates of a column of it whose values are text (`text`) or
    /// numbers.
    fn aggregates(&self, text: bool) -> Vec<Aggregate> {
        if text && self.source.expands() {
            return vec![Aggregate::Values];
        }
        match (&self.aggregates, &self.source) {
            (Some(declared), _) => declared.clone(),
            (None, Source::Property(_) | Source::BoundaryArea(_) | Source::Computed { .. }) => {
                vec![Aggregate::Sum]
            }
            (None, _) => vec![Aggregate::Values],
        }
    }

    /// The column name of `part` of an expanding column.
    fn part_name(&self, part: &str) -> String {
        let part = match &self.source {
            Source::PropertySet(_) => column_name(part),
            _ => part.to_owned(),
        };
        if self.name.is_empty() {
            part
        } else {
            format!("{}_{part}", self.name)
        }
    }
}

struct Declaration<'a> {
    /// Group column ids, one per level.
    group_ids: Vec<String>,
    levels: Vec<CategoryLevel>,
    measures: Vec<Measure<'a>>,
    across_sources: bool,
    /// How far from a face plane a boundary surface may lie, in metres.
    plane_tolerance: f64,
}

/// A value's kind: a dimension, or `None` for a plain number.
type Kind = Option<QuantityDimension>;

/// One member's value of one column.
#[derive(Clone, Debug)]
enum Cell {
    /// A number or quantity in `lower..=upper`, read exactly or measured.
    Number {
        lower: f64,
        upper: f64,
        kind: Kind,
        exact: bool,
    },
    /// Distinct texts, sorted.
    Text(Vec<String>),
    /// Nothing stated.
    Absent,
    /// It cannot be read: why, worded after what was read
    /// (`cannot be read (…)`).
    Unreadable(NotEvaluatedReason, String),
}

impl Cell {
    fn number(lower: f64, upper: f64, kind: Kind, exact: bool) -> Self {
        if lower.is_finite() && upper.is_finite() && lower <= upper {
            Self::Number {
                lower,
                upper,
                kind,
                exact,
            }
        } else {
            Self::Unreadable(NotEvaluatedReason::InvalidEvidence, "is not finite".into())
        }
    }

    /// The texts `values` lists for it.
    fn texts(&self) -> Vec<String> {
        match self {
            Self::Number {
                lower, upper, kind, ..
            } => vec![number_text(*lower, *upper, *kind)],
            Self::Text(texts) => texts.clone(),
            Self::Absent | Self::Unreadable(..) => Vec::new(),
        }
    }
}

/// What a member read for one column.
enum Read {
    /// One value.
    Single(Cell),
    /// One value per part found (an expanding column), or why none could
    /// be read.
    Parts(Result<BTreeMap<String, Cell>, Unavailable>),
}

/// One object the selection may hold.
struct Member {
    object: ObjectId,
    /// Its group, or `None` when it cannot be read.
    group: Option<Vec<String>>,
    /// Whether the selection surely holds it.
    certain: bool,
    /// What leaves its membership, its group or a value open.
    problems: Vec<(NotEvaluatedReason, String)>,
    /// One read per measure.
    reads: Vec<Read>,
}

/// One column of values before aggregation: a measure, or one part of an
/// expanding measure.
struct Field {
    measure: usize,
    part: Option<String>,
    /// The column name after the aggregate.
    name: String,
    /// How messages name the values.
    what: String,
    aggregates: Vec<Aggregate>,
    /// The numeric kind, or `None` when values disagree.
    kind: Option<Kind>,
    /// The unit of a number column counting a currency (`EUR`).
    label: Option<String>,
    exactness: ColumnExactness,
}

impl Member {
    fn cell(&self, field: &Field) -> Cell {
        match (&self.reads[field.measure], &field.part) {
            (Read::Single(cell), _) => cell.clone(),
            (Read::Parts(Ok(parts)), Some(part)) => {
                parts.get(part).cloned().unwrap_or(Cell::Absent)
            }
            (Read::Parts(Err((reason, message))), _) => {
                Cell::Unreadable(reason.clone(), format!("cannot be read ({message})"))
            }
            (Read::Parts(Ok(_)), None) => Cell::Absent,
        }
    }

    /// Reports the member not evaluated with every reason it is open:
    /// membership, group, and each value an aggregate of its group cannot
    /// do without.
    fn report(&self, fields: &[Field], evaluation: &mut CapabilityEvaluation) {
        let mut problems = self.problems.clone();
        for field in fields {
            let cell = self.cell(field);
            let numeric = field.kind.is_some()
                && field.aggregates.iter().any(|aggregate| aggregate.numeric());
            let why = match cell {
                Cell::Unreadable(reason, why) => Some((reason, why)),
                Cell::Absent if numeric => Some((
                    NotEvaluatedReason::IncompleteEvidence,
                    "has no value".into(),
                )),
                Cell::Text(texts) if numeric => Some((
                    NotEvaluatedReason::InvalidEvidence,
                    format!("states `{}`, not a number or quantity", texts.join(", ")),
                )),
                _ => None,
            };
            if let Some((reason, why)) = why {
                problems.push((
                    reason,
                    format!(
                        "`{}` {why}, so its group's `{}` is unknown",
                        field.what, field.name
                    ),
                ));
            }
        }
        if let Some((reason, _)) = problems.first() {
            let reason = reason.clone();
            let message = problems
                .into_iter()
                .map(|(_, message)| message)
                .collect::<Vec<_>>()
                .join("; ");
            evaluation.push_object_not_evaluated(
                self.object.clone(),
                reason,
                format!("quantity-takeoff: {message}"),
            );
        }
    }
}

impl<'a> Declaration<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let mut group_ids = Vec::new();
        let mut levels = Vec::new();
        for n in 1..=GROUPS {
            let key = format!("group_{n}");
            let property = parameters.property(&key)?;
            let path = parameters.strings(&format!("{key}_path"))?;
            let name = parameters.string(&format!("{key}_name"))?;
            let Some(property) = property else {
                if path.is_some() || name.is_some() {
                    return Err(invalid(format!(
                        "`{key}_path` or `{key}_name` without `{key}`"
                    )));
                }
                continue;
            };
            if levels.len() + 1 != n {
                return Err(invalid(format!(
                    "`{key}` is declared without `group_{}`",
                    levels.len() + 1
                )));
            }
            let path = match path {
                None => Vec::new(),
                Some(path) => {
                    Traversal::path(path).map_err(|(reason, message)| {
                        (reason, format!("`{key}_path`: {message}"))
                    })?;
                    path.to_vec()
                }
            };
            group_ids.push(name.map_or(key, str::to_owned));
            levels.push(CategoryLevel {
                property_set: property.set.map(str::to_owned),
                property: property.name.to_owned(),
                path,
            });
        }
        let measures = bind_computed(measures(&parameters)?)?;
        let plane_tolerance = match parameters.quantity("boundary_plane_tolerance")? {
            None => 0.0,
            Some((value, QuantityDimension::Length)) if value >= 0.0 => value,
            Some(_) => {
                return Err(invalid(
                    "`boundary_plane_tolerance` is not a non-negative length",
                ));
            }
        };
        let declaration = Self {
            group_ids,
            levels,
            measures,
            across_sources: parameters.boolean("across_sources")?.unwrap_or(false),
            plane_tolerance,
        };
        // Names, lengths and clashes of the columns known before any value
        // is read are refused up front.
        let fields: Vec<Field> = declaration
            .measures
            .iter()
            .enumerate()
            .filter(|(_, measure)| !measure.source.expands())
            .map(|(index, measure)| Field {
                measure: index,
                part: None,
                name: measure.name.clone(),
                what: measure.what.clone(),
                aggregates: measure.aggregates(false),
                kind: None,
                label: currency_label(measure),
                exactness: ColumnExactness::Exact,
            })
            .collect();
        declaration.empty_table(rule, &fields).map_err(|error| {
            invalid(format!(
                "{error}; name the columns with `group_<n>_name` or `measure_<n>_name`"
            ))
        })?;
        Ok(declaration)
    }

    /// The member `object` may be, if the selection may hold it, with
    /// every value it reads.
    fn member(
        &self,
        context: &RuleContext<'_>,
        rule: &CompiledRule,
        object: &Object,
        universe: &[&Object],
    ) -> Option<Member> {
        let mut evidence = Vec::new();
        let mut problems: Vec<(NotEvaluatedReason, String)> = Vec::new();
        let certain = match selector_matches(context, &rule.selector, object, &mut evidence) {
            Selection::NoMatch => return None,
            Selection::Match => true,
            Selection::NotEvaluated(reason, message) => {
                problems.push((
                    reason,
                    format!("whether it is selected is undecided ({message}), so it may or may not count"),
                ));
                false
            }
        };
        let group = match category_headings(context, object, &self.levels) {
            Ok((headings, _)) => Some(headings),
            Err((reason, message)) => {
                problems.push((
                    reason,
                    format!("its group cannot be read ({message}), so it may count in any group"),
                ));
                None
            }
        };
        let mut coverage: Option<Result<BoundaryCoverage, Unavailable>> = None;
        let mut reads: Vec<Read> = self
            .measures
            .iter()
            .map(|measure| match &measure.source {
                Source::Property(property) => {
                    Read::Single(measure.in_unit(property_cell(context, object, *property)))
                }
                Source::Related { property, path } => {
                    let zero = measure
                        .unit
                        .as_ref()
                        .map(|(_, unit)| unit.dimension().ok().flatten());
                    Read::Single(measure.in_unit(related_cell(
                        context, object, *property, path, universe, zero,
                    )))
                }
                Source::BoundaryArea(bounding) => {
                    let coverage = coverage.get_or_insert_with(|| {
                        measure_coverage(context, &object.id, self.plane_tolerance)
                    });
                    let (cell, open) = boundary_cell(context, coverage, bounding);
                    problems.extend(open.into_iter().map(|(reason, message)| {
                        (
                            reason,
                            format!(
                                "{message}, so its `{}` may or may not include it",
                                measure.what
                            ),
                        )
                    }));
                    Read::Single(cell)
                }
                Source::PropertySet(set) => Read::Parts(set_parts(context, object, set)),
                Source::Profile => Read::Parts(profile_parts(context, object)),
                // Computed below, once every column it reads is read.
                Source::Computed { .. } => Read::Single(Cell::Absent),
            })
            .collect();
        for (index, measure) in self.measures.iter().enumerate() {
            if let Source::Computed {
                bound: Some(computed),
                ..
            } = &measure.source
            {
                reads[index] = Read::Single(computed.cell(&reads));
            }
        }
        Some(Member {
            object: object.id.clone(),
            group,
            certain,
            problems,
            reads,
        })
    }

    /// The columns of values: one per measure, and one per part an
    /// expanding measure found on any member, in a stable order. A numeric
    /// column's kind is the one kind its values state, or the measure's
    /// own when none is read; values of several kinds leave it unknown
    /// (`None`) and the rule not evaluated.
    fn fields(&self, members: &[Member], evaluation: &mut CapabilityEvaluation) -> Vec<Field> {
        let mut fields = Vec::new();
        for (index, measure) in self.measures.iter().enumerate() {
            let parts: Vec<Option<String>> = if measure.source.expands() {
                let mut found = BTreeSet::new();
                for member in members {
                    if let Read::Parts(Ok(parts)) = &member.reads[index] {
                        found.extend(parts.keys().cloned());
                    }
                }
                let mut found: Vec<String> = found.into_iter().collect();
                if matches!(measure.source, Source::Profile) {
                    found.sort_by_key(|part| profile_order(part));
                }
                found.into_iter().map(Some).collect()
            } else {
                vec![None]
            };
            for part in parts {
                let (name, what) = match &part {
                    Some(part) => (measure.part_name(part), format!("{} {part}", measure.what)),
                    None => (measure.name.clone(), measure.what.clone()),
                };
                let mut field = Field {
                    measure: index,
                    part,
                    name,
                    what,
                    aggregates: Vec::new(),
                    kind: None,
                    label: currency_label(measure),
                    exactness: ColumnExactness::Exact,
                };
                let cells: Vec<Cell> = members.iter().map(|member| member.cell(&field)).collect();
                let text = cells.iter().any(|cell| matches!(cell, Cell::Text(_)))
                    || (measure.source.expands()
                        && !cells.iter().any(|cell| matches!(cell, Cell::Number { .. }))
                        && !profile_dimension(field.part.as_deref()));
                field.aggregates = measure.aggregates(text);
                let mut kinds: Vec<Kind> = Vec::new();
                let mut measured = false;
                let mut numbers = false;
                for cell in &cells {
                    if let Cell::Number { kind, exact, .. } = cell {
                        numbers = true;
                        measured |= !exact;
                        if !kinds.contains(kind) {
                            kinds.push(*kind);
                        }
                    }
                }
                field.exactness = if measured {
                    ColumnExactness::Bounded
                } else if numbers {
                    ColumnExactness::Exact
                } else {
                    measure.source.fallback_exactness()
                };
                let mut kinds = kinds.into_iter();
                field.kind = if let Some((_, unit)) = &measure.unit {
                    Some(unit.dimension().ok().flatten())
                } else {
                    match (kinds.next(), kinds.next()) {
                        (None, _) => Some(match &measure.source {
                            Source::Profile => Some(profile_kind(field.part.as_deref())),
                            source => source.fallback_kind(),
                        }),
                        (Some(kind), None) => Some(kind),
                        (Some(first), Some(second)) => {
                            if field.aggregates.iter().any(|aggregate| aggregate.numeric()) {
                                evaluation.push_not_evaluated(
                                NotEvaluatedReason::InvalidEvidence,
                                format!(
                                    "quantity-takeoff: `{}` is stated as {} and as {}, so no `{}` is aggregated",
                                    field.what,
                                    kind_text(first),
                                    kind_text(second),
                                    field.name
                                ),
                            );
                            }
                            None
                        }
                    }
                };
                fields.push(field);
            }
        }
        fields
    }

    fn empty_table(
        &self,
        rule: &CompiledRule,
        fields: &[Field],
    ) -> Result<ReportTable, axioval_ir::ReportTableError> {
        let mut columns =
            vec![ReportColumn::number("count").with_exactness(ColumnExactness::Exact)];
        for field in fields {
            for aggregate in &field.aggregates {
                let id = format!("{}_{}", aggregate.name(), field.name);
                let column = match (aggregate, field.kind.flatten()) {
                    (Aggregate::Values, _) => ReportColumn::text(id),
                    (_, Some(dimension)) => ReportColumn::quantity(id, dimension),
                    (_, None) => match &field.label {
                        Some(unit) => ReportColumn::amount(id, unit),
                        None => ReportColumn::number(id),
                    },
                };
                columns.push(column.with_exactness(field.exactness));
            }
        }
        ReportTable::grouped(
            rule.id.clone(),
            TAKEOFF_TABLE,
            self.group_ids.clone(),
            columns,
        )
    }

    /// One row per group of each scope.
    fn table(
        &self,
        rule: &CompiledRule,
        fields: &[Field],
        members: &[Member],
        scopes: &BTreeMap<Scope, Vec<usize>>,
    ) -> Result<ReportTable, axioval_ir::ReportTableError> {
        let mut table = self.empty_table(rule, fields)?;
        for (scope, indices) in scopes {
            let members: Vec<&Member> = indices.iter().map(|&index| &members[index]).collect();
            let groups: BTreeSet<&Vec<String>> = members
                .iter()
                .filter_map(|member| member.group.as_ref())
                .collect();
            for group in groups {
                let (sure, maybe): (Vec<&Member>, Vec<&Member>) = members
                    .iter()
                    .filter(|member| member.group.as_ref().is_none_or(|own| own == group))
                    .partition(|member| member.certain && member.group.is_some());
                #[allow(clippy::cast_precision_loss)]
                let mut values = vec![ReportValue::measured(
                    sure.len() as f64,
                    (sure.len() + maybe.len()) as f64,
                )];
                for field in fields {
                    let sure_cells: Vec<Cell> =
                        sure.iter().map(|member| member.cell(field)).collect();
                    let maybe_cells: Vec<Cell> =
                        maybe.iter().map(|member| member.cell(field)).collect();
                    let read = |cells: &[Cell]| -> Option<Vec<(f64, f64)>> {
                        cells
                            .iter()
                            .map(|cell| match cell {
                                Cell::Number { lower, upper, .. } => Some((*lower, *upper)),
                                _ => None,
                            })
                            .collect()
                    };
                    let bounds = field.kind.and(read(&sure_cells)).zip(read(&maybe_cells));
                    for aggregate in &field.aggregates {
                        values.push(match (aggregate, &bounds) {
                            (Aggregate::Values, _) => listed(&sure_cells, &maybe_cells),
                            (_, Some((sure, maybe))) => aggregated(*aggregate, sure, maybe),
                            (_, None) => ReportValue::Unknown,
                        });
                    }
                }
                table.push_group_row(scope.clone(), group.clone(), values)?;
            }
        }
        Ok(table)
    }
}

/// The declared measures, `measure_1` onwards.
fn measures<'a>(parameters: &Parameters<'a>) -> Result<Vec<Measure<'a>>, Unavailable> {
    let mut measures = Vec::new();
    for n in 1..=MEASURES {
        let key = format!("measure_{n}");
        let property = parameters.property(&key)?;
        let aggregates = parameters.strings(&format!("{key}_aggregates"))?;
        let name = parameters.string(&format!("{key}_name"))?;
        let kind = parameters.string(&format!("{key}_kind"))?;
        let path = parameters.strings(&format!("{key}_path"))?;
        let bounding = parameters.selector(&format!("{key}_bounding"))?;
        let set = parameters.string(&format!("{key}_property_set"))?;
        let expression = parameters.string(&format!("{key}_expression"))?;
        let unit = parameters.string(&format!("{key}_unit"))?;
        if property.is_none() && kind.is_none() {
            if aggregates.is_some()
                || name.is_some()
                || path.is_some()
                || bounding.is_some()
                || set.is_some()
                || expression.is_some()
                || unit.is_some()
            {
                return Err(invalid(format!(
                    "a parameter of `{key}` is declared without `{key}` or `{key}_kind`"
                )));
            }
            continue;
        }
        if measures.len() + 1 != n {
            return Err(invalid(format!(
                "`{key}` is declared without `measure_{}`",
                measures.len() + 1
            )));
        }
        let source = source(
            &key,
            kind.unwrap_or("property"),
            Declared {
                property,
                path,
                bounding,
                set,
                expression,
                unit,
            },
        )?;
        let unit = match unit {
            None => None,
            Some(text) => {
                let (scale, unit) = expression::parse_unit(text)
                    .map_err(|why| invalid(format!("`{key}_unit`: {why}")))?;
                unit.dimension()
                    .map_err(|why| invalid(format!("`{key}_unit`: {why}")))?;
                Some((scale, unit))
            }
        };
        let aggregates = aggregates
            .map(|names| parse_aggregates(&key, names))
            .transpose()?;
        let (default_name, what) = match &source {
            Source::Property(property) => (column_name(property.name), property.to_string()),
            Source::Related { property, path } => (
                column_name(property.name),
                format!("{property} via {}", path.relationship),
            ),
            Source::BoundaryArea(_) => ("boundary_area".to_owned(), "boundary area".to_owned()),
            Source::PropertySet(set) => (String::new(), (*set).to_owned()),
            Source::Profile => ("profile".to_owned(), "profile".to_owned()),
            Source::Computed { text, .. } => (String::new(), (*text).to_owned()),
        };
        let name = name.map_or(default_name, str::to_owned);
        if name.is_empty() && !matches!(source, Source::PropertySet(_)) {
            return Err(invalid(format!(
                "`{key}` gives no column name; declare `{key}_name`"
            )));
        }
        measures.push(Measure {
            source,
            aggregates,
            name,
            what,
            unit,
        });
    }
    Ok(measures)
}

/// Binds every computed column's expression, now that every column is
/// declared: its names resolve to the other columns (a computed one only
/// before it), whose units must be known, and its units must agree.
fn bind_computed(mut measures: Vec<Measure<'_>>) -> Result<Vec<Measure<'_>>, Unavailable> {
    for at in 0..measures.len() {
        let Source::Computed { text, .. } = &measures[at].source else {
            continue;
        };
        let text = *text;
        let key = format!("measure_{}_expression", at + 1);
        let mut inputs: Vec<Input> = Vec::new();
        let bound = {
            let declared = &measures;
            let mut resolve = |name: &str| -> Result<(usize, Unit), String> {
                if let Some(index) = inputs.iter().position(|input| input.name == name) {
                    return Ok((index, inputs[index].unit.clone()));
                }
                let (measure, part, unit) = input_of(declared, at, name)?;
                inputs.push(Input {
                    measure,
                    part,
                    unit: unit.clone(),
                    name: name.to_owned(),
                });
                Ok((inputs.len() - 1, unit))
            };
            expression::bind(text, &mut resolve)
        }
        .map_err(|why| invalid(format!("`{key}` `{text}`: {why}")))?;
        bound
            .unit()
            .dimension()
            .map_err(|why| invalid(format!("`{key}` `{text}`: {why}")))?;
        measures[at].unit = Some((1.0, bound.unit().clone()));
        measures[at].source = Source::Computed {
            text,
            bound: Some(Box::new(Computed {
                expression: bound,
                inputs,
            })),
        };
    }
    Ok(measures)
}

/// The column `name` names for the computed column `at`: a measure and,
/// for a profile's dimension (`profile_width`), the part, with the unit
/// its values are stated in.
fn input_of(
    measures: &[Measure<'_>],
    at: usize,
    name: &str,
) -> Result<(usize, Option<String>, Unit), String> {
    if measures[at].name == name {
        return Err(format!("`{name}` is the computed column itself"));
    }
    for (index, measure) in measures.iter().enumerate() {
        if index == at {
            continue;
        }
        if let Source::Profile = measure.source {
            let dimension = name
                .strip_prefix(measure.name.as_str())
                .and_then(|rest| rest.strip_prefix('_'))
                .and_then(|part| dimension_columns().find(|(column, _)| *column == part));
            if let Some((column, _)) = dimension {
                let unit = Unit::of(Some(profile_kind(Some(column))));
                return Ok((index, Some(column.to_owned()), unit));
            }
            continue;
        }
        if measure.name != name {
            continue;
        }
        let unit = match (&measure.source, &measure.unit) {
            (Source::PropertySet(_), _) => {
                return Err(format!(
                    "`{name}` expands into one column per property, which an expression cannot name"
                ));
            }
            (Source::Computed { .. }, _) if index > at => {
                return Err(format!("`{name}` is computed after this column"));
            }
            (_, Some((_, unit))) => unit.clone(),
            (Source::BoundaryArea(_), None) => Unit::of(Some(QuantityDimension::Area)),
            (Source::Property(property), None) if property.set == Some(MEASURED_SET) => {
                Unit::of(measured_kind(*property))
            }
            _ => {
                return Err(format!(
                    "the unit of `{name}` is unknown; declare `measure_{}_unit`",
                    index + 1
                ));
            }
        };
        return Ok((index, None, unit));
    }
    Err(format!("`{name}` names no column"))
}

/// The unit a number column of `measure` names: its own where it counts a
/// currency, which no dimension does.
fn currency_label(measure: &Measure<'_>) -> Option<String> {
    measure
        .unit
        .as_ref()
        .filter(|(_, unit)| unit.counts_currency())
        .map(|(_, unit)| unit.to_string())
}

/// What a column declares beside its kind.
#[derive(Clone, Copy)]
struct Declared<'a> {
    property: Option<PropertyRef<'a>>,
    path: Option<&'a [String]>,
    bounding: Option<&'a Selector>,
    set: Option<&'a str>,
    expression: Option<&'a str>,
    unit: Option<&'a str>,
}

/// Where the column `key` of `kind` takes its values from, refusing a
/// parameter the kind requires and lacks or does not take.
fn source<'a>(key: &str, kind: &str, declared: Declared<'a>) -> Result<Source<'a>, Unavailable> {
    let Declared {
        property,
        path,
        bounding,
        set,
        expression,
        unit,
    } = declared;
    let unused = |what: &str, present: bool| -> Result<(), Unavailable> {
        if present {
            Err(invalid(format!(
                "`{key}{what}` does not apply to a `{kind}` column"
            )))
        } else {
            Ok(())
        }
    };
    let required = |what: &str| invalid(format!("a `{kind}` column requires `{key}{what}`"));
    if kind != "computed" {
        unused("_expression", expression.is_some())?;
    }
    if !matches!(kind, "property" | "related") {
        unused("_unit", unit.is_some())?;
    }
    Ok(match kind {
        "property" | "related" => {
            let property = property.ok_or_else(|| required(""))?;
            unused("_bounding", bounding.is_some())?;
            unused("_property_set", set.is_some())?;
            if kind == "property" {
                unused("_path", path.is_some())?;
                Source::Property(property)
            } else {
                let path = path.ok_or_else(|| required("_path"))?;
                let path = Traversal::path(path)
                    .map_err(|(reason, message)| (reason, format!("`{key}_path`: {message}")))?;
                Source::Related { property, path }
            }
        }
        "boundary_area" => {
            unused("", property.is_some())?;
            unused("_path", path.is_some())?;
            unused("_property_set", set.is_some())?;
            Source::BoundaryArea(bounding.ok_or_else(|| required("_bounding"))?)
        }
        "property_set" => {
            unused("", property.is_some())?;
            unused("_path", path.is_some())?;
            unused("_bounding", bounding.is_some())?;
            let set = set.ok_or_else(|| required("_property_set"))?;
            if set.trim().is_empty() {
                return Err(invalid(format!("`{key}_property_set` is blank")));
            }
            Source::PropertySet(set)
        }
        "profile" => {
            unused("", property.is_some())?;
            unused("_path", path.is_some())?;
            unused("_bounding", bounding.is_some())?;
            unused("_property_set", set.is_some())?;
            Source::Profile
        }
        "computed" => {
            unused("", property.is_some())?;
            unused("_path", path.is_some())?;
            unused("_bounding", bounding.is_some())?;
            unused("_property_set", set.is_some())?;
            Source::Computed {
                text: expression.ok_or_else(|| required("_expression"))?,
                bound: None,
            }
        }
        other => {
            return Err(invalid(format!(
                "`{key}_kind` `{other}` is unsupported; use {KINDS}"
            )));
        }
    })
}

/// The declared aggregates of the column `key`, each once.
fn parse_aggregates(key: &str, names: &[String]) -> Result<Vec<Aggregate>, Unavailable> {
    if names.is_empty() {
        return Err(invalid(format!("`{key}_aggregates` is empty")));
    }
    let parsed = names
        .iter()
        .map(|name| Aggregate::parse(name.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    if parsed.iter().collect::<BTreeSet<_>>().len() != parsed.len() {
        return Err(invalid(format!("`{key}_aggregates` repeats an aggregate")));
    }
    Ok(parsed)
}

/// A property of `object` as a cell.
fn property_cell(context: &RuleContext<'_>, object: &Object, property: PropertyRef<'_>) -> Cell {
    match resolve(context, object, property) {
        Ok(resolved) => value_cell(resolved.value()),
        Err((reason, message)) => Cell::Unreadable(reason, format!("cannot be read ({message})")),
    }
}

/// A value as a cell: a number or quantity, text (a value of another type
/// as messages show it), or nothing stated.
fn value_cell(value: Option<&PropertyValue>) -> Cell {
    match value {
        value if undefined(value) => Cell::Absent,
        Some(PropertyValue::Integer(value)) => match exact_f64(*value) {
            Some(value) => Cell::number(value, value, None, true),
            None => Cell::Unreadable(
                NotEvaluatedReason::InvalidEvidence,
                "is too large to add exactly".into(),
            ),
        },
        Some(PropertyValue::Decimal(value)) => Cell::number(*value, *value, None, true),
        Some(PropertyValue::Quantity { value, dimension }) => {
            Cell::number(*value, *value, Some(*dimension), true)
        }
        Some(PropertyValue::Measured {
            lower,
            upper,
            dimension,
        }) => Cell::number(*lower, *upper, Some(*dimension), false),
        Some(PropertyValue::String(text)) => Cell::Text(vec![text.trim().to_owned()]),
        value => Cell::Text(vec![display(value)]),
    }
}

/// `property` on the objects `path` reaches from `object`: numbers of one
/// kind summed (every reached object must state one), texts listed; none
/// reached or stated is no value. With `zero` (a declared unit's kind),
/// reaching no object at all is an exact zero: the sum over nothing.
fn related_cell(
    context: &RuleContext<'_>,
    object: &Object,
    property: PropertyRef<'_>,
    path: &Traversal,
    universe: &[&Object],
    zero: Option<Kind>,
) -> Cell {
    let reached = match path.related(context, &object.id, universe) {
        Ok((reached, _)) => reached,
        Err((reason, message)) => {
            return Cell::Unreadable(reason, format!("cannot be followed ({message})"));
        }
    };
    if let (true, Some(kind)) = (reached.is_empty(), zero) {
        return Cell::number(0.0, 0.0, kind, true);
    }
    let mut numbers = Vec::new();
    let mut texts = BTreeSet::new();
    let mut without = Vec::new();
    for id in &reached {
        let Some(holder) = context.project.object(id) else {
            continue;
        };
        match property_cell(context, holder, property) {
            Cell::Unreadable(reason, why) => {
                return Cell::Unreadable(reason, format!("{why} on {id}"));
            }
            Cell::Absent => without.push(id),
            Cell::Text(found) => texts.extend(found),
            number @ Cell::Number { .. } => numbers.push(number),
        }
    }
    if !texts.is_empty() {
        texts.extend(numbers.iter().flat_map(Cell::texts));
        return Cell::Text(texts.into_iter().collect());
    }
    if numbers.is_empty() {
        return Cell::Absent;
    }
    if let Some(id) = without.first() {
        return Cell::Unreadable(
            NotEvaluatedReason::IncompleteEvidence,
            format!("has no value on {id}, so the values reached cannot be added"),
        );
    }
    let (mut lower, mut upper, mut all_exact) = (0.0, 0.0, true);
    let mut kinds = BTreeSet::new();
    for number in &numbers {
        if let Cell::Number {
            lower: low,
            upper: high,
            kind,
            exact,
        } = number
        {
            lower += low;
            upper += high;
            all_exact &= exact;
            kinds.insert(kind_text(*kind));
        }
    }
    let kind = match numbers.first() {
        Some(Cell::Number { kind, .. }) if kinds.len() == 1 => *kind,
        _ => {
            return Cell::Unreadable(
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "is stated as {} on the objects reached, so they cannot be added",
                    kinds.into_iter().collect::<Vec<_>>().join(" and as ")
                ),
            );
        }
    };
    Cell::number(lower, upper, kind, all_exact)
}

/// The boundary coverage of `space`, as the coverage service measures it.
fn measure_coverage(
    context: &RuleContext<'_>,
    space: &ObjectId,
    plane_tolerance: f64,
) -> Result<BoundaryCoverage, Unavailable> {
    let Some(service) = context.services.get::<BoundaryCoverageServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "space-boundary coverage service is not registered".into(),
        ));
    };
    BoundaryCoverageRequest::try_new(space.clone(), plane_tolerance)
        .and_then(|request| service.measure_boundary_coverage(&request))
        .map_err(|error| coverage_error(&error))
}

/// The area of the space's declared boundaries whose bounding element
/// `bounding` selects, and why a boundary may or may not count. A boundary
/// whose element's selection is undecided may add its area; one of the
/// kind lying on no face of the space's body leaves the area unknown.
fn boundary_cell(
    context: &RuleContext<'_>,
    coverage: &Result<BoundaryCoverage, Unavailable>,
    bounding: &Selector,
) -> (Cell, Vec<(NotEvaluatedReason, String)>) {
    let coverage = match coverage {
        Ok(coverage) => coverage,
        Err((reason, message)) => {
            return (
                Cell::Unreadable(reason.clone(), format!("cannot be measured ({message})")),
                Vec::new(),
            );
        }
    };
    let (mut lower, mut upper) = (0.0, 0.0);
    let mut open = Vec::new();
    for boundary in coverage.boundaries() {
        let Some(element) = boundary.element() else {
            continue;
        };
        let Some(object) = object_by_id(context, element) else {
            return (
                Cell::Unreadable(
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "cannot be measured: boundary {} bounds against {element}, which is not in the model",
                        boundary.boundary()
                    ),
                ),
                Vec::new(),
            );
        };
        let sure = match selector_matches(context, bounding, object, &mut Vec::new()) {
            Selection::NoMatch => continue,
            Selection::Match => true,
            Selection::NotEvaluated(reason, message) => {
                open.push((
                    reason,
                    format!(
                        "whether boundary {}'s element {element} is of the kind is undecided ({message})",
                        boundary.boundary()
                    ),
                ));
                false
            }
        };
        let BoundaryPlacement::OnSurface { area } = boundary.placement() else {
            return (
                Cell::Unreadable(
                    NotEvaluatedReason::InvalidEvidence,
                    format!(
                        "cannot be measured: boundary {} lies on no face of the space's body",
                        boundary.boundary()
                    ),
                ),
                Vec::new(),
            );
        };
        if sure {
            lower += area.lower_square_metres();
        }
        upper += area.upper_square_metres();
    }
    let exact = coverage.evidence().exact && open.is_empty();
    (
        Cell::number(lower, upper, Some(QuantityDimension::Area), exact),
        open,
    )
}

/// Every property of `set` on `object`, by its source name.
fn set_parts(
    context: &RuleContext<'_>,
    object: &Object,
    set: &str,
) -> Result<BTreeMap<String, Cell>, Unavailable> {
    let enumeration = enumerate(context, object, NameSpec::Exact(set), NameSpec::Any)?;
    Ok(enumeration
        .properties()
        .iter()
        .map(|property| (property.name.clone(), value_cell(Some(&property.value))))
        .collect())
}

/// The body's swept profile: `type`, `name` and every dimension it states;
/// nothing for a body that is no single swept profile.
fn profile_parts(
    context: &RuleContext<'_>,
    object: &Object,
) -> Result<BTreeMap<String, Cell>, Unavailable> {
    let mut body = BodyFacts::of(context, object)?;
    let Ok(profile) = read_profile(&mut body)? else {
        return Ok(BTreeMap::new());
    };
    let mut parts = BTreeMap::new();
    parts.insert("type".to_owned(), Cell::Text(vec![profile.family.clone()]));
    if let Some(name) = &profile.name {
        parts.insert("name".to_owned(), Cell::Text(vec![name.clone()]));
    }
    for (column, angle) in dimension_columns() {
        let cell = match dimension_value(&profile, column, &mut body) {
            Ok(None) => continue,
            Ok(Some(value)) => {
                let kind = if angle {
                    QuantityDimension::PlaneAngle
                } else {
                    QuantityDimension::Length
                };
                Cell::number(value, value, Some(kind), true)
            }
            Err((reason, message)) => {
                Cell::Unreadable(reason, format!("cannot be read ({message})"))
            }
        };
        parts.insert(column.to_owned(), cell);
    }
    Ok(parts)
}

/// A profile part's place: type, name, then the dimensions in their order.
fn profile_order(part: &str) -> usize {
    match part {
        "type" => 0,
        "name" => 1,
        part => {
            2 + dimension_columns()
                .position(|(column, _)| column == part)
                .unwrap_or(usize::MAX - 2)
        }
    }
}

/// Whether a profile part is a dimension.
fn profile_dimension(part: Option<&str>) -> bool {
    part.is_some_and(|part| dimension_columns().any(|(column, _)| column == part))
}

/// The kind of a profile part: a length, or a plane angle.
fn profile_kind(part: Option<&str>) -> QuantityDimension {
    let angle =
        part.is_some_and(|part| dimension_columns().any(|(column, angle)| column == part && angle));
    if angle {
        QuantityDimension::PlaneAngle
    } else {
        QuantityDimension::Length
    }
}

/// The distinct values of a group's members, listed: every value of a sure
/// member, and a possible member's only where it is listed already, since
/// whether it adds one is undecided; `-` for none. Unknown when a value
/// cannot be read.
fn listed(sure: &[Cell], maybe: &[Cell]) -> ReportValue {
    let mut texts = BTreeSet::new();
    for cell in sure {
        if matches!(cell, Cell::Unreadable(..)) {
            return ReportValue::Unknown;
        }
        texts.extend(cell.texts());
    }
    for cell in maybe {
        if matches!(cell, Cell::Unreadable(..))
            || cell.texts().iter().any(|text| !texts.contains(text))
        {
            return ReportValue::Unknown;
        }
    }
    if texts.is_empty() {
        ReportValue::text("-")
    } else {
        ReportValue::text(texts.into_iter().collect::<Vec<_>>().join(", "))
    }
}

/// A number as `values` lists it: `0.3 m`, `9.5..10.5 m²`.
fn number_text(lower: f64, upper: f64, kind: Kind) -> String {
    #[allow(clippy::float_cmp)]
    let number = if lower == upper {
        lower.to_string()
    } else {
        format!("{lower}..{upper}")
    };
    match kind {
        Some(dimension) => format!("{number} {}", dimension.unit_symbol()),
        None => number,
    }
}

/// `values` combined by `aggregate`, where `sure` surely belong to the group
/// and `maybe` may: an interval sure to hold the aggregate of every
/// membership and every value the intervals allow.
fn aggregated(aggregate: Aggregate, sure: &[(f64, f64)], maybe: &[(f64, f64)]) -> ReportValue {
    match aggregate {
        Aggregate::Sum => {
            let lower = sure.iter().map(|value| value.0).sum::<f64>()
                + maybe.iter().map(|value| value.0.min(0.0)).sum::<f64>();
            let upper = sure.iter().map(|value| value.1).sum::<f64>()
                + maybe.iter().map(|value| value.1.max(0.0)).sum::<f64>();
            ReportValue::measured(lower, upper)
        }
        // Values are listed, never aggregated as numbers.
        Aggregate::Values => ReportValue::Unknown,
        // With no sure member the group may hold no value at all.
        _ if sure.is_empty() => ReportValue::Unknown,
        Aggregate::Min => {
            let lower = sure
                .iter()
                .chain(maybe)
                .map(|value| value.0)
                .fold(f64::INFINITY, f64::min);
            let upper = sure
                .iter()
                .map(|value| value.1)
                .fold(f64::INFINITY, f64::min);
            ReportValue::measured(lower, upper)
        }
        Aggregate::Max => {
            let lower = sure
                .iter()
                .map(|value| value.0)
                .fold(f64::NEG_INFINITY, f64::max);
            let upper = sure
                .iter()
                .chain(maybe)
                .map(|value| value.1)
                .fold(f64::NEG_INFINITY, f64::max);
            ReportValue::measured(lower, upper)
        }
        Aggregate::Mean => {
            let lower = extreme_mean(
                sure.iter().map(|value| value.0),
                maybe.iter().map(|value| value.0),
                true,
            );
            let upper = extreme_mean(
                sure.iter().map(|value| value.1),
                maybe.iter().map(|value| value.1),
                false,
            );
            ReportValue::measured(lower, upper)
        }
    }
}

/// The least (`least`) or greatest mean of every `sure` value together
/// with any choice of `maybe` values: the optional values below (above)
/// the running mean, smallest (largest) first, each lowers (raises) it.
fn extreme_mean(
    sure: impl Iterator<Item = f64>,
    maybe: impl Iterator<Item = f64>,
    least: bool,
) -> f64 {
    let (mut total, mut count) = sure.fold((0.0, 0.0), |(total, count), value| {
        (total + value, count + 1.0)
    });
    let mut maybe: Vec<f64> = maybe.collect();
    maybe.sort_by(f64::total_cmp);
    if !least {
        maybe.reverse();
    }
    for value in maybe {
        let mean = total / count;
        if (least && value < mean) || (!least && value > mean) {
            total += value;
            count += 1.0;
        } else {
            break;
        }
    }
    total / count
}

/// The kind a measured name answers in, a length unless an area or volume;
/// a plain number for any other property.
fn measured_kind(property: PropertyRef<'_>) -> Kind {
    if property.set != Some(MEASURED_SET) {
        return None;
    }
    let name = property.name.to_ascii_lowercase();
    Some(match name.as_str() {
        MEASURED_AREA => QuantityDimension::Area,
        MEASURED_VOLUME => QuantityDimension::Volume,
        _ => QuantityDimension::Length,
    })
}

fn kind_text(kind: Kind) -> String {
    match kind {
        Some(dimension) => format!("a quantity in {}", dimension.unit_symbol()),
        None => "a plain number".to_owned(),
    }
}

/// A property name as a column name: the part after its last `.`, words
/// split at case changes and joined by `_`, lowercase (`t.NetSideArea` is
/// `net_side_area`).
fn column_name(property: &str) -> String {
    let base = property.rsplit('.').next().unwrap_or(property);
    let mut name = String::new();
    let mut after_lower = false;
    for character in base.chars() {
        if character.is_ascii_alphanumeric() {
            if character.is_ascii_uppercase() && after_lower {
                name.push('_');
            }
            after_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
            name.push(character.to_ascii_lowercase());
        } else {
            if !name.is_empty() && !name.ends_with('_') {
                name.push('_');
            }
            after_lower = false;
        }
    }
    name.trim_end_matches('_').to_owned()
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::{Aggregate, Cell, aggregated, column_name, listed};
    use axioval_ir::{QuantityDimension, ReportValue};

    #[test]
    fn column_names_follow_the_property_name() {
        assert_eq!(column_name("t.NetSideArea"), "net_side_area");
        assert_eq!(column_name("extent_z"), "extent_z");
        assert_eq!(column_name("Qto_WallBaseQuantities.Length"), "length");
        assert_eq!(column_name("GrossArea2"), "gross_area2");
        assert_eq!(column_name("Fläche"), "fl_che");
        assert_eq!(column_name("ÄÖ"), "");
    }

    #[test]
    fn sums_widen_by_what_possible_members_may_add() {
        let sure = [(10.0, 10.0), (12.0, 13.0)];
        assert_eq!(
            aggregated(Aggregate::Sum, &sure, &[]),
            ReportValue::measured(22.0, 23.0)
        );
        // A possible member adds nothing or its value.
        assert_eq!(
            aggregated(Aggregate::Sum, &sure, &[(5.0, 6.0)]),
            ReportValue::measured(22.0, 29.0)
        );
        assert_eq!(
            aggregated(Aggregate::Sum, &sure, &[(-2.0, 1.0)]),
            ReportValue::measured(20.0, 24.0)
        );
        assert_eq!(
            aggregated(Aggregate::Sum, &[], &[(5.0, 5.0)]),
            ReportValue::measured(0.0, 5.0)
        );
    }

    #[test]
    fn extremes_and_means_bound_every_membership() {
        let sure = [(4.0, 4.0), (6.0, 6.0)];
        let maybe = [(1.0, 1.0), (9.0, 9.0), (5.0, 5.0)];
        assert_eq!(
            aggregated(Aggregate::Min, &sure, &maybe),
            ReportValue::measured(1.0, 4.0)
        );
        assert_eq!(
            aggregated(Aggregate::Max, &sure, &maybe),
            ReportValue::measured(6.0, 9.0)
        );
        // Least mean: 4, 6 and 1 (11 / 3); greatest: 4, 6 and 9 (19 / 3).
        assert_eq!(
            aggregated(Aggregate::Mean, &sure, &maybe),
            ReportValue::measured(11.0 / 3.0, 19.0 / 3.0)
        );
        assert_eq!(
            aggregated(Aggregate::Mean, &sure, &[]),
            ReportValue::exact(5.0)
        );
        // A group that may be empty has no sure extreme or mean.
        for aggregate in [Aggregate::Min, Aggregate::Max, Aggregate::Mean] {
            assert_eq!(aggregated(aggregate, &[], &maybe), ReportValue::Unknown);
        }
    }

    #[test]
    fn listed_values_are_sure_only_where_a_possible_member_adds_none() {
        let text = |value: &str| Cell::Text(vec![value.to_owned()]);
        let length = Cell::number(0.3, 0.3, Some(QuantityDimension::Length), true);
        assert_eq!(
            listed(&[text("EG"), text("OG"), text("EG")], &[]),
            ReportValue::text("EG, OG")
        );
        assert_eq!(
            listed(&[length.clone(), Cell::Absent], &[]),
            ReportValue::text("0.3 m")
        );
        assert_eq!(listed(&[Cell::Absent], &[]), ReportValue::text("-"));
        // A possible member stating a listed value, or none, changes nothing.
        assert_eq!(
            listed(&[text("EG")], &[text("EG"), Cell::Absent]),
            ReportValue::text("EG")
        );
        // One that may add a value leaves the list open.
        assert_eq!(listed(&[text("EG")], &[text("OG")]), ReportValue::Unknown);
        assert_eq!(
            listed(
                &[text("EG")],
                &[Cell::Unreadable(
                    axioval_ir::NotEvaluatedReason::IncompleteEvidence,
                    String::new()
                )]
            ),
            ReportValue::Unknown
        );
    }
}
