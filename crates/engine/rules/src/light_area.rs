//! The light-transmitting area of an opening, derived by an explicit
//! fallback: a stated area, else a size table, else the overall size less a
//! frame allowance.
//!
//! This is a numerator mode of `area-ratio`, not a capability of its own: the
//! ratio's traversal, population and interval judgement are unchanged, and
//! only where each member's area comes from differs.

use std::collections::BTreeMap;

use axioval_engine::{
    Citation, ColumnKind, CompiledRule, MeasuredMemo, MeasuredProvider, Measurement,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, PropertyResolutionError, RuleContext,
    TableColumn,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension, RuleId};

use crate::measured_kinds::{interval, refused};
use crate::support::table::{Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, invalid, resolve, si_quantity,
    undefined,
};

/// The value of `numerator_derivation` that selects this mode.
pub(crate) const MODE: &str = "light-area";

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("type", ColumnKind::TextPattern),
    TableColumn::required("width", ColumnKind::Quantity),
    TableColumn::required("height", ColumnKind::Quantity),
    TableColumn::required("light_area", ColumnKind::Quantity),
];

/// Parameters that only this mode reads.
#[cfg(feature = "parity-reference")]
const OWN: [&str; 7] = [
    "overall_width",
    "overall_height",
    "light_area_table",
    "light_type",
    "light_type_path",
    "light_size_tolerance",
    "frame_width",
];

/// The parameters this mode adds to `area-ratio`.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("overall_width", ParameterType::PropertyReference),
        ParameterDescriptor::optional("overall_height", ParameterType::PropertyReference),
        ParameterDescriptor::optional("light_area_table", ParameterType::Table(COLUMNS)),
        ParameterDescriptor::optional("light_type", ParameterType::PropertyReference),
        ParameterDescriptor::optional("light_type_path", ParameterType::StringList),
        ParameterDescriptor::optional("light_size_tolerance", ParameterType::Quantity),
        ParameterDescriptor::optional("frame_width", ParameterType::Quantity),
    ]
}

/// One row of the light-area table, in SI units.
struct Row {
    kind: Option<TextPattern>,
    width: f64,
    height: f64,
    light: f64,
}

/// Where the type name a table row matches is read.
struct TypeKey<'a> {
    property: PropertyRef<'a>,
    path: Option<Traversal>,
}

/// The declared fallback chain.
pub(crate) struct LightArea<'a> {
    stated: Option<PropertyRef<'a>>,
    width: PropertyRef<'a>,
    height: PropertyRef<'a>,
    table: Vec<Row>,
    kind: Option<TypeKey<'a>>,
    tolerance: f64,
    frame: Option<f64>,
}

/// A non-negative length parameter in metres, `Ok(None)` when not declared.
pub(crate) fn length(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(Some(value)),
        Some(_) => Err(invalid(format!(
            "parameter `{name}` must be a non-negative length"
        ))),
    }
}

fn cell(
    row: crate::support::table::Row<'_>,
    index: usize,
    column: &str,
    dimension: QuantityDimension,
) -> Result<f64, Unavailable> {
    let (value, unit) = row
        .quantity(column)?
        .ok_or_else(|| invalid(format!("light-area row {index} has no `{column}`")))?;
    let (value, found) = si_quantity(value, unit)
        .map_err(|(reason, message)| (reason, format!("light-area row {index}: {message}")))?;
    if found != dimension || value <= 0.0 {
        return Err(invalid(format!(
            "light-area row {index}: `{column}` must be a positive {}",
            if dimension == QuantityDimension::Area {
                "area"
            } else {
                "length"
            }
        )));
    }
    Ok(value)
}

impl<'a> LightArea<'a> {
    /// The declared chain, or `None` when `numerator_derivation` is not
    /// `light-area`; `stated` is the rule's `numerator_property`, step one.
    #[cfg(feature = "parity-reference")]
    pub(crate) fn parse(
        parameters: &Parameters<'a>,
        stated: Option<PropertyRef<'a>>,
    ) -> Result<Option<Self>, Unavailable> {
        match parameters.string("numerator_derivation")? {
            None => {
                for name in OWN {
                    if parameters.0.parameters.contains_key(name) {
                        return Err(invalid(format!(
                            "`{name}` applies only to `numerator_derivation` `{MODE}`"
                        )));
                    }
                }
                return Ok(None);
            }
            Some(MODE) => {}
            Some(other) => {
                return Err(invalid(format!(
                    "numerator_derivation `{other}` is unsupported; the only one is `{MODE}`"
                )));
            }
        }
        Self::declared(parameters, stated).map(Some)
    }

    /// The chain the mode's own parameters declare, read and refused in
    /// the order and words of [`Self::parse`]; `stated` is step one.
    pub(crate) fn declared(
        parameters: &Parameters<'a>,
        stated: Option<PropertyRef<'a>>,
    ) -> Result<Self, Unavailable> {
        let width = parameters.property("overall_width")?;
        let height = parameters.property("overall_height")?;
        let (Some(width), Some(height)) = (width, height) else {
            return Err(invalid(format!(
                "`{MODE}` needs `overall_width` and `overall_height`"
            )));
        };
        let tolerance = length(parameters, "light_size_tolerance")?.unwrap_or(0.0);
        let frame = length(parameters, "frame_width")?;
        let mut table = Vec::new();
        for (index, row) in parameters
            .table("light_area_table")?
            .unwrap_or_default()
            .into_iter()
            .enumerate()
        {
            let width = cell(row, index, "width", QuantityDimension::Length)?;
            let height = cell(row, index, "height", QuantityDimension::Length)?;
            let light = cell(row, index, "light_area", QuantityDimension::Area)?;
            if light > width * height {
                return Err(invalid(format!(
                    "light-area row {index}: the light area exceeds width times height"
                )));
            }
            table.push(Row {
                kind: row.pattern("type", true)?,
                width,
                height,
                light,
            });
        }
        if table.is_empty() && frame.is_none() {
            return Err(invalid(format!(
                "`{MODE}` needs a fallback: `light_area_table` or `frame_width`"
            )));
        }
        if parameters.0.parameters.contains_key("light_size_tolerance") && table.is_empty() {
            return Err(invalid(
                "`light_size_tolerance` applies only to `light_area_table`",
            ));
        }
        let kind = match (
            parameters.property("light_type")?,
            parameters.strings("light_type_path")?,
        ) {
            (Some(property), path) => Some(TypeKey {
                property,
                path: path.map(Traversal::path).transpose()?,
            }),
            (None, Some(_)) => {
                return Err(invalid(
                    "`light_type_path` is declared without `light_type`",
                ));
            }
            (None, None) => None,
        };
        if kind.is_none() && table.iter().any(|row| row.kind.is_some()) {
            return Err(invalid(
                "a light-area row matches `type`, but the rule declares no `light_type`",
            ));
        }
        Ok(Self {
            stated,
            width,
            height,
            table,
            kind,
            tolerance,
            frame,
        })
    }
}

/// Which step of the chain produced a light area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Stated,
    Table(usize),
    Frame,
}

/// One member's light area, with the evidence it rests on and, for a stated
/// area, how it compares with the member's overall area.
#[derive(Clone)]
struct Derived {
    area: f64,
    step: Origin,
    evidence: Vec<Evidence>,
    #[cfg(feature = "parity-reference")]
    check: Check,
}

/// The comparison of a stated light area with the overall area.
#[cfg(feature = "parity-reference")]
#[derive(Clone)]
enum Check {
    /// Within the overall area, or derived and so within it by construction.
    Within,
    /// Larger than the overall area: the finding's message.
    Oversized(String),
    /// The overall area is unknown: why.
    Unchecked(String),
}

/// The overall width and height of an object, or why they are not known.
type Size = Result<(f64, f64, Vec<Evidence>), String>;

/// Whether two lengths are equal within `tolerance` and binary rounding.
fn same(left: f64, right: f64, tolerance: f64) -> bool {
    (left - right).abs() <= tolerance + 4.0 * f64::EPSILON * left.abs().max(right.abs())
}

/// A number to four decimals, as a reviewer reads it.
fn round(value: f64) -> f64 {
    (value * 1e4).round() / 1e4
}

impl LightArea<'_> {
    /// A positive length stated by `property`, `Ok(None)` when exactly absent.
    pub(crate) fn length(
        context: &RuleContext<'_>,
        object: &Object,
        property: PropertyRef<'_>,
        evidence: &mut Vec<Evidence>,
    ) -> Result<Option<f64>, Unavailable> {
        let resolved = resolve(context, object, property)?;
        evidence.extend(resolved.evidence());
        match resolved.value() {
            None => Ok(None),
            Some(PropertyValue::Quantity {
                value,
                dimension: QuantityDimension::Length,
            }) if value.is_finite() && *value > 0.0 => Ok(Some(*value)),
            other => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} {property} is {}, not a positive length",
                    object.id,
                    display(other)
                ),
            )),
        }
    }

    /// The overall size, `Err` with a reason when it is not exactly known;
    /// a failure to read it is returned as the outer error.
    fn size(&self, context: &RuleContext<'_>, object: &Object) -> Result<Size, Unavailable> {
        let mut evidence = Vec::new();
        let width = Self::length(context, object, self.width, &mut evidence)?;
        let height = Self::length(context, object, self.height, &mut evidence)?;
        Ok(match (width, height) {
            (Some(width), Some(height)) => Ok((width, height, evidence)),
            (None, _) => Err(format!("{} is absent", self.width)),
            (_, None) => Err(format!("{} is absent", self.height)),
        })
    }

    /// The type name rows match, or why it is unknown.
    fn type_name(
        context: &RuleContext<'_>,
        key: &TypeKey<'_>,
        object: &Object,
        evidence: &mut Vec<Evidence>,
    ) -> Result<Result<String, String>, Unavailable> {
        let holders = match &key.path {
            None => vec![object.id.clone()],
            Some(path) => {
                let everything: Vec<&Object> = context.project.objects().collect();
                let (reached, cited) = path.related(context, &object.id, &everything)?;
                evidence.extend(cited);
                if reached.is_empty() {
                    return Ok(Err(format!("{} reaches no object", path.relationship)));
                }
                reached
            }
        };
        let mut found: Option<String> = None;
        for holder in holders {
            let target = context
                .project
                .object(&holder)
                .ok_or_else(|| invalid(format!("{holder} is not in the project")))?;
            let resolved = resolve(context, target, key.property)?;
            evidence.extend(resolved.evidence());
            let text = match resolved.value() {
                Some(PropertyValue::String(text)) if !undefined(resolved.value()) => text.clone(),
                other => {
                    return Ok(Err(format!(
                        "{} of {holder} is {}, not a type name",
                        key.property,
                        display(other)
                    )));
                }
            };
            match &found {
                Some(held) if *held != text => {
                    return Ok(Err(format!(
                        "{} differs across the reached objects (`{held}`, `{text}`)",
                        key.property
                    )));
                }
                Some(_) => {}
                None => found = Some(text),
            }
        }
        Ok(found.ok_or_else(|| format!("{} has no value", key.property)))
    }

    /// The object's overall width and height, citing them, or why they are
    /// not known.
    fn overall(
        &self,
        context: &RuleContext<'_>,
        object: &Object,
        evidence: &mut Vec<Evidence>,
    ) -> Result<(f64, f64), String> {
        match self.size(context, object) {
            Ok(Ok((width, height, cited))) => {
                evidence.extend(cited);
                Ok((width, height))
            }
            Ok(Err(why)) | Err((_, why)) => Err(why),
        }
    }

    /// How a stated light area compares with the object's overall area.
    #[cfg(feature = "parity-reference")]
    fn check(
        &self,
        stated: PropertyRef<'_>,
        area: f64,
        overall: Result<(f64, f64), String>,
    ) -> Check {
        match overall {
            Ok((width, height)) => {
                if area > width * height * (1.0 + 4.0 * f64::EPSILON) {
                    Check::Oversized(format!(
                        "light area {} m² ({stated}) is larger than the overall area {} m² \
                         ({} {} m × {} {} m)",
                        round(area),
                        round(width * height),
                        self.width,
                        round(width),
                        self.height,
                        round(height),
                    ))
                } else {
                    Check::Within
                }
            }
            Err(why) => Check::Unchecked(why),
        }
    }

    /// The light area of one member, by the first step that produces one.
    ///
    /// A step is skipped only when its input is exactly absent (no stated
    /// property, no matching row); any other failure stops the chain.
    fn derive(&self, context: &RuleContext<'_>, object: &Object) -> Result<Derived, Unavailable> {
        let mut evidence = Vec::new();
        if let Some(stated) = self.stated {
            let resolved = resolve(context, object, stated)?;
            evidence.extend(resolved.evidence());
            match resolved.value() {
                None => {}
                Some(PropertyValue::Quantity {
                    value,
                    dimension: QuantityDimension::Area,
                }) if value.is_finite() && *value >= 0.0 => {
                    let area = *value;
                    // Only the parity reference keeps the comparison; the
                    // template judges a stated area against its member.
                    let overall = self.overall(context, object, &mut evidence);
                    #[cfg(not(feature = "parity-reference"))]
                    let _ = overall;
                    return Ok(Derived {
                        area,
                        step: Origin::Stated,
                        evidence,
                        #[cfg(feature = "parity-reference")]
                        check: self.check(stated, area, overall),
                    });
                }
                Some(other) => {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "{} {stated} is {}, not an area",
                            object.id,
                            display(Some(other))
                        ),
                    ));
                }
            }
        }
        let (width, height, cited) = match self.size(context, object)? {
            Ok(size) => size,
            Err(why) => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} has no light area: {}{why}",
                        object.id,
                        self.stated
                            .map(|stated| format!("{stated} is absent and "))
                            .unwrap_or_default()
                    ),
                ));
            }
        };
        evidence.extend(cited);
        if let Some((index, area)) =
            self.table_row(context, object, width, height, &mut evidence)?
        {
            return Ok(Derived {
                area,
                step: Origin::Table(index),
                evidence,
                #[cfg(feature = "parity-reference")]
                check: Check::Within,
            });
        }
        let Some(frame) = self.frame else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} has no light area: {}no light-area row matches {} m × {} m",
                    object.id,
                    self.stated
                        .map(|stated| format!("{stated} is absent and "))
                        .unwrap_or_default(),
                    round(width),
                    round(height),
                ),
            ));
        };
        let area = width * height - 2.0 * (width + height) * frame;
        if area <= 0.0 {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "the frame allowance of {frame} m leaves {} no light area",
                    object.id
                ),
            ));
        }
        Ok(Derived {
            area,
            step: Origin::Frame,
            evidence,
            #[cfg(feature = "parity-reference")]
            check: Check::Within,
        })
    }

    /// The row of the light-area table that applies, `None` when none does.
    fn table_row(
        &self,
        context: &RuleContext<'_>,
        object: &Object,
        width: f64,
        height: f64,
        evidence: &mut Vec<Evidence>,
    ) -> Result<Option<(usize, f64)>, Unavailable> {
        if self.table.is_empty() {
            return Ok(None);
        }
        let name = match &self.kind {
            Some(key) => Some(Self::type_name(context, key, object, evidence)?),
            None => None,
        };
        let test = |row: &Row| {
            if !same(row.width, width, self.tolerance) || !same(row.height, height, self.tolerance)
            {
                return RowTest::NoMatch;
            }
            match (&row.kind, &name) {
                (None, _) => RowTest::Match(0),
                (Some(pattern), Some(Ok(name))) => pattern.test(name),
                (Some(_), _) => RowTest::Undecided,
            }
        };
        match match_rows(&self.table, RowSelection::MostSpecific, test) {
            Matched::Rows(rows) => Ok(rows.first().map(|&(index, row)| (index, row.light))),
            Matched::Undecided => {
                let why = match name {
                    Some(Err(why)) => why,
                    _ => "the type name is unknown".into(),
                };
                Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the light-area row of {} cannot be decided: {why}",
                        object.id
                    ),
                ))
            }
            Matched::Ambiguous(rows) => Err(invalid(format!(
                "light-area rows {} apply equally to {}",
                rows.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                object.id
            ))),
        }
    }

    /// The evidence entry that records which step produced a light area.
    fn record(&self, object: &ObjectId, step: Origin) -> Evidence {
        let step = match step {
            Origin::Stated => "stated".to_owned(),
            Origin::Table(index) => format!("table;row={index}"),
            Origin::Frame => format!("frame-allowance;frame_width={}", self.frame.unwrap_or(0.0)),
        };
        Evidence::exact(
            object.source.clone(),
            format!("axioval:derived.light-area:{object}:step={step}"),
        )
    }

    /// The summed light areas of `members`.
    ///
    /// Every member is derived even after one fails, so that each stated
    /// area larger than its element is reported.
    #[cfg(feature = "parity-reference")]
    pub(crate) fn sum(&self, context: &RuleContext<'_>, members: &[ObjectId]) -> Summed {
        let mut summed = Summed::default();
        for id in members {
            let derived = context
                .project
                .object(id)
                .ok_or_else(|| invalid(format!("{id} is not in the project")))
                .and_then(|object| self.derive(context, object));
            let derived = match derived {
                Ok(derived) => derived,
                Err(error) => {
                    summed.failure.get_or_insert(error);
                    continue;
                }
            };
            let mut evidence = derived.evidence;
            evidence.push(self.record(id, derived.step));
            match derived.check {
                Check::Within => {}
                Check::Oversized(message) => {
                    summed
                        .oversized
                        .push((id.clone(), message, evidence.clone()));
                }
                Check::Unchecked(why) => summed.unchecked.push((
                    id.clone(),
                    format!("the light area cannot be compared with the overall area: {why}"),
                )),
            }
            summed.lower += derived.area;
            summed.upper += derived.area;
            summed.evidence.extend(evidence);
            match derived.step {
                Origin::Stated => summed.steps[0] += 1,
                Origin::Table(_) => summed.steps[1] += 1,
                Origin::Frame => summed.steps[2] += 1,
            }
        }
        summed
    }
}

/// The light areas of one anchor's members.
#[cfg(feature = "parity-reference")]
#[derive(Default)]
pub(crate) struct Summed {
    pub(crate) lower: f64,
    pub(crate) upper: f64,
    pub(crate) evidence: Vec<Evidence>,
    /// Why a member has no light area: the first failure.
    pub(crate) failure: Option<Unavailable>,
    /// Members whose stated light area exceeds their overall area.
    pub(crate) oversized: Vec<(ObjectId, String, Vec<Evidence>)>,
    /// Members whose stated light area could not be compared.
    pub(crate) unchecked: Vec<(ObjectId, String)>,
    /// How many areas each step produced: stated, table, frame allowance.
    steps: [usize; 3],
}

#[cfg(feature = "parity-reference")]
impl Summed {
    /// The steps that produced the areas, for a finding message.
    pub(crate) fn provenance(&self) -> String {
        let parts: Vec<String> = ["stated", "from the light-area table", "by frame allowance"]
            .iter()
            .zip(self.steps)
            .filter(|(_, count)| *count > 0)
            .map(|(how, count)| format!("{count} {how}"))
            .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!("; light areas: {}", parts.join(", "))
        }
    }
}

/// The rule a light-area value's arguments stand for: the parameters it
/// names, under the keys it names them by, which are the mode's own
/// parameter names (`overall_width=@overall_width`), so the chain is read
/// and refused in the mode's words.
pub(crate) fn synthesised(parameters: BTreeMap<String, ParameterValue>) -> CompiledRule {
    CompiledRule {
        id: RuleId::new("axioval-measured-light-area").expect("a valid rule id"),
        capability: "axioval:capability.area-ratio".into(),
        severity: Severity::Info,
        selector: Selector::All,
        parameters,
    }
}

/// Checks the rule parameters a light-area value names, as the rule states
/// them and keyed by the value's keys, as the mode reads its declaration:
/// what [`LightArea::declared`] refuses, in its order and words.
///
/// # Errors
///
/// The declaration's refusal.
pub(crate) fn check_arguments(
    arguments: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = synthesised(arguments.clone());
    let parameters = Parameters(&rule);
    let stated = parameters.property("stated")?;
    LightArea::declared(&parameters, stated).map(|_| ())
}

/// The rule parameters a bound light-area call holds, as a rule states
/// them, keyed by the call's keys: everything but what it reads.
fn stated_arguments(call: &MeasuredCall) -> BTreeMap<String, ParameterValue> {
    call.arguments
        .iter()
        .filter_map(|(key, argument)| {
            let value = match argument {
                MeasuredArgument::Property { set, name } => ParameterValue::PropertyReference {
                    property: name.clone(),
                    property_set: set.clone(),
                },
                MeasuredArgument::Table(rows) => ParameterValue::Table {
                    value: rows.clone(),
                },
                MeasuredArgument::Length(metres) => ParameterValue::Quantity {
                    value: *metres,
                    unit: "m".into(),
                },
                MeasuredArgument::Path(steps) => ParameterValue::StringList {
                    value: steps.clone(),
                },
                _ => return None,
            };
            Some(((*key).to_owned(), value))
        })
        .collect()
}

/// The memo key of one object's light area under one chain.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AreaKey;

/// The memo key of one object's overall size under one chain.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SizeKey;

/// One object's light area, with its step's record: what every read of it
/// shares.
#[derive(Clone)]
struct Lit {
    area: f64,
    step: Origin,
    evidence: Vec<Evidence>,
    record: Evidence,
}

/// Measures `light_area`, `light_size` and `light_step`: the light area of
/// an opening as the `light-area` mode derives it, the overall size it is
/// compared with, and which step gave it.
pub(crate) struct LightMeasures;

const LIGHT_AREA: &str = "light_area";
const LIGHT_SIZE: &str = "light_size";
const LIGHT_STEP: &str = "light_step";

impl LightMeasures {
    /// The light area of `object` under the chain `call` states, derived
    /// once per object and chain for the run.
    fn lit(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Lit, Unavailable> {
        let arguments = stated_arguments(call);
        let key = format!("{arguments:?}");
        MeasuredMemo::of(context.services, (AreaKey, object.clone(), key), || {
            let rule = synthesised(arguments);
            let parameters = Parameters(&rule);
            let chain = LightArea::declared(&parameters, parameters.property("stated")?)?;
            let member = context
                .project
                .object(object)
                .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
            let derived = chain.derive(context, member)?;
            Ok(Lit {
                area: derived.area,
                step: derived.step,
                evidence: derived.evidence,
                record: chain.record(object, derived.step),
            })
        })
    }

    /// The overall width and height of `object`, or why they are not known.
    fn size(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(f64, f64, Vec<Evidence>), Unavailable> {
        let arguments = stated_arguments(call);
        let key = format!("{arguments:?}");
        MeasuredMemo::of(context.services, (SizeKey, object.clone(), key), || {
            let rule = synthesised(arguments);
            let parameters = Parameters(&rule);
            let chain = LightArea::declared(&parameters, parameters.property("stated")?)?;
            let member = context
                .project
                .object(object)
                .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
            match chain.size(context, member) {
                Ok(Ok(size)) => Ok(size),
                // Unknown, however it is unknown: the comparison cannot be made.
                Ok(Err(why)) | Err((_, why)) => Err((NotEvaluatedReason::IncompleteEvidence, why)),
            }
        })
    }
}

/// Whether every evidence entry is exact.
fn exact(evidence: &[Evidence]) -> bool {
    evidence.iter().all(|evidence| evidence.exact)
}

impl MeasuredProvider for LightMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[LIGHT_AREA, LIGHT_SIZE, LIGHT_STEP]
    }

    /// A light area cites the record of the step that produced it beside
    /// its value, as the capability's evidence recorded each member's step.
    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        let measurement = self.measure(call, object, context)?;
        let cites = call.name() == LIGHT_AREA
            && matches!(call.choice("read"), None | Some("area" | "stated"))
            && !matches!(measurement, Measurement::Absent { .. });
        let citation = match Self::lit(call, object, context) {
            Ok(lit) if cites => Citation {
                related: Vec::new(),
                evidence: vec![lit.record],
                ..Citation::default()
            },
            _ => Citation::default(),
        };
        Ok((measurement, citation))
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let refused = refused(call.name(), object);
        match call.name() {
            LIGHT_SIZE => {
                let (width, height, evidence) =
                    Self::size(call, object, context).map_err(refused)?;
                let value = if call.choice("side") == Some("height") {
                    height
                } else {
                    width
                };
                Ok(interval(
                    (value, value),
                    Some(QuantityDimension::Length),
                    exact(&evidence),
                    format!("{LIGHT_SIZE}:{object}"),
                ))
            }
            LIGHT_STEP => {
                let lit = Self::lit(call, object, context).map_err(refused)?;
                let counted = match (call.choice("step"), lit.step) {
                    (Some("stated"), Origin::Stated)
                    | (Some("table"), Origin::Table(_))
                    | (Some("frame"), Origin::Frame) => 1.0,
                    _ => 0.0,
                };
                Ok(interval(
                    (counted, counted),
                    None,
                    true,
                    format!("{LIGHT_STEP}:{object}"),
                ))
            }
            _ => match call.choice("read") {
                Some("overall") => {
                    let (width, height, evidence) =
                        Self::size(call, object, context).map_err(refused)?;
                    let area = width * height;
                    Ok(interval(
                        (area, area),
                        Some(QuantityDimension::Area),
                        exact(&evidence),
                        format!("{LIGHT_SIZE}:{object}"),
                    ))
                }
                Some("stated") => match Self::lit(call, object, context) {
                    Ok(lit) if lit.step == Origin::Stated => Ok(light(&lit)),
                    // Taken from another step, or from none: not stated.
                    _ => Ok(Measurement::Absent {
                        locator: format!("{LIGHT_AREA}:{object}:not-stated"),
                    }),
                },
                _ => Ok(light(&Self::lit(call, object, context).map_err(refused)?)),
            },
        }
    }
}

/// A light area, exact where everything it rests on is, the record of its
/// step among that; [`LightMeasures::measure_cited`] cites the record.
fn light(lit: &Lit) -> Measurement {
    interval(
        (lit.area, lit.area),
        Some(QuantityDimension::Area),
        exact(&lit.evidence) && lit.record.exact,
        format!("{LIGHT_AREA}:{}", lit.record.locator),
    )
}
