//! Door and window measurements as values, measured by the same steps
//! `keyed-limit`'s built-in quantities take: a clear width or height from
//! what the door states or its leaves, a sill height above the floors a
//! path reaches, a threshold step, and the door's leaves and swing.
//!
//! Property parameters are written `set/name`. The door-type defaults
//! table is a rule's; a measured value reads what the door states.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CompiledRule, FreeSpaceServiceHandle, MeasuredProvider, Measurement, ObjectFrameServiceHandle,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId, QuantityDimension, RuleId};

use super::{ClearHeight, ClearWidth, LeafMode, difference, extent, extents, sill_interval};
use crate::door_swing::{self, Footprint};
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
                Ok(value(measured.lower, measured.upper, LENGTH, measured.what))
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
                Ok(value(measured.lower, measured.upper, LENGTH, measured.what))
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
            absent @ Measurement::Absent { .. } => absent,
        })
    }

    /// The sill height or threshold step over every floor the path
    /// reaches: the greatest, or with `measure=least` the least.
    fn above_floors(
        call: &MeasuredCall,
        object: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let service = extents(context)?;
        let own = extent(service, &object.id)?;
        let everything: Vec<&Object> = context.project.objects().collect();
        let (floors, _) = path(call)?.related(context, &object.id, &everything)?;
        if floors.is_empty() {
            return Ok(Measurement::Absent {
                locator: "the path reaches no floor".into(),
            });
        }
        let threshold = if call.name() == "threshold_step" {
            let rule = rule(call, &[("threshold", "threshold")]);
            match Parameters(&rule).property("threshold")? {
                None => Some(0.0),
                Some(property) => super::thickness(context, object, property, &mut Vec::new())?,
            }
        } else {
            None
        };
        let least = call.choice("measure") == Some("least");
        let mut hull: Option<(f64, f64)> = None;
        for floor in floors {
            let floor = extent(service, &floor)?;
            let (lower, upper) = if call.name() == "sill_height" {
                sill_interval(&own, &floor)
            } else {
                let Some(threshold) = threshold else {
                    return Err((
                        axioval_engine::NotEvaluatedReason::IncompleteEvidence,
                        "the door states no threshold".into(),
                    ));
                };
                // The step from the floor to the door's bottom and its
                // threshold, unsigned.
                let lower = difference(
                    difference(own.bottom().lower_metres(), floor.bottom().upper_metres()).0,
                    -threshold,
                )
                .0;
                let upper = difference(
                    difference(own.bottom().upper_metres(), floor.bottom().lower_metres()).1,
                    -threshold,
                )
                .1;
                if lower >= 0.0 {
                    (lower, upper)
                } else if upper <= 0.0 {
                    (-upper, -lower)
                } else {
                    (0.0, (-lower).max(upper))
                }
            };
            hull = Some(match hull {
                None => (lower, upper),
                Some((low, high)) if least => (low.min(lower), high.min(upper)),
                Some((low, high)) => (low.max(lower), high.max(upper)),
            });
        }
        let (lower, upper) = hull.unwrap_or((0.0, 0.0));
        Ok(value(
            lower,
            upper,
            LENGTH,
            "over the floors reached".into(),
        ))
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
        match call.name() {
            "leaf_count" => {
                let leaves = count(leaves.leaves().len());
                Ok(value(leaves, leaves, None, "leaves".into()))
            }
            "leaf_width" => {
                let widths = leaves
                    .leaves()
                    .iter()
                    .map(axioval_engine::DoorLeaf::width_metres);
                let width = match call.choice("measure") {
                    Some("narrowest") => widths.fold(f64::INFINITY, f64::min),
                    Some("total") => widths.sum(),
                    _ => widths.fold(f64::NEG_INFINITY, f64::max),
                };
                if !width.is_finite() {
                    return Ok(Measurement::Absent {
                        locator: "the door has no leaves".into(),
                    });
                }
                Ok(value(width, width, LENGTH, "leaf widths".into()))
            }
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
                for space in spaces {
                    let (relation, _) = door_swing::relation(free, &leaves, &space)?;
                    if relation.swings_into() {
                        into.insert(space);
                    }
                }
                let into = count(into.len());
                Ok(value(into, into, None, "spaces swung into".into()))
            }
        }
    }
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
