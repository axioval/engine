//! Door and window measurements as values, measured by the same steps
//! `keyed-limit`'s built-in quantities take: a clear width or height from
//! what the door states or its leaves, a sill height above the floors a
//! path reaches, a threshold step, and the door's leaves and swing.
//!
//! Property parameters are written `set/name`. The door-type defaults
//! table is a rule's; a measured value reads what the door states.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::expression::Interval;
use axioval_engine::{
    CompiledRule, FreeSpaceServiceHandle, MeasuredProvider, Measurement, NotEvaluatedReason,
    ObjectFrameServiceHandle, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId, QuantityDimension, RuleId};

use super::threshold;
use super::{ClearHeight, ClearWidth, LeafMode, extent, extents, sill_interval};
use crate::door_swing::{self, Footprint};
use crate::measured_kinds::{interval, objects_of_kinds};
use crate::support::{Parameters, Traversal, Unavailable};

/// The names measured here.
pub(crate) const NAMES: &[&str] = &[
    "door_clear_height",
    "door_clear_width",
    "leaf_count",
    "leaf_width",
    "sill_height",
    "swing_area",
    "swings_into",
    "threshold_step",
];

/// Measures door and window values.
pub(crate) struct DoorMeasures;

/// A synthetic rule carrying `call`'s property and length arguments under
/// the parameter names `keyed-limit` reads them by.
fn rule(call: &MeasuredCall, names: &[(&str, &str)]) -> CompiledRule {
    let mut parameters = BTreeMap::new();
    for (key, parameter) in names {
        let value = match call.argument(key) {
            Some(MeasuredArgument::Property { set, name }) => ParameterValue::PropertyReference {
                property_set: set.clone(),
                property: name.clone(),
            },
            Some(MeasuredArgument::Length(value)) => ParameterValue::Quantity {
                value: *value,
                unit: "m".into(),
            },
            _ => continue,
        };
        parameters.insert((*parameter).to_owned(), value);
    }
    CompiledRule {
        id: RuleId::new("axioval-measured-door").expect("a valid rule id"),
        capability: "axioval:capability.keyed-limit".into(),
        severity: Severity::Info,
        selector: Selector::All,
        parameters,
    }
}

fn path(call: &MeasuredCall) -> Result<Traversal, Unavailable> {
    let Some(MeasuredArgument::Path(steps)) = call.argument("floor_path") else {
        return Err(crate::support::invalid("`floor_path` is required"));
    };
    Traversal::path(steps)
}

fn value(
    lower: f64,
    upper: f64,
    dimension: Option<QuantityDimension>,
    locator: String,
) -> Measurement {
    Measurement::Value {
        lower,
        upper,
        dimension,
        locator,
    }
}

const LENGTH: Option<QuantityDimension> = Some(QuantityDimension::Length);

/// A length `keyed-limit` measured citing `evidence`: exact as that
/// evidence is, whatever rounding the interval holds, so an expression
/// over it cites what the capability cites.
fn cited(measured: super::Measured) -> Measurement {
    let (lower, upper, locator) = (measured.lower, measured.upper, measured.what);
    if measured.evidence.iter().all(|evidence| evidence.exact) {
        Measurement::Rounded {
            lower,
            upper,
            dimension: LENGTH,
            locator,
        }
    } else {
        Measurement::Cited {
            lower,
            upper,
            dimension: LENGTH,
            locator,
            exact: false,
        }
    }
}

impl DoorMeasures {
    fn measure_object(
        call: &MeasuredCall,
        object: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let locator = |what: &str| format!("{}:{}:{what}", call.name(), object.id);
        match call.name() {
            "door_clear_width" => {
                let rule = rule(
                    call,
                    &[
                        ("stated", "stated"),
                        ("overall", "overall"),
                        ("deduction", "deduction"),
                    ],
                );
                let parameters = Parameters(&rule);
                let leaves = match call.choice("from_leaves") {
                    Some("passage") => Some(LeafMode::Passage),
                    Some("widest-leaf") => Some(LeafMode::WidestLeaf),
                    _ => None,
                };
                let deduction = parameters.quantity("deduction")?.map(|(value, _)| value);
                let measured = ClearWidth::declared(
                    parameters.property("stated")?,
                    leaves,
                    parameters.property("overall")?,
                    deduction,
                    None,
                )?
                .measure(context, object)?;
                Ok(cited(measured))
            }
            "door_clear_height" => {
                let rule = rule(
                    call,
                    &[
                        ("stated", "stated"),
                        ("overall", "overall"),
                        ("lining", "lining"),
                        ("threshold", "threshold"),
                    ],
                );
                let parameters = Parameters(&rule);
                let measured = ClearHeight {
                    stated: parameters.property("stated")?,
                    overall: parameters.property("overall")?,
                    lining: parameters.property("lining")?,
                    threshold: parameters.property("threshold")?,
                    defaults: None,
                }
                .measure(context, object)?;
                Ok(cited(measured))
            }
            "sill_height" | "threshold_step" => Self::above_floors(call, object, context),
            _ => Self::leaves(call, object, context),
        }
        .map(|measurement| match measurement {
            Measurement::Value {
                lower,
                upper,
                dimension,
                locator: what,
            } => value(
                lower,
                upper,
                dimension,
                format!("{}: {what}", locator("measured")),
            ),
            Measurement::Rounded {
                lower,
                upper,
                dimension,
                locator: what,
            } => Measurement::Rounded {
                lower,
                upper,
                dimension,
                locator: format!("{}: {what}", locator("measured")),
            },
            Measurement::Cited {
                lower,
                upper,
                dimension,
                locator: what,
                exact,
            } => Measurement::Cited {
                lower,
                upper,
                dimension,
                locator: format!("{}: {what}", locator("measured")),
                exact,
            },
            absent @ Measurement::Absent { .. } => absent,
        })
    }

    /// The sill height or threshold step over every floor the path
    /// reaches: the greatest, or with `measure=least` the least. An interval
    /// resting on exact extents only holds rounding, so it is cited exact.
    fn above_floors(
        call: &MeasuredCall,
        object: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let least = call.choice("measure") == Some("least");
        let measured = if call.name() == "threshold_step" {
            Self::threshold_step(call, object, context, least)?
        } else {
            Self::sill_height(call, object, context, least)?
        };
        let Some((lower, upper, exact)) = measured else {
            return Ok(Measurement::Absent {
                locator: "the path reaches no floor".into(),
            });
        };
        let locator = "over the floors reached".to_owned();
        Ok(if exact {
            Measurement::Rounded {
                lower,
                upper,
                dimension: LENGTH,
                locator,
            }
        } else {
            value(lower, upper, LENGTH, locator)
        })
    }

    fn sill_height(
        call: &MeasuredCall,
        object: &Object,
        context: &RuleContext<'_>,
        least: bool,
    ) -> Result<Option<(f64, f64, bool)>, Unavailable> {
        let service = extents(context)?;
        let own = extent(service, &object.id)?;
        let everything: Vec<&Object> = context.project.objects().collect();
        let (floors, _) = path(call)?.related(context, &object.id, &everything)?;
        let mut exact = own.evidence().exact;
        let mut hull: Option<(f64, f64)> = None;
        for floor in floors {
            // As the capability reads it: a floor it cannot measure leaves
            // the sill undecided, whatever the reason.
            let floor = extent(service, &floor).map_err(|(_, why)| {
                (
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("the floor of {floor} cannot be measured: {why}"),
                )
            })?;
            exact &= floor.evidence().exact;
            let (lower, upper) = sill_interval(&own, &floor);
            hull = Some(match hull {
                None => (lower, upper),
                Some((low, high)) if least => (low.min(lower), high.min(upper)),
                Some((low, high)) => (low.max(lower), high.max(upper)),
            });
        }
        Ok(hull.map(|(lower, upper)| (lower, upper, exact)))
    }

    /// The threshold step as `keyed-limit`'s `threshold-step` reads its
    /// floors, a ramp of the `ramps` kinds within `ramp_reach` of the door
    /// standing in for the floor of a space it lies over.
    fn threshold_step(
        call: &MeasuredCall,
        object: &Object,
        context: &RuleContext<'_>,
        least: bool,
    ) -> Result<Option<(f64, f64, bool)>, Unavailable> {
        let mut rule = rule(
            call,
            &[
                ("threshold", "threshold_thickness"),
                ("ramp_reach", "ramp_reach"),
            ],
        );
        if call.argument("ramps").is_some() {
            let ramps = objects_of_kinds(context, call, "ramps", &object.id)
                .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?;
            rule.parameters.insert(
                "ramp_selector".to_owned(),
                ParameterValue::Selector {
                    value: Box::new(Selector::Objects { objects: ramps }),
                },
            );
        }
        threshold::ThresholdStep::parse(&Parameters(&rule), path(call)?, None)?
            .measure(context, object, least)
    }

    /// The door's leaves: how many, how wide, the area they sweep, and how
    /// many reached spaces they swing into.
    fn leaves(
        call: &MeasuredCall,
        object: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let frames = context
            .services
            .get::<ObjectFrameServiceHandle>()
            .ok_or_else(|| {
                (
                    axioval_engine::NotEvaluatedReason::MissingService,
                    "the object-frame service is not registered, so the leaves are unknown"
                        .to_owned(),
                )
            })?;
        let leaves = frames.leaves(&object.id).map_err(|error| {
            (
                door_swing::reason(&error),
                format!("the leaves are unknown: {error}"),
            )
        })?;
        #[allow(clippy::cast_precision_loss)]
        let count = |value: usize| value as f64;
        // Leaves are derived from what the source states; cited as exactly
        // as their evidence.
        let stated = leaves.evidence().exact;
        match call.name() {
            "leaf_count" => {
                let counted = count(leaves.leaves().len());
                Ok(interval((counted, counted), None, stated, "leaves".into()))
            }
            "leaf_width" => leaf_width(call, &leaves),
            "swing_area" => {
                let footprint = Footprint::of(&leaves)?;
                let (lower, upper) =
                    footprint
                        .parts
                        .iter()
                        .fold((0.0, 0.0), |(low, high), part| {
                            (
                                low + part.0.area_square_metres(),
                                high + part.1.area_square_metres(),
                            )
                        });
                Ok(value(
                    lower,
                    upper,
                    Some(QuantityDimension::Area),
                    "swing footprint".into(),
                ))
            }
            _ => {
                let free = context
                    .services
                    .get::<FreeSpaceServiceHandle>()
                    .ok_or_else(|| {
                        (
                            axioval_engine::NotEvaluatedReason::MissingService,
                            "the free-space service is not registered".to_owned(),
                        )
                    })?;
                let Some(MeasuredArgument::Path(steps)) = call.argument("path") else {
                    return Err(crate::support::invalid("`path` is required"));
                };
                let everything: Vec<&Object> = context.project.objects().collect();
                let (spaces, _) =
                    Traversal::path(steps)?.related(context, &object.id, &everything)?;
                let mut into = BTreeSet::new();
                let mut exact = true;
                for space in spaces {
                    let (relation, evidence) = door_swing::relation(free, &leaves, &space)?;
                    exact &= evidence.iter().all(|evidence| evidence.exact);
                    if relation.swings_into() {
                        into.insert(space);
                    }
                }
                let into = count(into.len());
                Ok(interval(
                    (into, into),
                    None,
                    exact,
                    "spaces swung into".into(),
                ))
            }
        }
    }
}

/// The widest, narrowest or total width of `leaves`, cited as exactly as
/// the leaves are; none for a door without leaves.
fn leaf_width(
    call: &MeasuredCall,
    leaves: &axioval_engine::DoorLeaves,
) -> Result<Measurement, Unavailable> {
    if leaves.leaves().is_empty() {
        return Ok(Measurement::Absent {
            locator: "the door has no leaves".into(),
        });
    }
    let mut widths = leaves
        .leaves()
        .iter()
        .map(axioval_engine::DoorLeaf::width_metres);
    let width = match call.choice("measure") {
        Some("narrowest") => Interval::point(widths.fold(f64::INFINITY, f64::min)),
        // Summed outward, so the total holds the exact sum.
        Some("total") => widths
            .try_fold(Interval::point(0.0), |total, width| {
                total.plus(Interval::point(width))
            })
            .map_err(|_| crate::support::invalid("the leaves' widths overflow"))?,
        _ => Interval::point(widths.fold(f64::NEG_INFINITY, f64::max)),
    };
    Ok(interval(
        (width.lower, width.upper),
        LENGTH,
        leaves.evidence().exact,
        "leaf widths".into(),
    ))
}

impl MeasuredProvider for DoorMeasures {
    fn names(&self) -> &'static [&'static str] {
        NAMES
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let unavailable = |why: String| {
            PropertyResolutionError::Unavailable(format!("`{}` of {object}: {why}", call.name()))
        };
        let object = context
            .project
            .object(object)
            .ok_or_else(|| unavailable("it is not in the project".into()))?;
        Self::measure_object(call, object, context).map_err(|(reason, why)| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{}` of {}: {why}", call.name(), object.id),
            ))
        })
    }
}
