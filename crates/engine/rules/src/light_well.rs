//! `light-well`: spaces stacked into a light well are contiguous, and their
//! shared plan section is large and wide enough for the well's height.
//!
//! Each selected object is a well; its spaces are the objects `member_path`
//! reaches from it (with IFC, the spaces of a zone through
//! `IfcRelAssignsToGroup:forward`). The well is judged on three things:
//!
//! - **contiguity**: ordered by their bottoms, no member starts more than
//!   `gap_tolerance_metres` above the top of the one below it, and the
//!   members share a plan section (the intersection of their footprints,
//!   `PlanSpanService::measure_section`) that is not empty;
//! - **area**: the section's area;
//! - **width**: the short side of the section's least-area rectangle, the
//!   one `measure_rectangle` answers for a footprint. A section whose
//!   rectangle is tied has no known width, and the service refuses it: the
//!   well is then not evaluated.
//!
//! The well's height runs from its lowest bottom to its highest top. The
//! applicable row of `requirements` is the first whose
//! `maximum_height_metres` the height does not exceed (a row without one
//! holds any height); it requires `minimum_area_square_metres` and
//! `minimum_width_metres`. No row means no requirement. Every value is an
//! interval, and one straddling a bound decides nothing.

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, PlanSection, PlanSpanError, PlanSpanServiceHandle, RuleCapability, RuleContext,
    TableColumn, VerticalExtent,
};
use axioval_ir::{Evidence, Object};

use crate::level_spacing::{extent, extents, shown};
use crate::plan_area::{Verdict, judge};
use crate::selection::select_objects;
use crate::support::table::{Matched, Row, RowSelection, RowTest, match_rows};
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("maximum_height_metres", ColumnKind::Number),
    TableColumn::optional("minimum_area_square_metres", ColumnKind::Number),
    TableColumn::optional("minimum_width_metres", ColumnKind::Number),
];

/// Requires stacked light-well spaces to be contiguous and large enough.
pub struct LightWell;

/// One row of the requirement table.
struct Requirement {
    up_to: Option<f64>,
    area: Option<f64>,
    width: Option<f64>,
}

impl Requirement {
    fn read(row: Row<'_>, index: usize) -> Result<Self, Unavailable> {
        let non_negative = |column: &str| -> Result<Option<f64>, Unavailable> {
            match row.number(column)? {
                Some(value) if value < 0.0 => Err(invalid(format!(
                    "row {index}: `{column}` must not be negative"
                ))),
                other => Ok(other),
            }
        };
        let requirement = Self {
            up_to: non_negative("maximum_height_metres")?,
            area: non_negative("minimum_area_square_metres")?,
            width: non_negative("minimum_width_metres")?,
        };
        if requirement.area.is_none() && requirement.width.is_none() {
            return Err(invalid(format!(
                "row {index} needs `minimum_area_square_metres`, `minimum_width_metres` or both"
            )));
        }
        Ok(requirement)
    }

    fn holds(&self, low: f64, high: f64) -> RowTest {
        match self.up_to {
            None => RowTest::Match(0),
            Some(bound) if high <= bound => RowTest::Match(0),
            Some(bound) if low > bound => RowTest::NoMatch,
            Some(_) => RowTest::Undecided,
        }
    }
}

struct Declaration<'a> {
    members: Traversal<'a>,
    rows: Vec<Requirement>,
    tolerance: f64,
}

fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let path = parameters
        .strings("member_path")?
        .ok_or_else(|| invalid("parameter `member_path` is required"))?;
    let members = Traversal::path(path)?;
    let rows = parameters
        .table("requirements")?
        .ok_or_else(|| invalid("parameter `requirements` is required"))?
        .into_iter()
        .enumerate()
        .map(|(index, row)| Requirement::read(row, index))
        .collect::<Result<Vec<_>, _>>()?;
    let tolerance = match parameters.number("gap_tolerance_metres")? {
        None => 0.0,
        Some(value) if value >= 0.0 => value,
        Some(_) => return Err(invalid("`gap_tolerance_metres` must not be negative")),
    };
    Ok(Declaration {
        members,
        rows,
        tolerance,
    })
}

fn section_unavailable(error: &PlanSpanError) -> Unavailable {
    let reason = match error {
        PlanSpanError::UnknownObject(_) | PlanSpanError::Unavailable(_) => {
            NotEvaluatedReason::IncompleteEvidence
        }
        PlanSpanError::InvalidMeasurement | PlanSpanError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (
        reason,
        format!("the shared plan section cannot be measured: {error}"),
    )
}

/// What one well's judgement found.
struct Judged {
    findings: Vec<(String, Vec<Evidence>)>,
    undecided: Vec<String>,
    members: Vec<axioval_ir::ObjectId>,
}

impl RuleCapability for LightWell {
    fn id(&self) -> &'static str {
        "axioval:capability.light-well"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("member_path", ParameterType::StringList),
            ParameterDescriptor::required("requirements", ParameterType::Table(COLUMNS)),
            ParameterDescriptor::optional("gap_tolerance_metres", ParameterType::Number),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("light-well: {message}"),
                );
            }
        };
        let (wells, mut evaluation) = select_objects(context, &rule.selector);
        for well in wells {
            match judge_well(context, &declared, well) {
                Ok(judged) => {
                    for (message, evidence) in judged.findings {
                        evaluation.push_finding(finding(
                            rule,
                            &well.id,
                            message,
                            evidence,
                            judged.members.clone(),
                        ));
                    }
                    for message in judged.undecided {
                        evaluation.push_object_not_evaluated(
                            well.id.clone(),
                            NotEvaluatedReason::IncompleteEvidence,
                            message,
                        );
                    }
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(well.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

fn judge_well(
    context: &RuleContext<'_>,
    declared: &Declaration<'_>,
    well: &Object,
) -> Result<Judged, Unavailable> {
    let universe: Vec<&Object> = context.project.objects().collect();
    let (members, mut evidence) = declared.members.related(context, &well.id, &universe)?;
    if members.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} reaches no space through {}",
                well.id, declared.members.relationship
            ),
        ));
    }
    let service = extents(context)?;
    let mut stack: Vec<VerticalExtent> = members
        .iter()
        .map(|member| extent(service, member))
        .collect::<Result<_, _>>()?;
    stack.sort_by(|a, b| {
        a.bottom()
            .lower_metres()
            .total_cmp(&b.bottom().lower_metres())
            .then_with(|| a.object().cmp(b.object()))
    });
    evidence.extend(stack.iter().map(|member| member.evidence().clone()));
    let spans = context
        .services
        .get::<PlanSpanServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "plan-span service is not registered".to_owned(),
            )
        })?;
    let section = spans
        .measure_section(&members)
        .map_err(|error| section_unavailable(&error))?;
    evidence.push(section.evidence().clone());
    let mut judged = Judged {
        findings: Vec::new(),
        undecided: Vec::new(),
        members: members.clone(),
    };
    contiguity(&stack, declared.tolerance, &evidence, &mut judged);
    if section.area_upper() <= 0.0 {
        judged.findings.push((
            format!(
                "the {} stacked spaces share no plan section, so the well is not contiguous",
                members.len()
            ),
            evidence,
        ));
        return Ok(judged);
    }
    let height = (
        (stack
            .iter()
            .map(|m| m.top().lower_metres())
            .fold(f64::MIN, f64::max)
            - stack
                .iter()
                .map(|m| m.bottom().upper_metres())
                .fold(f64::MAX, f64::min))
        .max(0.0),
        stack
            .iter()
            .map(|m| m.top().upper_metres())
            .fold(f64::MIN, f64::max)
            - stack
                .iter()
                .map(|m| m.bottom().lower_metres())
                .fold(f64::MAX, f64::min),
    );
    let (index, row) = match match_rows(&declared.rows, RowSelection::First, |row| {
        row.holds(height.0, height.1)
    }) {
        Matched::Rows(rows) => match rows.first() {
            Some((index, row)) => (*index, *row),
            None => return Ok(judged),
        },
        Matched::Undecided | Matched::Ambiguous(_) => {
            judged.undecided.push(format!(
                "which row applies to a well {} high is undecided",
                shown(height.0, height.1)
            ));
            return Ok(judged);
        }
    };
    dimensions(&section, row, index, height, &evidence, &mut judged);
    Ok(judged)
}

/// Gaps between consecutive members, bottom to top.
fn contiguity(
    stack: &[VerticalExtent],
    tolerance: f64,
    evidence: &[Evidence],
    judged: &mut Judged,
) {
    for pair in stack.windows(2) {
        let (below, above) = (&pair[0], &pair[1]);
        let low = above.bottom().lower_metres() - below.top().upper_metres();
        let high = above.bottom().upper_metres() - below.top().lower_metres();
        if high <= tolerance {
            continue;
        }
        let gap = shown(low.max(0.0), high.max(0.0));
        if low > tolerance {
            judged.findings.push((
                format!(
                    "{} starts {gap} above the top of {}, so the well is not contiguous",
                    above.object(),
                    below.object()
                ),
                evidence.to_vec(),
            ));
        } else {
            judged.undecided.push(format!(
                "the gap between {} and {} is {gap}",
                below.object(),
                above.object()
            ));
        }
    }
}

/// The section's area and width against the applicable row.
fn dimensions(
    section: &PlanSection,
    row: &Requirement,
    index: usize,
    height: (f64, f64),
    evidence: &[Evidence],
    judged: &mut Judged,
) {
    let tall = shown(height.0, height.1);
    let mut cited = evidence.to_vec();
    cited.extend(section.width().map(|width| width.evidence().clone()));
    let mut check = |what: &str, unit: &str, (low, high): (f64, f64), minimum: Option<f64>| {
        let shown_value = if unit == "m" {
            shown(low, high)
        } else {
            let round = |value: f64| (value * 1e4).round() / 1e4;
            #[allow(clippy::float_cmp)]
            if round(low) == round(high) {
                format!("{} m²", round(low))
            } else {
                format!("between {} and {} m²", round(low), round(high))
            }
        };
        match judge(low, high, minimum, None) {
            Verdict::Pass => {}
            Verdict::Fail(bound) => judged.findings.push((
                format!(
                    "the well's {what} is {shown_value}; row {index} requires {bound} {unit} for a \
                     well {tall} high"
                ),
                cited.clone(),
            )),
            Verdict::Undecided(bound) => judged.undecided.push(format!(
                "the well's {what} is {shown_value}; row {index} requires {bound} {unit}, undecided"
            )),
        }
    };
    check(
        "section area",
        "m²",
        (section.area_lower(), section.area_upper()),
        row.area,
    );
    if let Some(width) = section.width() {
        check(
            "width",
            "m",
            (width.lower_metres(), width.upper_metres()),
            row.width,
        );
    }
}
