//! Values the engine measures from geometry services and answers as the
//! reserved property set [`axioval_ir::MEASURED_SET`].
//!
//! Each value is an interval sure to hold the exact value: a point is a
//! quantity with exact evidence, anything wider a
//! [`PropertyValue::Measured`] whose evidence is never exact. A missing
//! service is a fact of the run, never of the object.
//!
//! Two names take parameters after the name, `;`-separated `key=value`
//! pairs: `bottom_above_level;path=<steps>` and
//! `boundary_area;kind=<kind>[;plane=<metres>]`. `level_height` is stated by
//! the source, not measured, and is answered by the host's resolver.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_ir::{
    Evidence, MEASURED_AREA, MEASURED_BOTTOM, MEASURED_BOTTOM_ABOVE_LEVEL, MEASURED_BOUNDARY_AREA,
    MEASURED_EXTENT_X, MEASURED_EXTENT_Y, MEASURED_EXTENT_Z, MEASURED_LEVEL_HEIGHT, MEASURED_SET,
    MEASURED_TOP, MEASURED_VOLUME, MEASURED_X, MEASURED_Y, MEASURED_Z, ObjectId, Project, Property,
    PropertyValue, QuantityDimension,
};

mod angles;
mod surface;

use axioval_ir::measured::{
    ANGLE_TO, BEARING, CROSS_FALL, GRADIENT_DIRECTION, INCLINATION, MeasuredCall, SKEW, SLOPE,
    SLOPE_ALONG,
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
use crate::vertical_extent::{SurfaceFace, VerticalExtentServiceHandle};

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
}

/// Parses a name in the measured set through the registry
/// ([`axioval_ir::measured`]), or says why it is none.
pub(crate) fn parse(name: &str) -> Result<MeasuredName, String> {
    use axioval_ir::measured::MeasuredArgument;
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
        name if call.descriptor.parameters.is_empty() => MeasuredName::Plain(name),
        name => return Err(format!("`{name}` is registered but not measured")),
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

/// A measured answer before it becomes a property.
enum Answer {
    Value(f64, f64, QuantityDimension, String),
    Absent(String),
}

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
    coordinates: Option<CoordinateSystemServiceHandle>,
    host: Option<PropertyResolutionServiceHandle>,
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
            coordinates: services.get::<CoordinateSystemServiceHandle>().cloned(),
            host: host.cloned(),
            kinds: Arc::new(
                project
                    .objects()
                    .map(|object| (object.id.clone(), object.kind().to_owned()))
                    .collect(),
            ),
        }
    }

    /// Answers one request in the measured set.
    pub(crate) fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let name =
            parse(request.property()).map_err(|_| PropertyResolutionError::InvalidRequest)?;
        if name == MeasuredName::Plain(MEASURED_LEVEL_HEIGHT) {
            // Stated by the source, not measured.
            return match &self.host {
                Some(host) => host.resolve(request),
                None => Err(PropertyResolutionError::MissingService(
                    "no property-resolution service states `level_height`".into(),
                )),
            };
        }
        let object = request.object_id();
        let locate = |locator: String| {
            format!(
                "{MEASURED_SET}/{}: {locator}",
                request.property().to_ascii_lowercase()
            )
        };
        let (lower, upper, dimension, locator) = match self.measure(&name, object)? {
            Answer::Value(lower, upper, dimension, locator) => (lower, upper, dimension, locator),
            Answer::Absent(locator) => {
                return Ok(PropertyResolution::Absent(
                    CompletePropertyAbsenceEvidence::try_new(
                        request.clone(),
                        Evidence::exact(object.source.clone(), locate(locator)),
                    )?,
                ));
            }
        };
        if !(lower.is_finite() && upper.is_finite() && lower <= upper) {
            return Err(PropertyResolutionError::InvalidValue);
        }
        let exact = lower.to_bits() == upper.to_bits();
        let value = if exact {
            PropertyValue::Quantity {
                value: lower,
                dimension,
            }
        } else {
            PropertyValue::Measured {
                lower,
                upper,
                dimension,
            }
        };
        let mut evidence = Evidence::exact(object.source.clone(), locate(locator));
        evidence.exact = exact;
        let property = Property::new(MEASURED_SET, request.property(), value)
            .map_err(|_| PropertyResolutionError::InvalidRequest)?
            .with_evidence(evidence);
        Ok(PropertyResolution::Present(ResolvedProperty::try_new(
            request.clone(),
            property,
        )?))
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
        }
    }

    fn plain(&self, name: &str, object: &ObjectId) -> Result<Answer, PropertyResolutionError> {
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let length = QuantityDimension::Length;
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
                Ok(Answer::Value(lower, upper, length, locator))
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
                ))
            }
            MEASURED_X | MEASURED_Y | MEASURED_Z => {
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
                    QuantityDimension::Area,
                    area.evidence().locator.clone(),
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
                    QuantityDimension::Volume,
                    body.evidence().locator.clone(),
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
        let answer = |value: crate::expression::Interval, locator: String| {
            Answer::Value(
                value.lower,
                value.upper,
                QuantityDimension::PlaneAngle,
                locator,
            )
        };
        if name == BEARING {
            let (direction, undirected, locator) = match call.choice("axis") {
                Some("long") => {
                    let rectangle = self.rectangle(name, object)?;
                    let long = rectangle.long_axis().map_err(unavailable)?;
                    (
                        rectangle.axes()[long],
                        true,
                        rectangle.evidence().locator.clone(),
                    )
                }
                axis => {
                    let (frame, locator) = self.frame(name, object)?;
                    let [x, y, _] = if axis == Some("own_y") {
                        frame.forward().components()
                    } else {
                        frame.right().components()
                    };
                    ([x, y], false, locator)
                }
            };
            let north = if call.choice("reference") == Some("true_north") {
                self.coordinates
                    .as_ref()
                    .ok_or_else(|| Self::missing(name, "coordinate-system"))?
                    .coordinate_system(&object.source)
                    .map_err(|error| unavailable(error.to_string()))?
                    .true_north()
                    .ok_or_else(|| unavailable("the source states no true north".into()))?
            } else {
                [0.0, 1.0]
            };
            let value = angles::bearing(direction, north, undirected).map_err(unavailable)?;
            return Ok(answer(value, locator));
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
            for other in &reached {
                let theirs = face(other)?;
                locators.push(theirs.evidence().locator.clone());
                let value =
                    angles::between_faces(own.normals(), theirs.normals()).map_err(unavailable)?;
                hull = Some(hull.map_or(value, |hull| hull.hull(value)));
            }
        } else {
            let own = self.rectangle(name, object)?;
            locators.push(own.evidence().locator.clone());
            for other in &reached {
                let theirs = self.rectangle(name, other)?;
                locators.push(theirs.evidence().locator.clone());
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
        Ok(answer(value, locators.join("; ")))
    }

    /// A slope, fall or tilt of `object`, as an angle.
    fn surface(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let angle = |value: crate::expression::Interval, locator: String| {
            Answer::Value(
                value.lower,
                value.upper,
                QuantityDimension::PlaneAngle,
                locator,
            )
        };
        if name == INCLINATION {
            let (frame, locator) = self.frame(name, object)?;
            let (axis, from_vertical) = match call.choice("axis") {
                Some("own_x") => (frame.right(), false),
                Some("own_y") => (frame.forward(), false),
                _ => (frame.up(), true),
            };
            let tilt = surface::inclination(axis, from_vertical).map_err(unavailable)?;
            return Ok(angle(tilt, locator));
        }
        let face = match call.choice("face") {
            Some("bottom") => SurfaceFace::Bottom,
            _ => SurfaceFace::Top,
        };
        let normals = self
            .vertical
            .as_ref()
            .ok_or_else(|| Self::missing(name, "vertical-extent"))?
            .measure_face_normals(object, face)
            .map_err(|error| unavailable(error.to_string()))?;
        let pieces = normals.normals();
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
        Ok(angle(value, normals.evidence().locator.clone()))
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
        Ok(Answer::Value(
            extent.bottom().lower_metres() - elevation,
            extent.bottom().upper_metres() - elevation,
            QuantityDimension::Length,
            format!("{} above level {first}", extent.evidence().locator),
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
        let (mut lower, mut upper) = (0.0, 0.0);
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
                lower += area.lower_square_metres();
                upper += area.upper_square_metres();
            }
        }
        Ok(Answer::Value(
            lower,
            upper,
            QuantityDimension::Area,
            format!("{} kind={kind}", coverage.evidence().locator),
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
