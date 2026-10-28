//! The elements of an IFC4X3 alignment's layouts
//! (`AlignmentService::measure_alignment_elements`), read from its
//! `IfcAlignmentHorizontal` and `IfcAlignmentVertical` segments as they
//! state them, in nesting order, without the zero-length segment closing a
//! layout.
//!
//! A horizontal segment's radii are its `StartRadiusOfCurvature` and
//! `EndRadiusOfCurvature`, `0` a straight end; its stations run from the
//! sum of the lengths before it. A vertical segment runs from its
//! `StartDistAlong` over its `HorizontalLength`; a circular arc or a
//! parabola states one `RadiusOfCurvature` at both ends (a parabola's only
//! where its gradients change over its length as that radius says), a
//! constant gradient is straight, and a vertical clothoid states neither
//! end curvature, so its ends are unread. Stations are labelled through
//! the alignment's station equations, as positions are.

use axioval_engine::{
    AlignmentElement, AlignmentElementKind, AlignmentElements, AlignmentElementsRequest,
    AlignmentError, AlignmentInterval, AlignmentLayout, ElementEnd,
};
use ifc_alignment::{
    AlignmentUnits, AlignmentView, HorizontalSegmentType, Stationing, VerticalLayout,
    VerticalSegmentType, read_horizontal_segment,
};
use ifc_model::EntityId;

use super::{IfcAlignmentService, Parsed, evidence, stations, widen};

/// Gradients and lengths stated for a parabola may disagree with its
/// stated radius by this share of it, the rounding of written decimals.
const AGREEMENT: f64 = 1e-6;

/// One element as read, its stations not yet labelled.
struct Read {
    kind: AlignmentElementKind,
    type_name: String,
    locator: String,
    /// Plan distances where it starts and ends.
    start: (f64, f64),
    end: (f64, f64),
    length: f64,
    ends: (ElementEnd, ElementEnd),
    gradients: Option<(f64, f64)>,
}

/// Both layouts of one alignment, each its elements or why they cannot be
/// read.
pub(super) struct Layouts {
    horizontal: Result<(String, Vec<AlignmentElement>), String>,
    vertical: Result<(String, Vec<AlignmentElement>), String>,
}

/// `a + b`: a point where the sum is exact, else widened.
fn sum(a: f64, b: f64) -> (f64, f64) {
    let total = a + b;
    let virtual_b = total - a;
    let error = (a - (total - virtual_b)) + (b - virtual_b);
    if error == 0.0 {
        (total, total)
    } else {
        widen((total, total))
    }
}

/// A plan end: straight at a zero radius, IFC's convention.
fn plan_end(radius: f64) -> ElementEnd {
    if radius == 0.0 {
        ElementEnd::Straight
    } else {
        ElementEnd::Radius(radius)
    }
}

fn horizontal(
    source: &Parsed,
    layout: EntityId,
    units: AlignmentUnits,
) -> Result<Vec<Read>, String> {
    let view = AlignmentView::for_model(&source.model).map_err(|error| error.to_string())?;
    let ids = view
        .layout_segments(layout)
        .map_err(|error| error.to_string())?;
    let count = ids.len();
    let mut out = Vec::with_capacity(count);
    let mut at = (0.0, 0.0);
    for (index, id) in ids.into_iter().enumerate() {
        let segment = read_horizontal_segment(&source.model, id, units)
            .map_err(|error| format!("segment {id} cannot be read ({error})"))?;
        if segment.segment_length == 0.0 && index + 1 == count && index > 0 {
            break;
        }
        let type_name = segment.segment_type.source_name().to_owned();
        let (start, end) = (plan_end(segment.start_radius), plan_end(segment.end_radius));
        let (kind, ends) = match &segment.segment_type {
            HorizontalSegmentType::Line => {
                let straight =
                    matches!((&start, &end), (ElementEnd::Straight, ElementEnd::Straight));
                let ends = if straight {
                    (start, end)
                } else {
                    let why = "a LINE that states a radius has no curvature to read".to_owned();
                    (ElementEnd::Unread(why.clone()), ElementEnd::Unread(why))
                };
                (AlignmentElementKind::Straight, ends)
            }
            HorizontalSegmentType::CircularArc => {
                #[allow(clippy::float_cmp)] // Stated values: the same or not.
                let ends =
                    if segment.start_radius == segment.end_radius && segment.start_radius != 0.0 {
                        (start, end)
                    } else {
                        let why =
                            "a CIRCULARARC whose radii differ or are zero has no curvature to read"
                                .to_owned();
                        (ElementEnd::Unread(why.clone()), ElementEnd::Unread(why))
                    };
                (AlignmentElementKind::Arc, ends)
            }
            HorizontalSegmentType::Transition(name) if name == "CLOTHOID" => {
                (AlignmentElementKind::Clothoid, (start, end))
            }
            HorizontalSegmentType::Transition(_) => {
                (AlignmentElementKind::Transition, (start, end))
            }
            _ => (AlignmentElementKind::Other, (start, end)),
        };
        let end_at = (
            sum(at.0, segment.segment_length).0,
            sum(at.1, segment.segment_length).1,
        );
        out.push(Read {
            kind,
            locator: format!("{id} ({type_name})"),
            type_name,
            start: at,
            end: end_at,
            length: segment.segment_length,
            ends,
            gradients: None,
        });
        // A start beyond the first carries the rounding of the sums.
        at = end_at;
    }
    if out.is_empty() {
        return Err("its horizontal layout holds no segment of positive length".into());
    }
    Ok(out)
}

fn vertical(source: &Parsed, layout: EntityId, units: AlignmentUnits) -> Result<Vec<Read>, String> {
    let resolved =
        VerticalLayout::resolve(&source.model, layout, units).map_err(|error| error.to_string())?;
    let segments = resolved.segments();
    let count = segments.len();
    let mut out = Vec::with_capacity(count);
    for (index, segment) in segments.iter().enumerate() {
        if segment.horizontal_length == 0.0 && index + 1 == count && index > 0 {
            break;
        }
        let type_name = match &segment.predefined_type {
            VerticalSegmentType::ConstantGradient => "CONSTANTGRADIENT".to_owned(),
            VerticalSegmentType::CircularArc => "CIRCULARARC".to_owned(),
            VerticalSegmentType::ParabolicArc => "PARABOLICARC".to_owned(),
            VerticalSegmentType::Clothoid => "CLOTHOID".to_owned(),
            VerticalSegmentType::UserDefined => "USERDEFINED".to_owned(),
            VerticalSegmentType::NotDefined => "NOTDEFINED".to_owned(),
            VerticalSegmentType::Other(name) => name.clone(),
            other => format!("{other:?}"),
        };
        let unread = |why: String| (ElementEnd::Unread(why.clone()), ElementEnd::Unread(why));
        let radius = segment.radius_of_curvature.filter(|radius| *radius != 0.0);
        let (kind, ends) = match (&segment.predefined_type, radius) {
            (VerticalSegmentType::ConstantGradient, None) => (
                AlignmentElementKind::Straight,
                (ElementEnd::Straight, ElementEnd::Straight),
            ),
            (VerticalSegmentType::ConstantGradient, Some(radius)) => (
                AlignmentElementKind::Straight,
                unread(format!(
                    "a CONSTANTGRADIENT that states a radius of {radius} m has no curvature to \
                     read"
                )),
            ),
            (VerticalSegmentType::CircularArc, Some(radius)) => (
                AlignmentElementKind::Arc,
                (ElementEnd::Radius(radius), ElementEnd::Radius(radius)),
            ),
            (VerticalSegmentType::ParabolicArc, Some(radius)) => {
                // z'' = (g2 - g1) / L is the parabola's law; its radius
                // must say the same.
                let change = segment.end_gradient - segment.start_gradient;
                let implied = segment.horizontal_length / change;
                let ends = if change != 0.0
                    && implied.is_finite()
                    && (radius - implied).abs() <= AGREEMENT * implied.abs()
                {
                    (ElementEnd::Radius(radius), ElementEnd::Radius(radius))
                } else {
                    unread(format!(
                        "it states a radius of {radius} m, but its gradients change over its \
                         length as for {implied} m"
                    ))
                };
                (AlignmentElementKind::Parabola, ends)
            }
            (VerticalSegmentType::CircularArc | VerticalSegmentType::ParabolicArc, None) => (
                if segment.predefined_type == VerticalSegmentType::CircularArc {
                    AlignmentElementKind::Arc
                } else {
                    AlignmentElementKind::Parabola
                },
                unread("it states no radius of curvature".into()),
            ),
            (VerticalSegmentType::Clothoid, _) => (
                AlignmentElementKind::Clothoid,
                unread("a vertical clothoid states neither its start nor its end curvature".into()),
            ),
            _ => (
                AlignmentElementKind::Other,
                unread("its type defines no curvature".into()),
            ),
        };
        out.push(Read {
            kind,
            locator: format!("{} ({type_name})", segment.entity),
            type_name,
            start: (segment.start_dist_along, segment.start_dist_along),
            end: sum(segment.start_dist_along, segment.horizontal_length),
            length: segment.horizontal_length,
            ends,
            gradients: Some((segment.start_gradient, segment.end_gradient)),
        });
    }
    if out.is_empty() {
        return Err("its vertical layout holds no segment of positive length".into());
    }
    Ok(out)
}

/// `read`'s elements, labelled with stations.
fn labelled(
    read: Vec<Read>,
    stationing: Option<&Stationing>,
) -> Result<Vec<AlignmentElement>, String> {
    let interval =
        |(low, high): (f64, f64)| AlignmentInterval::try_new(low, high).map_err(|e| e.to_string());
    read.into_iter()
        .map(|element| {
            let from = interval(stations(stationing, element.start)?)?;
            let to = interval(stations(stationing, element.end)?)?;
            AlignmentElement::try_new(
                element.kind,
                element.type_name,
                element.locator,
                (from, to),
                element.length,
                element.ends,
                element.gradients,
            )
            .map_err(|error| format!("{error}"))
        })
        .collect()
}

impl Layouts {
    /// The layouts `alignment` of `source` nests.
    pub(super) fn read(source: &Parsed, alignment: EntityId) -> Self {
        let units = match source.units() {
            Ok(units) => AlignmentUnits {
                length_to_metres: units.length_to_metres,
                angle_to_radians: units.angle_to_radians,
            },
            Err(reason) => {
                return Self {
                    horizontal: Err(reason.clone()),
                    vertical: Err(reason),
                };
            }
        };
        let stationing = match Stationing::resolve(&source.model, alignment, units) {
            Ok(stationing) if stationing.equations().is_empty() => Ok(None),
            Ok(stationing) => Ok(Some(stationing)),
            Err(error) => Err(format!("its stationing cannot be read ({error})")),
        };
        let hierarchy = AlignmentView::for_model(&source.model)
            .and_then(|view| view.hierarchy(alignment))
            .map_err(|error| format!("its layouts cannot be read ({error})"));
        let layout = |which: AlignmentLayout| -> Result<(String, Vec<AlignmentElement>), String> {
            let hierarchy = hierarchy.as_ref().map_err(Clone::clone)?;
            let stationing = stationing.as_ref().map_err(Clone::clone)?;
            let (sole, kind) = match which {
                AlignmentLayout::Horizontal => {
                    (hierarchy.sole_horizontal(), "IFCALIGNMENTHORIZONTAL")
                }
                AlignmentLayout::Vertical => (hierarchy.sole_vertical(), "IFCALIGNMENTVERTICAL"),
            };
            let entity = sole
                .map_err(|error| format!("its {} layout cannot be read ({error})", which.name()))?
                .ok_or_else(|| format!("it nests no {} layout", which.name()))?;
            let read = match which {
                AlignmentLayout::Horizontal => horizontal(source, entity, units),
                AlignmentLayout::Vertical => vertical(source, entity, units),
            }
            .map_err(|error| {
                format!(
                    "its {} layout {entity} cannot be read: {error}",
                    which.name()
                )
            })?;
            let locator = format!(
                "{} layout {entity} ({kind}) of {alignment} (IFCALIGNMENT)",
                which.name()
            );
            Ok((locator, labelled(read, stationing.as_ref())?))
        };
        Self {
            horizontal: layout(AlignmentLayout::Horizontal),
            vertical: layout(AlignmentLayout::Vertical),
        }
    }
}

impl IfcAlignmentService {
    /// The elements of one layout of an alignment (module documentation).
    pub(super) fn elements(
        &self,
        request: &AlignmentElementsRequest,
    ) -> Result<AlignmentElements, AlignmentError> {
        let alignment = request.alignment();
        let layouts = self
            .layouts
            .get(alignment)
            .ok_or_else(|| AlignmentError::NotAlignment(alignment.clone()))?;
        let layout = match request.layout() {
            AlignmentLayout::Horizontal => &layouts.horizontal,
            AlignmentLayout::Vertical => &layouts.vertical,
        };
        let (locator, elements) = layout.as_ref().map_err(|reason| {
            AlignmentError::Unavailable(format!(
                "the {} elements of {alignment} cannot be read: {reason}",
                request.layout().name()
            ))
        })?;
        let exact = elements
            .iter()
            .all(|element| element.from().is_exact() && element.to().is_exact());
        AlignmentElements::try_new(
            request.clone(),
            elements.clone(),
            evidence(&alignment.source, locator.clone(), exact),
        )
    }
}

#[cfg(test)]
mod tests {
    use axioval_engine::{
        AlignmentParameter, AlignmentParameterRequest, AlignmentProfileRequest, AlignmentRequest,
        AlignmentServiceHandle, ProfileExtreme,
    };
    use axioval_ir::ObjectId;

    use super::*;

    /// The plan distance a crest of radius 5000 m takes from +2 % to -2 %:
    /// `R (sin t0 - sin t1)`, `tan t0 = 0.02`.
    fn crest_length() -> f64 {
        2.0 * 5000.0 * 0.02 / 1.0004_f64.sqrt()
    }

    /// An IFC4X3 alignment by its layouts: a plan of a 100 m line, a
    /// 100 m clothoid from straight into a left-hand radius of 500 m and
    /// 100 m of that arc; a vertical crest arc of radius 5000 m from
    /// +2 % to -2 % at 50 m from the start, then -2 % to 300 m. A proxy
    /// stands above the crest's midpoint (#604).
    fn crest() -> String {
        let (radius, length) = (500.0_f64, 100.0_f64);
        // The clothoid's end by Simpson's rule (its seam is not verified).
        let steps = 4000_u32;
        let width = length / f64::from(steps);
        let heading = |s: f64| s * s / (2.0 * radius * length);
        let (mut x, mut y) = (0.0, 0.0);
        for step in 0..=steps {
            let s = f64::from(step) * width;
            let weight = if step == 0 || step == steps {
                1.0
            } else if step % 2 == 1 {
                4.0
            } else {
                2.0
            };
            x += weight * heading(s).cos();
            y += weight * heading(s).sin();
        }
        let (x, y) = (100.0 + x * width / 3.0, y * width / 3.0);
        let turned = length / (2.0 * radius);
        let (cx, cy) = (x - radius * turned.sin(), y + radius * turned.cos());
        let end = turned + 100.0 / radius;
        let (ex, ey) = (cx + radius * end.sin(), cy - radius * end.cos());
        let crest = crest_length();
        let tail = 300.0 - crest;
        let low = 50.0 - 0.02 * tail;
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
             FILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4X3_ADD2'));\nENDSEC;\nDATA;\n\
             #4=IFCCARTESIANPOINT((0.,0.,0.));\n\
             #5=IFCAXIS2PLACEMENT3D(#4,$,$);\n\
             #7=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-6,#5,$);\n\
             #9001=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
             #9002=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);\n\
             #9003=IFCUNITASSIGNMENT((#9001,#9002));\n\
             #9004=IFCPROJECT('0000000000000000009004',$,'P',$,$,$,$,(#7),#9003);\n\
             #100=IFCCARTESIANPOINT((0.,0.));\n\
             #101=IFCALIGNMENTHORIZONTALSEGMENT($,$,#100,0.,0.,0.,100.,$,.LINE.);\n\
             #102=IFCCARTESIANPOINT((100.,0.));\n\
             #103=IFCALIGNMENTHORIZONTALSEGMENT($,$,#102,0.,0.,500.,100.,$,.CLOTHOID.);\n\
             #104=IFCCARTESIANPOINT(({x:?},{y:?}));\n\
             #105=IFCALIGNMENTHORIZONTALSEGMENT($,$,#104,{turned:?},500.,500.,100.,$,.CIRCULARARC.);\n\
             #106=IFCCARTESIANPOINT(({ex:?},{ey:?}));\n\
             #107=IFCALIGNMENTHORIZONTALSEGMENT($,$,#106,{end:?},0.,0.,0.,$,.LINE.);\n\
             #111=IFCALIGNMENTSEGMENT('seg-h-0000000000000001',$,$,$,$,$,$,#101);\n\
             #112=IFCALIGNMENTSEGMENT('seg-h-0000000000000002',$,$,$,$,$,$,#103);\n\
             #113=IFCALIGNMENTSEGMENT('seg-h-0000000000000003',$,$,$,$,$,$,#105);\n\
             #114=IFCALIGNMENTSEGMENT('seg-h-0000000000000004',$,$,$,$,$,$,#107);\n\
             #121=IFCALIGNMENTHORIZONTAL('align-horiz-0000000001',$,$,$,$,$,$);\n\
             #122=IFCRELNESTS('nest-h-000000000000001',$,$,$,#121,(#111,#112,#113,#114));\n\
             #200=IFCALIGNMENTVERTICALSEGMENT($,$,0.,{crest:?},50.,0.02,-0.02,-5000.,.CIRCULARARC.);\n\
             #201=IFCALIGNMENTVERTICALSEGMENT($,$,{crest:?},{tail:?},50.,-0.02,-0.02,$,.CONSTANTGRADIENT.);\n\
             #202=IFCALIGNMENTVERTICALSEGMENT($,$,300.,0.,{low:?},-0.02,-0.02,$,.CONSTANTGRADIENT.);\n\
             #211=IFCALIGNMENTSEGMENT('seg-v-0000000000000001',$,$,$,$,$,$,#200);\n\
             #212=IFCALIGNMENTSEGMENT('seg-v-0000000000000002',$,$,$,$,$,$,#201);\n\
             #213=IFCALIGNMENTSEGMENT('seg-v-0000000000000003',$,$,$,$,$,$,#202);\n\
             #221=IFCALIGNMENTVERTICAL('align-vert-00000000001',$,$,$,$,$,$);\n\
             #222=IFCRELNESTS('nest-v-000000000000001',$,$,$,#221,(#211,#212,#213));\n\
             #401=IFCALIGNMENT('0000000000000000000401',$,$,$,$,$,$,$);\n\
             #402=IFCRELNESTS('nest-a-000000000000001',$,$,$,#401,(#121,#221));\n\
             #601=IFCCARTESIANPOINT(({half:?},0.,60.));\n\
             #602=IFCAXIS2PLACEMENT3D(#601,$,$);\n\
             #603=IFCLOCALPLACEMENT($,#602);\n\
             #604=IFCBUILDINGELEMENTPROXY('0000000000000000000604',$,$,$,$,#603,$,$,$);\n\
             ENDSEC;\nEND-ISO-10303-21;\n",
            half = crest / 2.0,
        )
    }

    /// The service over `bytes`, the alignment and the proxy.
    fn service(bytes: &str) -> (AlignmentServiceHandle, ObjectId, ObjectId) {
        use super::super::super::{ModelBytes, Options, attach};
        let session = axioval_ifc::import_ifc_session("crest.ifc", bytes.as_bytes()).unwrap();
        let models: ModelBytes = session
            .snapshots()
            .map(|snapshot| (snapshot.source().clone(), bytes.as_bytes().to_vec()))
            .collect();
        let (session, _) = attach(session, &models, Options::default()).unwrap();
        let source = session.snapshots().next().unwrap().source().clone();
        let service = session.service::<AlignmentServiceHandle>().unwrap();
        (
            service.clone(),
            ObjectId::new(source.clone(), "#401").unwrap(),
            ObjectId::new(source, "#604").unwrap(),
        )
    }

    fn holds(value: AlignmentInterval, expected: f64, width: f64) {
        assert!(
            value.lower() <= expected
                && value.upper() >= expected
                && value.upper() - value.lower() <= width,
            "{value:?} does not hold {expected} within {width}"
        );
    }

    /// #337: the crest's radius of 5000 m at its midpoint, within the
    /// bound of the located station.
    #[test]
    fn a_crest_measures_its_radius_at_its_midpoint() {
        let (service, alignment, proxy) = service(&crest());
        let position = service
            .measure_alignment_position(
                &AlignmentRequest::try_new(proxy, alignment.clone()).unwrap(),
            )
            .unwrap();
        holds(position.station(), crest_length() / 2.0, 1e-5);
        let curvature = |distance| {
            service
                .measure_alignment_parameter(
                    &AlignmentParameterRequest::try_new(
                        alignment.clone(),
                        AlignmentParameter::VerticalCurvature,
                        distance,
                    )
                    .unwrap(),
                )
                .unwrap()
        };
        let crest = curvature(position.distance());
        let value = crest.value().unwrap();
        holds(value, -1.0 / 5000.0, 1e-15);
        let (near, far) = (1.0 / -value.lower(), 1.0 / -value.upper());
        assert!(near <= 5000.0 && far >= 5000.0 && far - near < 1e-6);
        assert!(!crest.evidence().exact);
        // The constant gradient beyond it is exactly straight.
        let tail = curvature(AlignmentInterval::try_new(250.0, 260.0).unwrap());
        assert_eq!(tail.value(), Some(AlignmentInterval::exact(0.0).unwrap()));
        assert!(tail.evidence().exact);
        // Its profile: the crest the least, the constant gradient the
        // greatest, exactly zero.
        let profile = service
            .measure_alignment_profile(
                &AlignmentProfileRequest::try_new(
                    alignment,
                    AlignmentParameter::VerticalCurvature,
                    None,
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        holds(
            profile.extreme(ProfileExtreme::Least).unwrap().value(),
            -1.0 / 5000.0,
            1e-15,
        );
        let greatest = profile.extreme(ProfileExtreme::Greatest).unwrap();
        assert_eq!(greatest.value(), AlignmentInterval::exact(0.0).unwrap());
    }

    /// #337: the plan's elements, the clothoid with its parameter
    /// `A = sqrt(100 * 500)`, and the vertical crest with its radius.
    #[test]
    fn the_element_lists_yield_kinds_radii_and_the_clothoid_parameter() {
        let (service, alignment, proxy) = service(&crest());
        let elements = |layout| {
            service.measure_alignment_elements(&AlignmentElementsRequest::new(
                alignment.clone(),
                layout,
            ))
        };
        let plan = elements(AlignmentLayout::Horizontal).unwrap();
        assert!(plan.evidence().exact);
        let kinds: Vec<_> = plan.elements().iter().map(AlignmentElement::kind).collect();
        assert_eq!(
            kinds,
            [
                AlignmentElementKind::Straight,
                AlignmentElementKind::Clothoid,
                AlignmentElementKind::Arc
            ]
        );
        let [line, clothoid, arc] = plan.elements() else {
            unreachable!()
        };
        assert_eq!((line.from().lower(), line.to().upper()), (0.0, 100.0));
        assert!(line.radius().unwrap_err().contains("unbounded"));
        assert_eq!(clothoid.type_name(), "CLOTHOID");
        assert_eq!(clothoid.locator(), "#103 (CLOTHOID)");
        assert_eq!(*clothoid.start(), ElementEnd::Straight);
        assert_eq!(*clothoid.end(), ElementEnd::Radius(500.0));
        holds(
            clothoid.clothoid_parameter().unwrap().unwrap(),
            50_000.0_f64.sqrt(),
            1e-9,
        );
        assert_eq!(clothoid.radius(), Ok(None));
        assert_eq!(arc.radius(), Ok(Some(500.0)));
        assert_eq!((arc.from().lower(), arc.to().upper()), (200.0, 300.0));
        assert_eq!(arc.clothoid_parameter(), Ok(None));

        let profile = elements(AlignmentLayout::Vertical).unwrap();
        let [crest, tail] = profile.elements() else {
            panic!("two vertical elements: {:?}", profile.elements());
        };
        assert_eq!(crest.kind(), AlignmentElementKind::Arc);
        assert_eq!(crest.radius(), Ok(Some(5000.0)));
        assert_eq!(*crest.start(), ElementEnd::Radius(-5000.0));
        assert_eq!(crest.gradients(), Some((0.02, -0.02)));
        assert_eq!(tail.kind(), AlignmentElementKind::Straight);
        assert_eq!(tail.gradients(), Some((-0.02, -0.02)));

        // A product is no alignment.
        assert!(matches!(
            service.measure_alignment_elements(&AlignmentElementsRequest::new(
                proxy,
                AlignmentLayout::Horizontal
            )),
            Err(AlignmentError::NotAlignment(_))
        ));
    }

    /// A parabola stating a radius its gradients and length contradict
    /// leaves its curvature unread, never one of the two picked.
    #[test]
    fn a_parabola_whose_radius_contradicts_its_gradients_is_unread() {
        let length = crest_length();
        // A parabola over the same length: z'' = -0.04 / L, so its radius
        // is L / 0.04, not 4000 m.
        let bytes = crest().replace("-0.02,-5000.,.CIRCULARARC.", "-0.02,-4000.,.PARABOLICARC.");
        let (service, alignment, _) = service(&bytes);
        let profile = service
            .measure_alignment_elements(&AlignmentElementsRequest::new(
                alignment,
                AlignmentLayout::Vertical,
            ))
            .unwrap();
        let parabola = &profile.elements()[0];
        assert_eq!(parabola.kind(), AlignmentElementKind::Parabola);
        let ElementEnd::Unread(why) = parabola.start() else {
            panic!("unread: {:?}", parabola.start());
        };
        assert!(
            why.contains("-4000") && why.contains(&format!("{}", length / -0.04)),
            "{why}"
        );
        assert!(parabola.radius().is_err());
    }
}
