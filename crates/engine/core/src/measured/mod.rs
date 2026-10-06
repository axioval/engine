//! Values the engine measures from geometry services and answers as the
//! reserved property set [`axioval_ir::MEASURED_SET`].
//!
//! Each value is an interval sure to hold the exact value, and its evidence
//! is exact exactly when the measurement states so: the service's evidence
//! for what the engine measures itself, the [`provider::Measurement`]
//! variant for built-in code. Exactness is never inferred from a point: a
//! tessellation may measure a point too. An exact point is a quantity (or
//! a number); anything else is a [`PropertyValue::Measured`], with exact
//! evidence when the interval holds only the rounding of exact arithmetic.
//! Arithmetic over measured values (sums, differences, shares) rounds
//! outward. A missing service is a fact of the run, never of the object.
//!
//! Two names take parameters after the name, `;`-separated `key=value`
//! pairs: `bottom_above_level;path=<steps>` and
//! `boundary_area;kind=<kind>[;plane=<metres>]`. `level_height` is stated by
//! the source, not measured, and is answered by the host's resolver.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, Mutex};

use axioval_ir::{
    Evidence, MEASURED_AREA, MEASURED_BOTTOM, MEASURED_BOTTOM_ABOVE_LEVEL, MEASURED_BOUNDARY_AREA,
    MEASURED_EXTENT_X, MEASURED_EXTENT_Y, MEASURED_EXTENT_Z, MEASURED_LEVEL_HEIGHT, MEASURED_SET,
    MEASURED_TOP, MEASURED_VOLUME, MEASURED_X, MEASURED_Y, MEASURED_Z, ObjectId, Project, Property,
    PropertyValue, QuantityDimension,
};

mod alignment;
mod angles;
mod areas;
mod clearance;
pub(crate) mod faces;
mod levels;
pub(crate) mod provider;
mod section;
mod space;
mod surface;

use axioval_ir::measured::{
    ANGLE_TO, BEARING, CROSS_FALL, EXTENT, GRADIENT_DIRECTION, INCLINATION, LENGTH,
    MeasuredArgument, MeasuredCall, PERIMETER, SKEW, SLOPE, SLOPE_ALONG, THICKNESS,
};

use crate::ServiceRegistry;
use crate::boundary_coverage::{
    BoundaryCoverageRequest, BoundaryCoverageServiceHandle, BoundaryPlacement,
};
use crate::concepts::TypeHierarchyServiceHandle;
use crate::coordinate_system::CoordinateSystemServiceHandle;
use crate::free_space::MetricDirection;
use crate::object_frame::ObjectFrameServiceHandle;
use crate::path::PathSegment;
use crate::plan_area::PlanAreaServiceHandle;
use crate::plan_span::{PlanRectangle, PlanSpanServiceHandle};
use crate::properties::{
    CompletePropertyAbsenceEvidence, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionServiceHandle, ResolvedProperty,
};
use crate::proximity::ProximityServiceHandle;
use crate::relationships::{
    AbsentEndPolicy, RelationshipSelectionError, RelationshipSelectionServiceHandle,
};
use crate::space::SpaceServiceHandle;
use crate::vertical_extent::{SurfaceFace, VerticalExtentServiceHandle};
use crate::walking_surface::WalkingSurfaceServiceHandle;

/// A measured name, parsed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MeasuredName {
    /// A name without parameters.
    Plain(&'static str),
    /// The bottom above the one level the path reaches.
    BottomAboveLevel(Vec<PathSegment>),
    /// The summed area of the space boundaries against elements of `kind`.
    BoundaryArea { kind: String, plane: f64 },
    /// A slope, fall or tilt, measured from a face's normals or an axis.
    Surface(MeasuredCall),
    /// An angle to other objects or a bearing, with the path's steps.
    Angle(MeasuredCall, Vec<PathSegment>),
    /// A clearance: headroom, clearance below, clear width or height.
    Clearance(MeasuredCall),
    /// A space-validation aspect of a space or storey.
    Space(MeasuredCall),
    /// Where an object's level stands among its source's levels.
    Level(MeasuredCall),
    /// An area, share or coverage of the object.
    Area(MeasuredCall),
    /// A dimension of the body: an extent, length, thickness or perimeter.
    Dimension(MeasuredCall),
    /// A position along an alignment, or a parameter of it there.
    Alignment(MeasuredCall),
    /// A section across an alignment, or a clearance envelope swept along
    /// it.
    Section(MeasuredCall),
    /// A value a registered provider measures.
    Provided(MeasuredCall),
}

/// The names without parameters the engine measures itself.
const CORE: &[&str] = &[
    MEASURED_AREA,
    MEASURED_BOTTOM,
    MEASURED_EXTENT_X,
    MEASURED_EXTENT_Y,
    MEASURED_EXTENT_Z,
    MEASURED_LEVEL_HEIGHT,
    MEASURED_TOP,
    MEASURED_VOLUME,
    MEASURED_X,
    MEASURED_Y,
    MEASURED_Z,
];

/// Whether the engine measures `name` itself, rather than a provider.
pub(crate) fn measured_by_core(name: &str) -> bool {
    CORE.contains(&name)
        || [
            MEASURED_BOTTOM_ABOVE_LEVEL,
            MEASURED_BOUNDARY_AREA,
            SLOPE,
            SLOPE_ALONG,
            CROSS_FALL,
            INCLINATION,
            GRADIENT_DIRECTION,
            ANGLE_TO,
            BEARING,
            SKEW,
            EXTENT,
            LENGTH,
            THICKNESS,
            PERIMETER,
        ]
        .contains(&name)
        || clearance::NAMES.contains(&name)
        || space::NAMES.contains(&name)
        || levels::NAMES.contains(&name)
        || areas::NAMES.contains(&name)
        || alignment::NAMES.contains(&name)
        || section::NAMES.contains(&name)
}

/// Parses a name in the measured set through the registry
/// ([`axioval_ir::measured`]), or says why it is none.
pub(crate) fn parse(name: &str) -> Result<MeasuredName, String> {
    let call = axioval_ir::measured::parse(name).map_err(|error| error.to_string())?;
    Ok(match call.descriptor.name {
        MEASURED_BOTTOM_ABOVE_LEVEL => {
            let Some(MeasuredArgument::Path(steps)) = call.argument("path") else {
                return Err("`bottom_above_level` needs `path`".into());
            };
            MeasuredName::BottomAboveLevel(
                steps
                    .iter()
                    .map(|step| PathSegment::parse(step))
                    .collect::<Result<_, _>>()?,
            )
        }
        MEASURED_BOUNDARY_AREA => {
            let Some(MeasuredArgument::SourceKind(kind)) = call.argument("kind") else {
                return Err("`boundary_area` needs `kind`".into());
            };
            let Some(MeasuredArgument::Length(plane)) = call.argument("plane") else {
                return Err("`boundary_area` needs `plane`".into());
            };
            MeasuredName::BoundaryArea {
                kind: kind.clone(),
                plane: *plane,
            }
        }
        SLOPE | SLOPE_ALONG | CROSS_FALL | INCLINATION | GRADIENT_DIRECTION => {
            MeasuredName::Surface(call)
        }
        EXTENT | THICKNESS => {
            let (first, second) = if call.name() == EXTENT {
                ("axis", "direction")
            } else {
                ("direction", "face")
            };
            if call.argument(first).is_some() == call.argument(second).is_some() {
                return Err(format!(
                    "`{}` takes either `{first}` or `{second}`",
                    call.name()
                ));
            }
            MeasuredName::Dimension(call)
        }
        LENGTH | PERIMETER => MeasuredName::Dimension(call),
        name if clearance::NAMES.contains(&name) => MeasuredName::Clearance(call),
        name if space::NAMES.contains(&name) => MeasuredName::Space(call),
        name if levels::NAMES.contains(&name) => MeasuredName::Level(call),
        name if areas::NAMES.contains(&name) => MeasuredName::Area(call),
        name if alignment::NAMES.contains(&name) => MeasuredName::Alignment(call),
        name if section::NAMES.contains(&name) => MeasuredName::Section(call),
        ANGLE_TO | BEARING | SKEW => {
            let steps = match call.argument("path") {
                Some(MeasuredArgument::Path(steps)) => steps
                    .iter()
                    .map(|step| PathSegment::parse(step))
                    .collect::<Result<_, _>>()?,
                _ => Vec::new(),
            };
            MeasuredName::Angle(call, steps)
        }
        name if CORE.contains(&name) => MeasuredName::Plain(name),
        _ => MeasuredName::Provided(call),
    })
}

/// The measured value `name` (with its parameters) of `object`, measured
/// with the geometry services in `services` as a run would, outside a run:
/// for a host previewing a value or a test comparing one with a
/// capability's judgement. `project` holds the objects a path may reach.
///
/// # Errors
///
/// As a run's property resolution: an unknown or malformed name, a missing
/// service, or a measurement that cannot be made.
pub fn measured_value(
    services: &ServiceRegistry,
    project: &Project,
    object: &ObjectId,
    name: &str,
) -> Result<PropertyResolution, PropertyResolutionError> {
    let host = services.get::<PropertyResolutionServiceHandle>();
    let request = PropertyRequest::try_new(object.clone(), Some(MEASURED_SET.to_owned()), name)?;
    Measures::of(services, host, project).resolve(&request)
}

/// `minuend - subtrahend` as an interval holding the exact difference.
pub(crate) fn rounded_difference(minuend: f64, subtrahend: f64) -> (f64, f64) {
    let rounded = minuend - subtrahend;
    let back = rounded - minuend;
    let error = (minuend - (rounded - back)) + (-subtrahend - back);
    if error > 0.0 {
        (rounded, rounded.next_up())
    } else if error < 0.0 {
        (rounded.next_down(), rounded)
    } else {
        (rounded, rounded)
    }
}

/// `[lower, upper]` as an interval, for sound arithmetic over measured
/// bounds.
pub(crate) fn span(lower: f64, upper: f64) -> crate::expression::Interval {
    crate::expression::Interval { lower, upper }
}

/// The bounds of a sum, difference or share of measured intervals, rounded
/// outward; an overflow is no value.
pub(crate) fn bounds(
    result: crate::expression::IntervalResult,
) -> Result<(f64, f64), PropertyResolutionError> {
    result
        .map(|interval| (interval.lower, interval.upper))
        .map_err(|_| PropertyResolutionError::InvalidValue)
}

/// One measured value of one object as the run's resolver answers a
/// request for it, without the request binding a property resolution
/// carries: the same value and evidence ([`MeasuredValues`]).
#[derive(Clone, Debug, PartialEq)]
pub enum MeasuredRead {
    /// The value, and the evidence it cites.
    Value(PropertyValue, Option<Evidence>),
    /// No value, known exactly, and the evidence of its absence.
    Absent(Evidence),
}

/// The run's measured values, read directly: what the run's resolver
/// answers for a request in the measured set (`axioval:measured`), the very
/// same measurement, value and evidence, for many objects at once, without
/// building a request and a resolution per object.
///
/// Every run installs one beside its resolver, sharing its measurements
/// (and the run's [`MeasuredMemo`](provider::MeasuredMemo)). A template
/// reads a value that is one measured read through it; anything else goes
/// through property resolution.
#[derive(Clone)]
pub struct MeasuredValues(Measures);

impl MeasuredValues {
    /// The measured values of `project`'s objects as `services` measure
    /// them, `level_height` stated by the resolver `services` hold: what
    /// [`measured_value`] reads, for a host or a test reading values outside
    /// a run as a run reads them. A run installs its own.
    #[must_use]
    pub fn of(services: &ServiceRegistry, project: &Project) -> Self {
        let host = services.get::<PropertyResolutionServiceHandle>();
        Self(Measures::of(services, host, project))
    }

    /// The values `measures` answer: the run's resolver's own.
    pub(crate) fn sharing(measures: &Measures) -> Self {
        Self(measures.clone())
    }

    /// The measured value `name` (with its parameters, as a request in
    /// the measured set names it) of each of `objects`, one answer per
    /// object in order: each exactly the value and evidence the run's
    /// resolver answers for that request, or its refusal. A provider
    /// measures every object through its batch entry point.
    #[must_use]
    pub fn read_batch(
        &self,
        name: &str,
        objects: &[&ObjectId],
    ) -> Vec<Result<MeasuredRead, PropertyResolutionError>> {
        self.0.read_batch(name, objects)
    }
}

/// The answer a provider's measurement is. The provider states
/// exactness by the variant it answers; a `Value` states none, so it is
/// never exact, a point included.
fn answer_of(measurement: provider::Measurement) -> Answer {
    match measurement {
        provider::Measurement::Value {
            lower,
            upper,
            dimension,
            locator,
        } => Answer::Value(lower, upper, dimension, locator, false),
        provider::Measurement::Rounded {
            lower,
            upper,
            dimension,
            locator,
        } => Answer::Value(lower, upper, dimension, locator, true),
        provider::Measurement::Cited {
            lower,
            upper,
            dimension,
            locator,
            exact,
        } => Answer::Value(lower, upper, dimension, locator, exact),
        provider::Measurement::Absent { locator } => Answer::Absent(locator),
    }
}

/// A measured answer before it becomes a property.
enum Answer {
    /// A value sure to lie in `[lower, upper]`, of a dimension (`None` for a
    /// plain number, such as a count), located, and whether its evidence is
    /// exact. Every measurement states its exactness; none is inferred from
    /// a point.
    Value(f64, f64, Option<QuantityDimension>, String, bool),
    Absent(String),
}

/// Every measured name parsed in this process, as requested. Parsing is a
/// pure function of the name (the registry's tables and the path grammar),
/// so a run reading one name of many objects, and every later run, parses
/// it once; a name that does not parse is not kept. Cleared when it holds
/// [`PARSED_LIMIT`] names, so a long-lived host never grows it unbounded.
static PARSED: LazyLock<Mutex<HashMap<String, Arc<MeasuredName>>>> = LazyLock::new(Mutex::default);

/// How many parsed names [`PARSED`] keeps at most.
const PARSED_LIMIT: usize = 4096;

/// The geometry services a run measures with, as the host registered them,
/// and the project's objects a path may reach.
#[derive(Clone, Default)]
pub(crate) struct Measures {
    vertical: Option<VerticalExtentServiceHandle>,
    plan: Option<PlanAreaServiceHandle>,
    proximity: Option<ProximityServiceHandle>,
    frames: Option<ObjectFrameServiceHandle>,
    relationships: Option<RelationshipSelectionServiceHandle>,
    boundaries: Option<BoundaryCoverageServiceHandle>,
    hierarchy: Option<TypeHierarchyServiceHandle>,
    rectangles: Option<PlanSpanServiceHandle>,
    walking: Option<WalkingSurfaceServiceHandle>,
    facades: Option<crate::facade_area::FacadeAreaServiceHandle>,
    contacts: Option<crate::contact::ContactServiceHandle>,
    spaces: Option<SpaceServiceHandle>,
    coordinates: Option<CoordinateSystemServiceHandle>,
    alignments: Option<crate::alignment::AlignmentServiceHandle>,
    host: Option<PropertyResolutionServiceHandle>,
    providers: Option<(provider::Providers, ServiceRegistry)>,
    kinds: Arc<BTreeMap<ObjectId, String>>,
}

impl Measures {
    pub(crate) fn of(
        services: &ServiceRegistry,
        host: Option<&PropertyResolutionServiceHandle>,
        project: &Project,
    ) -> Self {
        Self {
            vertical: services.get::<VerticalExtentServiceHandle>().cloned(),
            plan: services.get::<PlanAreaServiceHandle>().cloned(),
            proximity: services.get::<ProximityServiceHandle>().cloned(),
            frames: services.get::<ObjectFrameServiceHandle>().cloned(),
            relationships: services
                .get::<RelationshipSelectionServiceHandle>()
                .cloned(),
            boundaries: services.get::<BoundaryCoverageServiceHandle>().cloned(),
            hierarchy: services.get::<TypeHierarchyServiceHandle>().cloned(),
            rectangles: services.get::<PlanSpanServiceHandle>().cloned(),
            walking: services.get::<WalkingSurfaceServiceHandle>().cloned(),
            facades: services
                .get::<crate::facade_area::FacadeAreaServiceHandle>()
                .cloned(),
            contacts: services
                .get::<crate::contact::ContactServiceHandle>()
                .cloned(),
            spaces: services.get::<SpaceServiceHandle>().cloned(),
            coordinates: services.get::<CoordinateSystemServiceHandle>().cloned(),
            alignments: services
                .get::<crate::alignment::AlignmentServiceHandle>()
                .cloned(),
            host: host.cloned(),
            providers: services
                .get::<provider::Providers>()
                .map(|providers| (providers.clone(), services.clone())),
            kinds: Arc::new(
                project
                    .objects()
                    .map(|object| (object.id.clone(), object.kind().to_owned()))
                    .collect(),
            ),
        }
    }

    /// `name` parsed, once per process ([`PARSED`]).
    fn parsed(name: &str) -> Result<Arc<MeasuredName>, PropertyResolutionError> {
        if let Ok(parsed) = PARSED.lock()
            && let Some(known) = parsed.get(name)
        {
            return Ok(known.clone());
        }
        let known = Arc::new(parse(name).map_err(|_| PropertyResolutionError::InvalidRequest)?);
        if let Ok(mut parsed) = PARSED.lock() {
            if parsed.len() >= PARSED_LIMIT {
                parsed.clear();
            }
            parsed.insert(name.to_owned(), known.clone());
        }
        Ok(known)
    }

    /// Answers one request in the measured set.
    pub(crate) fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let name = Self::parsed(request.property())?;
        if *name == MeasuredName::Plain(MEASURED_LEVEL_HEIGHT) {
            return self.level_height(request);
        }
        Self::resolution(request, self.measure(&name, request.object_id()))
    }

    /// `level_height`, stated by the source, not measured.
    fn level_height(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        match &self.host {
            Some(host) => host.resolve(request),
            None => Err(PropertyResolutionError::MissingService(
                "no property-resolution service states `level_height`".into(),
            )),
        }
    }

    /// The answer to `request` as a property, or its stated absence.
    fn resolution(
        request: &PropertyRequest,
        answer: Result<Answer, PropertyResolutionError>,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        match Self::read(request.object_id(), request.property(), answer)? {
            MeasuredRead::Absent(evidence) => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(request.clone(), evidence)?,
            )),
            MeasuredRead::Value(value, evidence) => {
                let mut property = Property::new(MEASURED_SET, request.property(), value)
                    .map_err(|_| PropertyResolutionError::InvalidRequest)?;
                property.evidence = evidence;
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
        }
    }

    /// The answer about `object` to a request for the measured value
    /// `name`: its value and evidence, or its stated absence.
    fn read(
        object: &ObjectId,
        name: &str,
        answer: Result<Answer, PropertyResolutionError>,
    ) -> Result<MeasuredRead, PropertyResolutionError> {
        let locate = |locator: String| {
            let mut located =
                String::with_capacity(MEASURED_SET.len() + name.len() + locator.len() + 3);
            located.push_str(MEASURED_SET);
            located.push('/');
            located.extend(name.chars().map(|letter| letter.to_ascii_lowercase()));
            located.push_str(": ");
            located.push_str(&locator);
            located
        };
        let (lower, upper, dimension, locator, exact) = match answer? {
            Answer::Value(lower, upper, dimension, locator, exact) => {
                (lower, upper, dimension, locator, exact)
            }
            Answer::Absent(locator) => {
                return Ok(MeasuredRead::Absent(Evidence::exact(
                    object.source.clone(),
                    locate(locator),
                )));
            }
        };
        if !(lower.is_finite() && upper.is_finite() && lower <= upper) {
            return Err(PropertyResolutionError::InvalidValue);
        }
        let point = lower.to_bits() == upper.to_bits();
        // Exact only as the measurement states: a point measured on a
        // tessellation is as inexact as an interval.
        let value = match (point && exact, dimension) {
            (true, Some(dimension)) => PropertyValue::Quantity {
                value: lower,
                dimension,
            },
            // A whole count is an integer; any other plain number a decimal.
            #[allow(clippy::cast_possible_truncation)]
            (true, None) if lower.fract() == 0.0 && lower.abs() < 9.0e15 => {
                PropertyValue::Integer(lower as i64)
            }
            (true, None) => PropertyValue::Decimal(lower),
            (false, dimension) => PropertyValue::Measured {
                lower,
                upper,
                dimension,
            },
        };
        let mut evidence = Evidence::exact(object.source.clone(), locate(locator));
        evidence.exact = exact;
        Ok(MeasuredRead::Value(value, Some(evidence)))
    }

    /// The measured value `name` of each of `objects`, read directly
    /// ([`MeasuredValues::read_batch`]).
    fn read_batch(
        &self,
        name: &str,
        objects: &[&ObjectId],
    ) -> Vec<Result<MeasuredRead, PropertyResolutionError>> {
        let refused =
            |error: PropertyResolutionError| objects.iter().map(|_| Err(error.clone())).collect();
        let parsed = match Self::parsed(name) {
            Ok(parsed) => parsed,
            Err(error) => return refused(error),
        };
        // A name no property can carry is refused as a resolution refuses it.
        if Property::new(MEASURED_SET, name, PropertyValue::Null).is_err() {
            return refused(PropertyResolutionError::InvalidRequest);
        }
        if *parsed == MeasuredName::Plain(MEASURED_LEVEL_HEIGHT) {
            return objects
                .iter()
                .map(|object| {
                    let request = PropertyRequest::try_new(
                        (*object).clone(),
                        Some(MEASURED_SET.to_owned()),
                        name,
                    )?;
                    Ok(match self.level_height(&request)? {
                        PropertyResolution::Present(resolved) => {
                            let property = resolved.into_property();
                            MeasuredRead::Value(property.value, property.evidence)
                        }
                        PropertyResolution::Absent(proof) => {
                            MeasuredRead::Absent(proof.evidence().clone())
                        }
                    })
                })
                .collect();
        }
        let answers = match &*parsed {
            MeasuredName::Provided(call) => self.provided_batch(call, objects),
            parsed => objects
                .iter()
                .map(|object| self.measure(parsed, object))
                .collect(),
        };
        objects
            .iter()
            .zip(answers)
            .map(|(object, answer)| Self::read(object, name, answer))
            .collect()
    }

    fn missing(name: &str, service: &str) -> PropertyResolutionError {
        PropertyResolutionError::MissingService(format!(
            "no {service} service is registered, so `{MEASURED_SET}` value `{name}` cannot be \
             measured"
        ))
    }

    fn unavailable(name: &str, object: &ObjectId, error: &str) -> PropertyResolutionError {
        PropertyResolutionError::Unavailable(format!(
            "`{MEASURED_SET}` value `{name}` of {object}: {error}"
        ))
    }

    fn measure(
        &self,
        name: &MeasuredName,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        match name {
            MeasuredName::Plain(name) => self.plain(name, object),
            MeasuredName::BottomAboveLevel(steps) => self.bottom_above_level(steps, object),
            MeasuredName::BoundaryArea { kind, plane } => self.boundary_area(kind, *plane, object),
            MeasuredName::Surface(call) => self.surface(call, object),
            MeasuredName::Angle(call, steps) => self.angle(call, steps, object),
            MeasuredName::Dimension(call) => self.dimension(call, object),
            MeasuredName::Clearance(call) => self.clearance(call, object),
            MeasuredName::Space(call) => self.space(call, object),
            MeasuredName::Level(call) => self.level_measure(call, object),
            MeasuredName::Area(call) => self.area_measure(call, object),
            MeasuredName::Alignment(call) => self.along_alignment(call, object),
            MeasuredName::Section(call) => self.along_section(call, object),
            MeasuredName::Provided(call) => self.provided(call, object),
        }
    }

    fn plain(&self, name: &str, object: &ObjectId) -> Result<Answer, PropertyResolutionError> {
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let length = Some(QuantityDimension::Length);
        match name {
            MEASURED_BOTTOM | MEASURED_TOP | MEASURED_EXTENT_Z => {
                let extent = self
                    .vertical
                    .as_ref()
                    .ok_or_else(|| Self::missing(name, "vertical-extent"))?
                    .measure_vertical_extent(object)
                    .map_err(|error| unavailable(error.to_string()))?;
                let locator = extent.evidence().locator.clone();
                let (lower, upper) = match name {
                    MEASURED_BOTTOM => (
                        extent.bottom().lower_metres(),
                        extent.bottom().upper_metres(),
                    ),
                    MEASURED_TOP => (extent.top().lower_metres(), extent.top().upper_metres()),
                    _ => extent.height_metres(),
                };
                Ok(Answer::Value(
                    lower,
                    upper,
                    length,
                    locator,
                    extent.evidence().exact,
                ))
            }
            MEASURED_EXTENT_X | MEASURED_EXTENT_Y => {
                let axis = if name == MEASURED_EXTENT_X {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                let direction = MetricDirection::try_new(axis)
                    .map_err(|error| unavailable(error.to_string()))?;
                let extent = self
                    .vertical
                    .as_ref()
                    .ok_or_else(|| Self::missing(name, "vertical-extent"))?
                    .measure_directional_extent(object, direction)
                    .map_err(|error| unavailable(error.to_string()))?;
                let (lower, upper) = extent.length_metres();
                Ok(Answer::Value(
                    lower,
                    upper,
                    length,
                    extent.evidence().locator.clone(),
                    extent.evidence().exact,
                ))
            }
            MEASURED_X | MEASURED_Y | MEASURED_Z => {
                // A placement is read only when stated exactly.
                let (coordinate, locator) = self.origin(name, object)?;
                let axis = match name {
                    MEASURED_X => 0,
                    MEASURED_Y => 1,
                    _ => 2,
                };
                Ok(Answer::Value(
                    coordinate[axis],
                    coordinate[axis],
                    length,
                    locator,
                    true,
                ))
            }
            MEASURED_AREA => {
                let area = self
                    .plan
                    .as_ref()
                    .ok_or_else(|| Self::missing(name, "plan-area"))?
                    .measure_footprint(object)
                    .map_err(|error| unavailable(error.to_string()))?;
                Ok(Answer::Value(
                    area.lower_square_metres(),
                    area.upper_square_metres(),
                    Some(QuantityDimension::Area),
                    area.evidence().locator.clone(),
                    area.evidence().exact,
                ))
            }
            MEASURED_VOLUME => {
                let body = self
                    .proximity
                    .as_ref()
                    .ok_or_else(|| Self::missing(name, "proximity"))?
                    .measure_body_volume(object)
                    .map_err(|error| unavailable(error.to_string()))?;
                Ok(Answer::Value(
                    body.volume().lower_cubic_metres(),
                    body.volume().upper_cubic_metres(),
                    Some(QuantityDimension::Volume),
                    body.evidence().locator.clone(),
                    body.evidence().exact,
                ))
            }
            _ => Err(PropertyResolutionError::InvalidRequest),
        }
    }

    /// `object`'s placement frame, stated exactly, and its locator.
    fn frame(
        &self,
        name: &str,
        object: &ObjectId,
    ) -> Result<(crate::free_space::MetricFrame, String), PropertyResolutionError> {
        let frame = self
            .frames
            .as_ref()
            .ok_or_else(|| Self::missing(name, "object-frame"))?
            .object_frame(object)
            .map_err(|error| Self::unavailable(name, object, &error.to_string()))?;
        if !frame.evidence().exact {
            return Err(Self::unavailable(
                name,
                object,
                "its placement is not stated exactly",
            ));
        }
        Ok((frame.frame().clone(), frame.evidence().locator.clone()))
    }

    /// The world coordinates of `object`'s placement origin, stated exactly.
    fn origin(
        &self,
        name: &str,
        object: &ObjectId,
    ) -> Result<([f64; 3], String), PropertyResolutionError> {
        let (frame, locator) = self.frame(name, object)?;
        Ok((frame.origin().coordinates_metres(), locator))
    }

    /// The direction an axis argument names for `object`: an own axis of
    /// its placement, a world axis, or a stated vector.
    fn direction(
        &self,
        name: &str,
        argument: &MeasuredArgument,
        object: &ObjectId,
    ) -> Result<MetricDirection, PropertyResolutionError> {
        let vector = match argument {
            MeasuredArgument::Vector(vector) => *vector,
            MeasuredArgument::Choice("x") => [1.0, 0.0, 0.0],
            MeasuredArgument::Choice("y") => [0.0, 1.0, 0.0],
            MeasuredArgument::Choice("z") => [0.0, 0.0, 1.0],
            MeasuredArgument::Choice(own) => {
                let (frame, _) = self.frame(name, object)?;
                match *own {
                    "own_y" => frame.forward(),
                    "own_z" => frame.up(),
                    _ => frame.right(),
                }
                .components()
            }
            _ => return Err(PropertyResolutionError::InvalidRequest),
        };
        MetricDirection::try_new(vector)
            .map_err(|error| Self::unavailable(name, object, &error.to_string()))
    }

    /// An extent, length, thickness or perimeter of `object`'s body.
    fn dimension(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let length = Some(QuantityDimension::Length);
        if name == PERIMETER {
            let perimeter = self
                .plan
                .as_ref()
                .ok_or_else(|| Self::missing(name, "plan-area"))?
                .measure_footprint_perimeter(object)
                .map_err(|error| unavailable(error.to_string()))?;
            return Ok(Answer::Value(
                perimeter.lower_metres(),
                perimeter.upper_metres(),
                length,
                perimeter.evidence().locator.clone(),
                perimeter.evidence().exact,
            ));
        }
        let service = self
            .vertical
            .as_ref()
            .ok_or_else(|| Self::missing(name, "vertical-extent"))?;
        let along = |key: &str| call.argument(key);
        if name == THICKNESS {
            // A direction square to a face measured on a tessellation is
            // itself approximate, whatever the thickness along it.
            let mut stated = true;
            let direction = match (along("direction"), call.choice("face")) {
                (Some(argument), _) => self.direction(name, argument, object)?,
                (None, face) => {
                    let face = if face == Some("bottom") {
                        SurfaceFace::Bottom
                    } else {
                        SurfaceFace::Top
                    };
                    let normals = service
                        .measure_face_normals(object, face)
                        .map_err(|error| unavailable(error.to_string()))?;
                    stated = normals.evidence().exact;
                    surface::plane_normal(normals.normals())
                        .and_then(|normal| {
                            MetricDirection::try_new(normal).map_err(|error| error.to_string())
                        })
                        .map_err(unavailable)?
                }
            };
            let thickness = service
                .measure_thickness(object, direction)
                .map_err(|error| unavailable(error.to_string()))?;
            return Ok(Answer::Value(
                thickness.lower_metres(),
                thickness.upper_metres(),
                length,
                thickness.evidence().locator.clone(),
                stated && thickness.evidence().exact,
            ));
        }
        let argument = along("axis")
            .or_else(|| along("direction"))
            .ok_or(PropertyResolutionError::InvalidRequest)?;
        let direction = self.direction(name, argument, object)?;
        let extent = service
            .measure_directional_extent(object, direction)
            .map_err(|error| unavailable(error.to_string()))?;
        let (lower, upper) = extent.length_metres();
        Ok(Answer::Value(
            lower,
            upper,
            length,
            extent.evidence().locator.clone(),
            extent.evidence().exact,
        ))
    }

    /// What the provider registered for `call`'s name measures of `object`.
    fn provided(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let Some((providers, services)) = &self.providers else {
            return Err(Self::missing(name, "built-in measurement"));
        };
        let provider = providers
            .of(name)
            .ok_or_else(|| Self::missing(name, "built-in measurement"))?;
        let context = crate::RuleContext {
            project: &providers.project,
            services,
        };
        provider.measure(call, object, &context).map(answer_of)
    }

    /// What the provider registered for `call`'s name measures of each of
    /// `objects`, through its batch entry point.
    fn provided_batch(
        &self,
        call: &MeasuredCall,
        objects: &[&ObjectId],
    ) -> Vec<Result<Answer, PropertyResolutionError>> {
        let name = call.name();
        let Some((providers, services)) = &self.providers else {
            return objects
                .iter()
                .map(|_| Err(Self::missing(name, "built-in measurement")))
                .collect();
        };
        let Some(provider) = providers.of(name) else {
            return objects
                .iter()
                .map(|_| Err(Self::missing(name, "built-in measurement")))
                .collect();
        };
        let context = crate::RuleContext {
            project: &providers.project,
            services,
        };
        let mut answers: Vec<Result<Answer, PropertyResolutionError>> = provider
            .measure_batch(call, objects, &context)
            .into_iter()
            .map(|measured| measured.map(answer_of))
            .collect();
        // A provider answering for other objects than asked answers none.
        if answers.len() != objects.len() {
            answers = objects
                .iter()
                .map(|_| Err(PropertyResolutionError::ResponseRequestMismatch))
                .collect();
        }
        answers
    }

    /// The least-area rectangle of `object`'s footprint.
    fn rectangle(
        &self,
        name: &str,
        object: &ObjectId,
    ) -> Result<PlanRectangle, PropertyResolutionError> {
        self.rectangles
            .as_ref()
            .ok_or_else(|| Self::missing(name, "plan-span"))?
            .measure_rectangle(object)
            .map_err(|error| Self::unavailable(name, object, &error.to_string()))
    }

    /// An angle between `object` and the objects `steps` reach, or a
    /// bearing of one of its axes.
    fn angle(
        &self,
        call: &MeasuredCall,
        steps: &[PathSegment],
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let answer = |value: crate::expression::Interval, locator: String, exact: bool| {
            Answer::Value(
                value.lower,
                value.upper,
                Some(QuantityDimension::PlaneAngle),
                locator,
                exact,
            )
        };
        if name == BEARING {
            let (value, locator, exact) = self.bearing(call, object)?;
            return Ok(answer(value, locator, exact));
        }
        let (reached, cited) = self.reach(name, steps, object)?;
        if reached.is_empty() {
            return Ok(Answer::Absent(format!(
                "the path reaches no object ({})",
                cited.join("; ")
            )));
        }
        let faces = name == ANGLE_TO && call.choice("between") == Some("face_normal");
        let mut hull: Option<crate::expression::Interval> = None;
        let mut locators = Vec::new();
        let mut exact = true;
        if faces {
            let service = self
                .vertical
                .as_ref()
                .ok_or_else(|| Self::missing(name, "vertical-extent"))?;
            let face = |of: &ObjectId| {
                service
                    .measure_face_normals(of, SurfaceFace::Top)
                    .map_err(|error| Self::unavailable(name, of, &error.to_string()))
            };
            let own = face(object)?;
            locators.push(own.evidence().locator.clone());
            exact &= own.evidence().exact;
            for other in &reached {
                let theirs = face(other)?;
                locators.push(theirs.evidence().locator.clone());
                exact &= theirs.evidence().exact;
                let value =
                    angles::between_faces(own.normals(), theirs.normals()).map_err(unavailable)?;
                hull = Some(hull.map_or(value, |hull| hull.hull(value)));
            }
        } else {
            let own = self.rectangle(name, object)?;
            locators.push(own.evidence().locator.clone());
            exact &= own.evidence().exact;
            for other in &reached {
                let theirs = self.rectangle(name, other)?;
                locators.push(theirs.evidence().locator.clone());
                exact &= theirs.evidence().exact;
                let value = if name == SKEW {
                    angles::skew(&own, &theirs)
                } else {
                    angles::between_axes(&own, &theirs)
                }
                .map_err(unavailable)?;
                hull = Some(hull.map_or(value, |hull| hull.hull(value)));
            }
        }
        let value = hull.ok_or(PropertyResolutionError::InvalidRequest)?;
        Ok(answer(value, locators.join("; "), exact))
    }

    /// The plan bearing of one of `object`'s axes, its locator, and
    /// whether it is exact.
    fn bearing(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<(crate::expression::Interval, String, bool), PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        // A placement is read only when stated exactly.
        let (direction, undirected, locator, mut exact) = match call.choice("axis") {
            Some("long") => {
                let rectangle = self.rectangle(name, object)?;
                let long = rectangle.long_axis().map_err(unavailable)?;
                (
                    rectangle.axes()[long],
                    true,
                    rectangle.evidence().locator.clone(),
                    rectangle.evidence().exact,
                )
            }
            axis => {
                let (frame, locator) = self.frame(name, object)?;
                let [x, y, _] = if axis == Some("own_y") {
                    frame.forward().components()
                } else {
                    frame.right().components()
                };
                ([x, y], false, locator, true)
            }
        };
        let north = if call.choice("reference") == Some("true_north") {
            let system = self
                .coordinates
                .as_ref()
                .ok_or_else(|| Self::missing(name, "coordinate-system"))?
                .coordinate_system(&object.source)
                .map_err(|error| unavailable(error.to_string()))?;
            exact &= system.evidence().exact;
            system
                .true_north()
                .ok_or_else(|| unavailable("the source states no true north".into()))?
        } else {
            [0.0, 1.0]
        };
        let value = angles::bearing(direction, north, undirected).map_err(unavailable)?;
        Ok((value, locator, exact))
    }

    /// A slope, fall or tilt of `object`, as an angle.
    fn surface(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let angle = |value: crate::expression::Interval, locator: String, exact: bool| {
            Answer::Value(
                value.lower,
                value.upper,
                Some(QuantityDimension::PlaneAngle),
                locator,
                exact,
            )
        };
        if name == INCLINATION {
            // A placement is read only when stated exactly.
            let (frame, locator) = self.frame(name, object)?;
            let (axis, from_vertical) = match call.choice("axis") {
                Some("own_x") => (frame.right(), false),
                Some("own_y") => (frame.forward(), false),
                _ => (frame.up(), true),
            };
            let tilt = surface::inclination(axis, from_vertical).map_err(unavailable)?;
            return Ok(angle(tilt, locator, true));
        }
        let (normals, evidence) = faces::face_normals(
            self.vertical
                .as_ref()
                .ok_or_else(|| Self::missing(name, "vertical-extent"))?,
            call,
            object,
        )
        .map_err(unavailable)?;
        let pieces = &normals[..];
        let value = match name {
            SLOPE => surface::slope(pieces),
            GRADIENT_DIRECTION => surface::gradient_direction(pieces),
            _ => {
                let key = if name == SLOPE_ALONG {
                    "direction"
                } else {
                    "axis"
                };
                let direction = match call.choice(key) {
                    Some("x") => [1.0, 0.0, 0.0],
                    Some("y") => [0.0, 1.0, 0.0],
                    Some("own_y") => self.frame(name, object)?.0.forward().components(),
                    _ => self.frame(name, object)?.0.right().components(),
                };
                let direction = surface::PlanDirection::of(direction).map_err(unavailable)?;
                if name == SLOPE_ALONG {
                    surface::slope_along(pieces, direction)
                } else {
                    surface::cross_fall(pieces, direction)
                }
            }
        }
        .map_err(unavailable)?;
        Ok(angle(value, evidence.locator, evidence.exact))
    }

    /// The objects `steps` reach from `object`, walked one after another
    /// over the whole project, `object` itself left out, and the locators
    /// the walk cites.
    fn reach(
        &self,
        name: &str,
        steps: &[PathSegment],
        object: &ObjectId,
    ) -> Result<(BTreeSet<ObjectId>, Vec<String>), PropertyResolutionError> {
        let service = self
            .relationships
            .as_ref()
            .ok_or_else(|| Self::missing(name, "relationship-selection"))?;
        let universe: Vec<ObjectId> = self.kinds.keys().cloned().collect();
        let mut frontier = BTreeSet::from([object.clone()]);
        let mut cited: Vec<String> = Vec::new();
        for step in steps {
            let mut reached = BTreeSet::new();
            for from in &frontier {
                let (found, evidence) = step
                    .walk(
                        service,
                        from,
                        &universe,
                        &universe,
                        step.chain(),
                        AbsentEndPolicy::Refuse,
                    )
                    .map_err(|error| match error {
                        RelationshipSelectionError::Unavailable(message) => {
                            Self::unavailable(name, object, &message)
                        }
                        RelationshipSelectionError::InvalidRequest => {
                            PropertyResolutionError::InvalidRequest
                        }
                        other => PropertyResolutionError::Incomplete(format!(
                            "`{MEASURED_SET}` value `{name}` of {object}: {other}"
                        )),
                    })?;
                cited.extend(evidence.iter().map(|e| e.locator.clone()));
                reached.extend(found);
            }
            reached.remove(object);
            frontier = reached;
        }
        Ok((frontier, cited))
    }

    /// The object's bottom above the elevation of the one level `steps`
    /// reach from it: the level's placement origin. No level reached is an
    /// exact absence; levels at different elevations are a conflict.
    fn bottom_above_level(
        &self,
        steps: &[PathSegment],
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = MEASURED_BOTTOM_ABOVE_LEVEL;
        let (frontier, cited) = self.reach(name, steps, object)?;
        if frontier.is_empty() {
            return Ok(Answer::Absent(format!(
                "the path reaches no level ({})",
                cited.join("; ")
            )));
        }
        let mut elevations = Vec::new();
        for level in &frontier {
            let (origin, _) = self.origin(name, level)?;
            elevations.push((level, origin[2]));
        }
        let (first, elevation) = elevations[0];
        #[allow(clippy::float_cmp)]
        if let Some((other, _)) = elevations.iter().find(|(_, other)| *other != elevation) {
            return Err(PropertyResolutionError::Conflicting(format!(
                "the path from {object} reaches levels {first} and {other} at different elevations"
            )));
        }
        let extent = self
            .vertical
            .as_ref()
            .ok_or_else(|| Self::missing(name, "vertical-extent"))?
            .measure_vertical_extent(object)
            .map_err(|error| Self::unavailable(name, object, &error.to_string()))?;
        // The difference rounds; keep the exact one inside. The level's
        // elevation is stated exactly, so the bottom's evidence decides.
        let (lower, upper) = bounds(
            span(
                extent.bottom().lower_metres(),
                extent.bottom().upper_metres(),
            )
            .minus(span(elevation, elevation)),
        )?;
        Ok(Answer::Value(
            lower,
            upper,
            Some(QuantityDimension::Length),
            format!("{} above level {first}", extent.evidence().locator),
            extent.evidence().exact,
        ))
    }

    /// The summed area of `space`'s boundaries against elements of `kind`
    /// (or a subtype), each as its surface interval. A boundary naming no
    /// element may be of any kind, so it refuses the sum.
    fn boundary_area(
        &self,
        kind: &str,
        plane: f64,
        space: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = MEASURED_BOUNDARY_AREA;
        let unavailable = |error: String| Self::unavailable(name, space, &error);
        let request = BoundaryCoverageRequest::try_new(space.clone(), plane)
            .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let coverage = self
            .boundaries
            .as_ref()
            .ok_or_else(|| Self::missing(name, "boundary-coverage"))?
            .measure_boundary_coverage(&request)
            .map_err(|error| unavailable(error.to_string()))?;
        // Summed outward, so the sum holds the exact one.
        let mut sum = span(0.0, 0.0);
        for boundary in coverage.boundaries() {
            let Some(element) = boundary.element() else {
                return Err(unavailable(format!(
                    "boundary {} names no bounding element, so its kind is unknown",
                    boundary.boundary()
                )));
            };
            if !self.is_kind(element, kind).map_err(unavailable)? {
                continue;
            }
            if let BoundaryPlacement::OnSurface { area } = boundary.placement() {
                sum = sum
                    .plus(span(area.lower_square_metres(), area.upper_square_metres()))
                    .map_err(|_| PropertyResolutionError::InvalidValue)?;
            }
        }
        Ok(Answer::Value(
            sum.lower,
            sum.upper,
            Some(QuantityDimension::Area),
            format!("{} kind={kind}", coverage.evidence().locator),
            coverage.evidence().exact,
        ))
    }

    /// Whether `element` is of the source kind `kind`, subtypes through the
    /// source's type hierarchy.
    fn is_kind(&self, element: &ObjectId, kind: &str) -> Result<bool, String> {
        let held = self
            .kinds
            .get(element)
            .ok_or_else(|| format!("bounding element {element} is not in the project"))?;
        if held.eq_ignore_ascii_case(kind) {
            return Ok(true);
        }
        let hierarchy = self.hierarchy.as_ref().ok_or_else(|| {
            "no type-hierarchy service is registered; subtype membership is unknown".to_owned()
        })?;
        hierarchy
            .is_a(&element.source, held, kind)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_with_their_parameters() {
        assert_eq!(
            parse("Extent_Z"),
            Ok(MeasuredName::Plain(MEASURED_EXTENT_Z))
        );
        assert_eq!(
            parse("bottom_above_level;path=IfcRelContainedInSpatialStructure:backward"),
            Ok(MeasuredName::BottomAboveLevel(vec![
                PathSegment::parse("IfcRelContainedInSpatialStructure:backward").unwrap()
            ]))
        );
        assert_eq!(
            parse("boundary_area;kind=IfcWall;plane=0.01"),
            Ok(MeasuredName::BoundaryArea {
                kind: "IfcWall".into(),
                plane: 0.01
            })
        );
        for invalid in [
            "height",
            "bottom_above_level",
            "boundary_area;plane=0.1",
            "boundary_area;kind=IfcWall;plane=-1",
            "extent_x;path=a",
            "bottom_above_level;path=a;path=b",
        ] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
    }
}
