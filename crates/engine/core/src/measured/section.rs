//! Sections along an alignment as values: the area and thickness of an
//! object's own body in the section at a station, and how many selected
//! bodies reach into a clearance envelope swept along a range, each as the
//! alignment service measures it ([`crate::section`]).
//!
//! The alignment of a section is found as the positions find theirs
//! (`alignment` kinds and an optional `path`); the clearance envelope is
//! measured on the alignment itself, over the bodies of the `bodies` kinds
//! in its source.

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Answer, Measures};
use crate::alignment::AlignmentError;
use crate::properties::PropertyResolutionError;
use crate::section::{EnvelopeRequest, Intrusion, SectionAxis, SectionPolygon, SectionRequest};

/// The area of the object's section at a station.
pub(super) const AREA: &str = "station_section_area";
/// The thickness of the object's section at a station.
pub(super) const THICKNESS: &str = "station_section_thickness";
/// The bodies reaching into a clearance envelope swept along a range.
pub(super) const INTRUSIONS: &str = "envelope_intrusions";

/// The names measured here.
pub(super) const NAMES: &[&str] = &[INTRUSIONS, AREA, THICKNESS];

/// At most this many bodies are named in a value's evidence.
const NAMED: usize = 5;

impl Measures {
    fn length_argument(call: &MeasuredCall, key: &str) -> Result<f64, PropertyResolutionError> {
        match call.argument(key) {
            Some(MeasuredArgument::Length(value)) => Ok(*value),
            _ => Err(PropertyResolutionError::InvalidRequest),
        }
    }

    fn refused(name: &str, object: &ObjectId, error: AlignmentError) -> PropertyResolutionError {
        match error {
            AlignmentError::InvalidMeasurement | AlignmentError::InexactEvidence => {
                PropertyResolutionError::InvalidValue
            }
            other => Self::unavailable(name, object, &other.to_string()),
        }
    }

    /// A section value or the envelope intrusions of `object`.
    pub(super) fn along_section(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let service = self
            .alignments
            .as_ref()
            .ok_or_else(|| Self::missing(name, "alignment"))?;
        if name == INTRUSIONS {
            return self.intrusions(call, object, service);
        }
        let Some(alignment) = self.alignment_of(call, object)? else {
            return Ok(Answer::Absent(format!(
                "no alignment of the stated kinds is selected for {object}"
            )));
        };
        let station = Self::length_argument(call, "station")?;
        let request = SectionRequest::try_new(alignment, station, vec![object.clone()])
            .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let section = service
            .measure_section(&request)
            .map_err(|error| Self::refused(name, object, error))?;
        let [body] = section.bodies() else {
            return Err(PropertyResolutionError::InvalidValue);
        };
        let locator = section.evidence().locator.clone();
        let exact = section.evidence().exact;
        let invalid = |_| PropertyResolutionError::InvalidValue;
        if name == AREA {
            let area = body.area().map_err(invalid)?;
            return Ok(Answer::Value(
                area.lower(),
                area.upper(),
                Some(QuantityDimension::Area),
                locator,
                exact && area.is_exact(),
            ));
        }
        let axis = if call.choice("direction") == Some("lateral") {
            SectionAxis::Lateral
        } else {
            SectionAxis::Up
        };
        Ok(match body.extent(axis).map_err(invalid)? {
            Some(extent) => Answer::Value(
                extent.lower(),
                extent.upper(),
                Some(QuantityDimension::Length),
                locator,
                exact && extent.is_exact(),
            ),
            None => Answer::Absent(format!(
                "the section at station {station} misses {object} ({locator})"
            )),
        })
    }

    /// How many bodies of the `bodies` kinds in the source of `alignment`
    /// reach into the envelope swept along it.
    fn intrusions(
        &self,
        call: &MeasuredCall,
        alignment: &ObjectId,
        service: &crate::alignment::AlignmentServiceHandle,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let Some(MeasuredArgument::Polygon(vertices)) = call.argument("envelope") else {
            return Err(PropertyResolutionError::InvalidRequest);
        };
        let envelope = SectionPolygon::try_new(vertices.clone())
            .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let range = (
            Self::length_argument(call, "from")?,
            Self::length_argument(call, "to")?,
        );
        let step = Self::length_argument(call, "step")?;
        if range.0 > range.1 {
            return Err(Self::unavailable(
                name,
                alignment,
                &format!(
                    "the range runs backwards, from station {} to station {}",
                    range.0, range.1
                ),
            ));
        }
        let mut bodies = self.of_kinds(call, "bodies", alignment)?;
        bodies.retain(|body| body.source == alignment.source);
        if bodies.is_empty() {
            // Nothing selected: a count of none, known exactly.
            return Ok(Answer::Value(
                0.0,
                0.0,
                None,
                format!(
                    "no body of the stated kinds in the source of {alignment}, so none intrudes"
                ),
                true,
            ));
        }
        let request = EnvelopeRequest::try_new(alignment.clone(), envelope, range, step, bodies)
            .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        let sweep = service
            .measure_envelope(&request)
            .map_err(|error| Self::refused(name, alignment, error))?;
        let (sure, possible) = sweep.count();
        let mut locator = sweep.evidence().locator.clone();
        let mut named = |label: &str, matching: &dyn Fn(&Intrusion) -> Option<String>| {
            let listed: Vec<String> = sweep
                .intrusions()
                .iter()
                .filter_map(|(body, intrusion)| {
                    matching(intrusion).map(|detail| format!("{body} {detail}"))
                })
                .collect();
            if listed.is_empty() {
                return;
            }
            let more = listed.len().saturating_sub(NAMED);
            locator.push_str("; ");
            locator.push_str(label);
            locator.push_str(": ");
            locator.push_str(&listed[..listed.len().min(NAMED)].join(", "));
            if more > 0 {
                locator.push_str(" and ");
                locator.push_str(&more.to_string());
                locator.push_str(" more");
            }
        };
        named("intruding", &|intrusion| match intrusion {
            Intrusion::Sure { distance } => Some(format!(
                "(proven inside at [{}, {}] m along it)",
                distance.lower(),
                distance.upper()
            )),
            _ => None,
        });
        named("undecided", &|intrusion| match intrusion {
            Intrusion::Possible(reason) => Some(format!("({reason})")),
            _ => None,
        });
        #[allow(clippy::cast_precision_loss)]
        let (sure, possible) = (sure as f64, possible as f64);
        Ok(Answer::Value(
            sure,
            possible,
            None,
            locator,
            sweep.evidence().exact,
        ))
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::cast_precision_loss)]
mod tests {
    use std::sync::Arc;

    use axioval_ir::{Evidence, Object, Project, PropertyValue, SourceId};

    use super::*;
    use crate::alignment::{
        AlignmentPosition, AlignmentRequest, AlignmentService, AlignmentServiceHandle,
    };
    use crate::properties::PropertyResolution;
    use crate::section::{BodySection, EnvelopeSweep, Section};
    use crate::{ServiceRegistry, measured_value};

    fn source() -> SourceId {
        SourceId::new("test", "bridge").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn evidence(locator: &str, exact: bool) -> Evidence {
        let mut evidence = Evidence::exact(source(), locator);
        evidence.exact = exact;
        evidence
    }

    /// A wall whose section at any station is a rectangle 2 m wide and
    /// 1 m high within 1 mm; beyond station 100 the plane misses it. Of the
    /// bodies along the axis the pier intrudes, the mast may and the kerb
    /// is clear; the post is unmeasured.
    struct Axis;

    impl AlignmentService for Axis {
        fn measure_alignment_position(
            &self,
            request: &AlignmentRequest,
        ) -> Result<AlignmentPosition, AlignmentError> {
            Err(AlignmentError::UnknownObject(request.object().clone()))
        }

        fn measure_section(&self, request: &SectionRequest) -> Result<Section, AlignmentError> {
            if request.station() > 200.0 {
                return Err(AlignmentError::OffRange(
                    "station 250 lies beyond the end at 200 m".into(),
                ));
            }
            let bodies = request
                .objects()
                .iter()
                .map(|object| {
                    if request.station() > 100.0 {
                        return BodySection::try_new(object.clone(), Vec::new(), Vec::new(), 0.001);
                    }
                    let corners = [[-1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [-1.0, 1.0]];
                    let cut: Vec<[[f64; 2]; 2]> =
                        (0..4).map(|i| [corners[i], corners[(i + 1) % 4]]).collect();
                    let band = cut.iter().map(|[a, b]| vec![*a, *b]).collect();
                    BodySection::try_new(object.clone(), cut, band, 0.001)
                })
                .collect::<Result<_, _>>()?;
            Section::try_new(request.clone(), bodies, evidence("cut of #175", false))
        }

        fn measure_envelope(
            &self,
            request: &EnvelopeRequest,
        ) -> Result<EnvelopeSweep, AlignmentError> {
            let intrusions = request
                .bodies()
                .iter()
                .map(|body| match body.local_id.as_str() {
                    "pier" => Ok((
                        body.clone(),
                        Intrusion::Sure {
                            distance: crate::AlignmentInterval::try_new(4.0, 4.0)?,
                        },
                    )),
                    "mast" => Ok((
                        body.clone(),
                        Intrusion::Possible("between 2 and 3 m".into()),
                    )),
                    "post" => Err(AlignmentError::Unavailable(format!("{body} is unmeasured"))),
                    _ => Ok((body.clone(), Intrusion::Clear)),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let exact = intrusions
                .iter()
                .all(|(_, intrusion)| !matches!(intrusion, Intrusion::Possible(_)));
            EnvelopeSweep::try_new(
                request.clone(),
                intrusions,
                evidence("sweep of #175", exact),
            )
        }
    }

    fn project(kinds: &[(&str, &str)]) -> Project {
        Project::new(
            kinds
                .iter()
                .map(|(local, kind)| Object::new(id(local), *kind))
                .collect(),
        )
        .unwrap()
    }

    fn services() -> ServiceRegistry {
        let mut services = ServiceRegistry::new();
        services
            .register(AlignmentServiceHandle::new(Arc::new(Axis)))
            .unwrap();
        services
    }

    fn interval(resolution: &PropertyResolution) -> (f64, f64, Option<QuantityDimension>, bool) {
        let PropertyResolution::Present(resolved) = resolution else {
            panic!("expected a value: {resolution:?}");
        };
        let exact = resolved.property().evidence.as_ref().unwrap().exact;
        match resolved.property().value {
            PropertyValue::Measured {
                lower,
                upper,
                dimension,
            } => (lower, upper, dimension, exact),
            PropertyValue::Quantity { value, dimension } => (value, value, Some(dimension), exact),
            PropertyValue::Integer(value) => (value as f64, value as f64, None, exact),
            ref other => panic!("not an interval: {other:?}"),
        }
    }

    const WALL: &[(&str, &str)] = &[("wall", "IfcWall"), ("axis", "IfcAlignment")];

    #[test]
    fn the_section_area_and_thickness_are_the_region_s_intervals() {
        let value = |name: &str| measured_value(&services(), &project(WALL), &id("wall"), name);
        let (lower, upper, dimension, exact) =
            interval(&value("station_section_area;alignment=IfcAlignment;station=5").unwrap());
        assert_eq!(dimension, Some(QuantityDimension::Area));
        assert!(lower < 2.0 && lower > 1.98 && upper > 2.0 && upper < 2.02 && !exact);
        let (lower, upper, dimension, _) =
            interval(&value("station_section_thickness;alignment=IfcAlignment;station=5").unwrap());
        assert_eq!(dimension, Some(QuantityDimension::Length));
        assert!(lower < 1.0 && lower > 0.99 && upper > 1.0 && upper < 1.01);
        let (lower, upper, _, _) = interval(
            &value("station_section_thickness;alignment=IfcAlignment;station=5;direction=lateral")
                .unwrap(),
        );
        assert!(lower < 2.0 && lower > 1.99 && upper > 2.0 && upper < 2.01);
        // A plane missing the body: no thickness, and an area of zero.
        assert!(matches!(
            value("station_section_thickness;alignment=IfcAlignment;station=150").unwrap(),
            PropertyResolution::Absent(_)
        ));
        let (lower, upper, _, _) =
            interval(&value("station_section_area;alignment=IfcAlignment;station=150").unwrap());
        assert_eq!((lower, upper), (0.0, 0.0));
        let Err(PropertyResolutionError::Unavailable(reason)) =
            value("station_section_area;alignment=IfcAlignment;station=250")
        else {
            panic!("off the range must be refused");
        };
        assert!(reason.contains("beyond the end"), "{reason}");
    }

    const ALONG: &[(&str, &str)] = &[
        ("axis", "IfcAlignment"),
        ("pier", "IfcPier"),
        ("mast", "IfcMast"),
        ("kerb", "IfcKerb"),
        ("post", "IfcPost"),
    ];

    fn intrusions(kinds: &str) -> Result<PropertyResolution, PropertyResolutionError> {
        measured_value(
            &services(),
            &project(ALONG),
            &id("axis"),
            &format!(
                "envelope_intrusions;bodies={kinds};envelope=-2:0,2:0,2:5,-2:5;from=0;to=10;step=1"
            ),
        )
    }

    #[test]
    fn intrusions_count_sure_up_to_possible_bodies_and_name_them() {
        let resolution = intrusions("IfcPier,IfcMast,IfcKerb").unwrap();
        let (lower, upper, dimension, exact) = interval(&resolution);
        assert_eq!((lower, upper, dimension, exact), (1.0, 2.0, None, false));
        let PropertyResolution::Present(resolved) = &resolution else {
            unreachable!()
        };
        let locator = &resolved.property().evidence.as_ref().unwrap().locator;
        assert!(
            locator.contains("intruding: ") && locator.contains("pier (proven inside at [4, 4] m"),
            "{locator}"
        );
        assert!(locator.contains("between 2 and 3 m"), "{locator}");
        // Decided bodies only: a whole count, exact.
        let (lower, upper, _, exact) = interval(&intrusions("IfcKerb").unwrap());
        assert_eq!((lower, upper, exact), (0.0, 0.0, true));
        // No body of the kinds: none intrudes.
        let (lower, upper, _, _) = interval(&intrusions("IfcTunnel").unwrap());
        assert_eq!((lower, upper), (0.0, 0.0));
        // An unmeasured body refuses the whole value.
        let Err(PropertyResolutionError::Unavailable(reason)) = intrusions("IfcPost,IfcKerb")
        else {
            panic!("an unmeasured body must refuse");
        };
        assert!(reason.contains("unmeasured"), "{reason}");
    }

    #[test]
    fn without_a_service_the_values_are_missing_for_the_run() {
        // A service on the trait's defaults.
        struct Bare;
        impl AlignmentService for Bare {
            fn measure_alignment_position(
                &self,
                request: &AlignmentRequest,
            ) -> Result<AlignmentPosition, AlignmentError> {
                Err(AlignmentError::UnknownObject(request.object().clone()))
            }
        }
        let error = measured_value(
            &ServiceRegistry::new(),
            &project(ALONG),
            &id("axis"),
            "envelope_intrusions;bodies=IfcPier;envelope=-2:0,2:0,2:5;from=0;to=10;step=1",
        )
        .unwrap_err();
        assert!(matches!(error, PropertyResolutionError::MissingService(_)));
        let mut services = ServiceRegistry::new();
        services
            .register(AlignmentServiceHandle::new(Arc::new(Bare)))
            .unwrap();
        let Err(PropertyResolutionError::Unavailable(reason)) = measured_value(
            &services,
            &project(WALL),
            &id("wall"),
            "station_section_area;alignment=IfcAlignment;station=5",
        ) else {
            panic!("the default must refuse");
        };
        assert!(reason.contains("cuts no sections"), "{reason}");
    }
}
