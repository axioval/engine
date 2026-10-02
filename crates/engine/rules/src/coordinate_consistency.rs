//! `coordinate-consistency`: the sources of a federation share one
//! coordinate system.
//!
//! Discipline models are coordinated in one frame: the same map conversion,
//! true north, world frame and site. This capability compares every source's
//! coordinate system ([`CoordinateSystemServiceHandle`]) with one reference
//! source's, within explicit tolerances, and reports each source that differs
//! as a finding against that source.
//!
//! Nothing a source leaves unstated is assumed to agree. A source without a
//! map conversion is not georeferenced: unless the rule requires one
//! (`require_map_conversion`, then a finding), it is not evaluated, since
//! whether it shares the reference's georeference is unknown. The same
//! comparison decides whether a host may put several sources' geometry into
//! one frame ([`CoordinateConsistency::shares_frame`]).

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CoordinateFrame, CoordinateSystemServiceHandle,
    MapConversion, NotEvaluatedReason, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext, SitePlacement, SourceCoordinateSystem, SourceDisciplines,
};
use axioval_ir::{Finding, Scope, SourceId};

use crate::comparison::{distance, plan_angle, rotation};
use crate::pairs::severity;
use crate::support::{Parameters, Unavailable, invalid, sources};

/// How far two coordinate systems may differ and still agree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateTolerance {
    length_metres: f64,
    angle_radians: f64,
    scale: f64,
}

impl Default for CoordinateTolerance {
    /// One millimetre, a hundredth of a degree, and an identical scale.
    fn default() -> Self {
        Self {
            length_metres: 0.001,
            angle_radians: 0.01_f64.to_radians(),
            scale: 0.0,
        }
    }
}

impl CoordinateTolerance {
    /// Tolerances for lengths (metres), angles (radians) and the map scale
    /// (absolute difference).
    ///
    /// # Errors
    ///
    /// A tolerance that is negative or not finite.
    pub fn try_new(length_metres: f64, angle_radians: f64, scale: f64) -> Result<Self, String> {
        for (name, value) in [
            ("length", length_metres),
            ("angle", angle_radians),
            ("scale", scale),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!(
                    "the {name} tolerance must be finite and not negative"
                ));
            }
        }
        Ok(Self {
            length_metres,
            angle_radians,
            scale,
        })
    }
}

/// One statement of a coordinate system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoordinateAspect {
    /// The frame model coordinates are stated in.
    WorldFrame,
    /// The plan direction of true north.
    TrueNorth,
    /// The map conversion and its target system.
    MapConversion,
    /// The site's placement.
    Site,
}

impl CoordinateAspect {
    /// Whether geometry stated in two sources' coordinates is in one frame
    /// only when this aspect agrees.
    fn places_geometry(self) -> bool {
        matches!(self, Self::WorldFrame | Self::MapConversion)
    }
}

/// How one source's coordinate system relates to a reference's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoordinateConsistency {
    /// Statements that surely differ beyond the tolerance, described.
    pub differences: Vec<(CoordinateAspect, String)>,
    /// Statements that cannot be compared, with the reason.
    pub unknown: Vec<(CoordinateAspect, String)>,
    /// Whether the reference and the other source state a map conversion.
    pub georeferenced: (bool, bool),
}

impl CoordinateConsistency {
    /// Whether geometry stated in the two sources' model coordinates lies in
    /// one frame: their world frames and map conversions agree, or neither
    /// states a map conversion and their world frames agree. Otherwise, why
    /// not.
    ///
    /// Two sources without a map conversion are taken to share their model
    /// coordinates; true north and the site do not move geometry.
    ///
    /// # Errors
    ///
    /// Why the frames are not known to be one, every reason joined.
    pub fn shares_frame(&self) -> Result<(), String> {
        let blocking: Vec<&str> = self
            .differences
            .iter()
            .chain(&self.unknown)
            .filter(|(aspect, _)| aspect.places_geometry())
            .map(|(_, reason)| reason.as_str())
            .collect();
        if !blocking.is_empty() {
            return Err(blocking.join("; "));
        }
        match self.georeferenced {
            (true, false) => Err("only the reference states a map conversion".into()),
            (false, true) => Err("only this source states a map conversion".into()),
            _ => Ok(()),
        }
    }
}

/// Compares `other`'s coordinate system with `reference`'s, statement by
/// statement, within `tolerance`. A difference is reported only when it
/// exceeds the tolerance; a statement one side makes and the other does not
/// is unknown, never agreement or difference, except the map conversion,
/// whose presence is reported in [`CoordinateConsistency::georeferenced`].
pub fn compare_coordinate_systems(
    reference: &SourceCoordinateSystem,
    other: &SourceCoordinateSystem,
    tolerance: CoordinateTolerance,
) -> CoordinateConsistency {
    let mut result = CoordinateConsistency {
        georeferenced: (reference.map().is_some(), other.map().is_some()),
        ..CoordinateConsistency::default()
    };
    match (reference.world(), other.world()) {
        (Some(a), Some(b)) => frames(
            CoordinateAspect::WorldFrame,
            "world frame",
            a,
            b,
            tolerance,
            &mut result,
        ),
        (a, b) => result.unknown.push((
            CoordinateAspect::WorldFrame,
            format!("the world frame is {}", one_sided(a.is_some(), b.is_some())),
        )),
    }
    match (reference.true_north(), other.true_north()) {
        (Some(a), Some(b)) => {
            let angle = plan_angle(a, b);
            if angle > tolerance.angle_radians {
                result.differences.push((
                    CoordinateAspect::TrueNorth,
                    format!("true north turned by {}", degrees(angle)),
                ));
            }
        }
        (None, None) => {}
        (a, b) => result.unknown.push((
            CoordinateAspect::TrueNorth,
            format!("true north is {}", one_sided(a.is_some(), b.is_some())),
        )),
    }
    if let (Some(a), Some(b)) = (reference.map(), other.map()) {
        maps(a, b, tolerance, &mut result);
    }
    match (reference.site(), other.site()) {
        (SitePlacement::Stated(a), SitePlacement::Stated(b)) => frames(
            CoordinateAspect::Site,
            "site placement",
            a,
            b,
            tolerance,
            &mut result,
        ),
        (SitePlacement::Absent, SitePlacement::Absent) => {}
        (SitePlacement::Unknown(reason), _) => result.unknown.push((
            CoordinateAspect::Site,
            format!("the reference's site placement is unknown: {reason}"),
        )),
        (_, SitePlacement::Unknown(reason)) => result.unknown.push((
            CoordinateAspect::Site,
            format!("the site placement is unknown: {reason}"),
        )),
        (a, _) => result.unknown.push((
            CoordinateAspect::Site,
            format!(
                "a site is {}",
                one_sided(
                    matches!(a, SitePlacement::Stated(_)),
                    !matches!(a, SitePlacement::Stated(_))
                )
            ),
        )),
    }
    result
}

fn one_sided(reference: bool, other: bool) -> &'static str {
    match (reference, other) {
        (true, false) => "stated only by the reference",
        (false, true) => "stated only by this source",
        _ => "stated by neither source",
    }
}

fn metres(value: f64) -> String {
    format!("{value:.4} m")
}

fn degrees(radians: f64) -> String {
    format!("{:.4}°", radians.to_degrees())
}

fn frames(
    aspect: CoordinateAspect,
    name: &str,
    a: &CoordinateFrame,
    b: &CoordinateFrame,
    tolerance: CoordinateTolerance,
    result: &mut CoordinateConsistency,
) {
    let shift = distance(a.origin_metres(), b.origin_metres());
    if shift > tolerance.length_metres {
        result
            .differences
            .push((aspect, format!("{name} moved by {}", metres(shift))));
    }
    let turn = rotation(a.axes(), b.axes());
    if turn > tolerance.angle_radians {
        result
            .differences
            .push((aspect, format!("{name} turned by {}", degrees(turn))));
    }
}

fn maps(
    a: &MapConversion,
    b: &MapConversion,
    tolerance: CoordinateTolerance,
    result: &mut CoordinateConsistency,
) {
    let aspect = CoordinateAspect::MapConversion;
    if a.target() != b.target() {
        let name = |map: &MapConversion| map.target().unwrap_or("(unnamed)").to_owned();
        result.differences.push((
            aspect,
            format!("map target `{}` instead of `{}`", name(b), name(a)),
        ));
    }
    match (a.offset_metres(), b.offset_metres()) {
        (Some(x), Some(y)) => {
            let shift = distance(x, y);
            if shift > tolerance.length_metres {
                result
                    .differences
                    .push((aspect, format!("map offset moved by {}", metres(shift))));
            }
        }
        // Identical statements in one unit are equal whatever the unit.
        #[allow(clippy::float_cmp)] // Identical statements, not measurements.
        _ if a.offset() == b.offset() && a.metres_per_map_unit() == b.metres_per_map_unit() => {}
        _ => result.unknown.push((
            aspect,
            "the map unit is not stated exactly, so the map offsets cannot be compared in metres"
                .into(),
        )),
    }
    let turn = plan_angle(a.x_axis(), b.x_axis());
    if turn > tolerance.angle_radians {
        result
            .differences
            .push((aspect, format!("map rotation turned by {}", degrees(turn))));
    }
    let scale = (a.scale() - b.scale()).abs();
    if scale > tolerance.scale {
        result.differences.push((
            aspect,
            format!("map scale {} instead of {}", b.scale(), a.scale()),
        ));
    }
}

/// Requires every source of the run to share the reference source's
/// coordinate system.
///
/// The reference is the one source of discipline `reference`, or, without
/// it, the first source in identity order. Each other source's world frame,
/// true north, map conversion (target system, offset, rotation and scale)
/// and site placement are compared with the reference's within
/// `length_tolerance` (metres, default 0.001), `angle_tolerance` (degrees,
/// default 0.01) and `scale_tolerance` (default 0); a source that differs is
/// a finding naming it and every difference. A statement only one of the two
/// makes, a map unit that is not known exactly and an unreadable coordinate
/// system leave the source not evaluated. A source without a map conversion
/// is a finding with `require_map_conversion`, and not evaluated otherwise.
pub struct CoordinateConsistencyCheck;

struct Declaration<'a> {
    reference: Option<&'a str>,
    tolerance: CoordinateTolerance,
    require_map: bool,
}

impl<'a> Declaration<'a> {
    fn read(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let length = parameters.number("length_tolerance")?.unwrap_or(0.001);
        let angle = parameters.number("angle_tolerance")?.unwrap_or(0.01);
        let scale = parameters.number("scale_tolerance")?.unwrap_or(0.0);
        let tolerance =
            CoordinateTolerance::try_new(length, angle.to_radians(), scale).map_err(invalid)?;
        Ok(Self {
            reference: parameters.string("reference")?,
            tolerance,
            require_map: parameters
                .boolean("require_map_conversion")?
                .unwrap_or(false),
        })
    }

    /// The reference source, and every other source.
    fn sources(&self, context: &RuleContext<'_>) -> Result<(SourceId, Vec<SourceId>), Unavailable> {
        let all: Vec<SourceId> = sources(context).into_iter().collect();
        if all.len() < 2 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the run checks {} source(s); coordinate consistency compares at least two",
                    all.len()
                ),
            ));
        }
        let reference = match self.reference {
            None => all[0].clone(),
            Some(discipline) => {
                let Some(disciplines) = context.services.get::<SourceDisciplines>() else {
                    return Err((
                        NotEvaluatedReason::MissingService,
                        "source disciplines are not available outside an evidence session".into(),
                    ));
                };
                let mut found = Vec::new();
                let mut undeclared = 0_usize;
                for source in &all {
                    match disciplines.of(source).map(axioval_ir::Discipline::as_str) {
                        Some(declared) if declared == discipline => found.push(source.clone()),
                        Some(_) => {}
                        None => undeclared += 1,
                    }
                }
                match <[SourceId; 1]>::try_from(found) {
                    Ok([source]) => source,
                    Err(found) if found.is_empty() && undeclared > 0 => {
                        return Err((
                            NotEvaluatedReason::NotRecorded,
                            format!(
                                "no source declares discipline `{discipline}`, and {undeclared} declare none"
                            ),
                        ));
                    }
                    Err(found) if found.is_empty() => {
                        return Err((
                            NotEvaluatedReason::IncompleteEvidence,
                            format!("no source of discipline `{discipline}` is checked"),
                        ));
                    }
                    Err(found) => {
                        return Err((
                            NotEvaluatedReason::InvalidEvidence,
                            format!(
                                "{} sources declare discipline `{discipline}`; the reference is one",
                                found.len()
                            ),
                        ));
                    }
                }
            }
        };
        let others = all
            .into_iter()
            .filter(|source| *source != reference)
            .collect();
        Ok((reference, others))
    }
}

impl RuleCapability for CoordinateConsistencyCheck {
    fn id(&self) -> &'static str {
        "axioval:capability.coordinate-consistency"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("reference", ParameterType::String),
            ParameterDescriptor::optional("length_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("angle_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("scale_tolerance", ParameterType::Number),
            ParameterDescriptor::optional("require_map_conversion", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = Declaration::read(rule).and_then(|declaration| {
            let Some(service) = context.services.get::<CoordinateSystemServiceHandle>() else {
                return Err((
                    NotEvaluatedReason::MissingService,
                    "no coordinate-system service is registered".into(),
                ));
            };
            let sources = declaration.sources(context)?;
            Ok((declaration, service, sources))
        });
        let (declaration, service, (reference, others)) = match declared {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("coordinate-consistency: {message}"),
                );
            }
        };
        let mut evaluation = CapabilityEvaluation::default();
        let base = match service.coordinate_system(&reference) {
            Ok(base) => base,
            Err(error) => {
                for source in others {
                    evaluation.push_source_not_evaluated(
                        source,
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "coordinate-consistency: the reference `{reference}`'s coordinate system cannot be read: {error}"
                        ),
                    );
                }
                return evaluation;
            }
        };
        if declaration.require_map && base.map().is_none() {
            evaluation.push_finding(
                Finding::new(
                    rule.id.clone(),
                    Scope::Source(reference.clone()),
                    severity(rule),
                    format!(
                        "`{reference}`, the reference, states no map conversion; the federation requires one"
                    ),
                )
                .with_evidence(vec![base.evidence().clone()]),
            );
        }
        for source in others {
            let system = match service.coordinate_system(&source) {
                Ok(system) => system,
                Err(error) => {
                    evaluation.push_source_not_evaluated(
                        source,
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("coordinate-consistency: {error}"),
                    );
                    continue;
                }
            };
            judge(
                rule,
                &declaration,
                (&reference, &base),
                (&source, &system),
                &mut evaluation,
            );
        }
        evaluation
    }
}

fn judge(
    rule: &CompiledRule,
    declaration: &Declaration<'_>,
    (reference, base): (&SourceId, &SourceCoordinateSystem),
    (source, system): (&SourceId, &SourceCoordinateSystem),
    evaluation: &mut CapabilityEvaluation,
) {
    let consistency = compare_coordinate_systems(base, system, declaration.tolerance);
    let mut differences: Vec<String> = consistency
        .differences
        .iter()
        .map(|(_, difference)| difference.clone())
        .collect();
    let mut unknown: Vec<String> = consistency
        .unknown
        .iter()
        .map(|(_, reason)| reason.clone())
        .collect();
    let mut not_recorded = false;
    match consistency.georeferenced {
        (_, false) if declaration.require_map => {
            differences.push("states no map conversion".into());
        }
        (true, true) => {}
        // The reference's own missing conversion is its own finding.
        (false, true) if declaration.require_map => {}
        (reference_map, source_map) => {
            not_recorded = true;
            unknown.push(format!(
                "{} no map conversion, so whether the georeferences agree is unknown",
                match (reference_map, source_map) {
                    (true, false) => "this source states",
                    (false, true) => "the reference states",
                    _ => "neither source states",
                }
            ));
        }
    }
    if !differences.is_empty() {
        evaluation.push_finding(
            Finding::new(
                rule.id.clone(),
                Scope::Source(source.clone()),
                severity(rule),
                format!(
                    "`{source}` does not share the coordinate system of `{reference}`: {}",
                    differences.join("; ")
                ),
            )
            .with_evidence(vec![base.evidence().clone(), system.evidence().clone()]),
        );
    } else if !unknown.is_empty() {
        let reason = if not_recorded && unknown.len() == 1 {
            NotEvaluatedReason::NotRecorded
        } else {
            NotEvaluatedReason::IncompleteEvidence
        };
        evaluation.push_source_not_evaluated(
            source.clone(),
            reason,
            format!(
                "coordinate-consistency: `{source}` against `{reference}`: {}",
                unknown.join("; ")
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use axioval_engine::MetricDirection;
    use axioval_ir::Evidence;

    use super::*;

    fn source() -> SourceId {
        SourceId::new("test", "a").unwrap()
    }

    fn frame(origin: [f64; 3]) -> CoordinateFrame {
        let axis = |v| MetricDirection::try_new(v).unwrap();
        CoordinateFrame::try_new(
            origin,
            axis([1.0, 0.0, 0.0]),
            axis([0.0, 1.0, 0.0]),
            axis([0.0, 0.0, 1.0]),
        )
        .unwrap()
    }

    fn system(world: [f64; 3], map: Option<f64>) -> SourceCoordinateSystem {
        let map = map.map(|easting| {
            MapConversion::try_new(
                Some("EPSG:25832".into()),
                [easting, 0.0, 0.0],
                [1.0, 0.0],
                1.0,
                Some(1.0),
            )
            .unwrap()
        });
        SourceCoordinateSystem::try_new(
            source(),
            Some(frame(world)),
            None,
            map,
            Evidence::exact(source(), "crs"),
        )
        .unwrap()
        .with_site(SitePlacement::Absent)
    }

    #[test]
    fn frames_are_shared_only_when_world_frame_and_map_agree() {
        let tolerance = CoordinateTolerance::default();
        let same = compare_coordinate_systems(
            &system([0.0; 3], Some(1.0)),
            &system([0.0; 3], Some(1.0)),
            tolerance,
        );
        assert_eq!(
            same,
            CoordinateConsistency {
                georeferenced: (true, true),
                ..CoordinateConsistency::default()
            }
        );
        assert!(same.shares_frame().is_ok());
        let shifted = compare_coordinate_systems(
            &system([0.0; 3], Some(1.0)),
            &system([0.0; 3], Some(2.0)),
            tolerance,
        );
        assert_eq!(
            shifted.differences,
            vec![(
                CoordinateAspect::MapConversion,
                "map offset moved by 1.0000 m".to_owned()
            )]
        );
        assert!(shifted.shares_frame().is_err());
        let local =
            compare_coordinate_systems(&system([0.0; 3], None), &system([0.0; 3], None), tolerance);
        assert!(local.shares_frame().is_ok(), "neither is georeferenced");
        let one_sided = compare_coordinate_systems(
            &system([0.0; 3], Some(1.0)),
            &system([0.0; 3], None),
            tolerance,
        );
        assert!(one_sided.shares_frame().is_err());
    }

    #[test]
    fn a_shift_within_the_tolerance_agrees() {
        let consistency = compare_coordinate_systems(
            &system([0.0; 3], Some(1.0)),
            &system([0.0005, 0.0, 0.0], Some(1.0005)),
            CoordinateTolerance::default(),
        );
        assert!(consistency.differences.is_empty(), "{consistency:?}");
    }
}
