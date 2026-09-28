//! Model comparison contract tests.
//!
//! Two revisions of one model are exported with different local ids, as every
//! re-export renumbers. Matching must follow the external identity, and what
//! cannot be matched or read must be reported rather than dropped.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    Bounds3, CompletePropertyAbsenceEvidence, CoordinateFrame, CoordinateSystemError,
    CoordinateSystemService, CoordinateSystemServiceHandle, EvidenceSession, GeometryFidelity,
    MapConversion, MetricDirection, MetricFrame, MetricPoint, ObjectBounds, ObjectFrame,
    ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, ObjectFront, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, ProximityError, ProximityEvidence, ProximityRequest,
    ProximityService, ProximityServiceHandle, ResolvedProperty, SourceCoordinateSystem,
    SourceSnapshot,
};
use axioval_ir::{
    Classification, Evidence, ExternalId, NotEvaluatedReason, Object, ObjectId, Project, Property,
    PropertyValue, RuleId, Scope, Severity, SourceId,
};
use axioval_rules::{
    ComparedProperty, ComparisonRequest, ComparisonTolerance, Difference, Facet, Matcher,
    ModelComparison, ObjectChange, Side, compare_sessions,
};

const SCHEME: &str = "guid";

fn base_source() -> SourceId {
    SourceId::new("cad", "model-r1").unwrap()
}
fn revised_source() -> SourceId {
    SourceId::new("cad", "model-r2").unwrap()
}

/// An object with a local id that differs per revision and a stable guid.
fn object(source: &SourceId, local: &str, guid: Option<&str>, kind: &str) -> Object {
    let object = Object::new(ObjectId::new(source.clone(), local).unwrap(), kind);
    match guid {
        Some(guid) => object.with_external_id(ExternalId::new(SCHEME, guid).unwrap()),
        None => object,
    }
}

fn session(source: &SourceId, objects: Vec<Object>) -> EvidenceSession {
    let snapshot = SourceSnapshot::try_new(source.clone(), "r", "sha256:fixture").unwrap();
    EvidenceSession::try_new(Project::new(objects).unwrap(), [snapshot]).unwrap()
}

/// Resolves `FireRating` from a fixed table; absent elsewhere; errors for
/// objects listed as unreadable.
struct Ratings {
    values: BTreeMap<String, &'static str>,
    unreadable: Vec<String>,
    snapshots: Vec<SourceSnapshot>,
}

impl PropertyResolutionService for Ratings {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let local = &request.object_id().local_id;
        if self.unreadable.contains(local) {
            return Err(PropertyResolutionError::Unavailable("fixture".into()));
        }
        let evidence = Evidence::exact(request.object_id().source.clone(), format!("#{local}"));
        match self.values.get(local) {
            Some(value) => Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                request.clone(),
                Property::new(
                    "Pset_Common",
                    "FireRating",
                    PropertyValue::String((*value).into()),
                )
                .unwrap()
                .with_evidence(evidence),
            )?)),
            None => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(request.clone(), evidence)?,
            )),
        }
    }
}

fn with_ratings(
    source: &SourceId,
    objects: Vec<Object>,
    values: &[(&str, &'static str)],
    unreadable: &[&str],
) -> EvidenceSession {
    let snapshot = SourceSnapshot::try_new(source.clone(), "r", "sha256:fixture").unwrap();
    EvidenceSession::try_new(Project::new(objects).unwrap(), [snapshot.clone()])
        .unwrap()
        .with_service(PropertyResolutionServiceHandle::new(Arc::new(Ratings {
            values: values.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            unreadable: unreadable.iter().map(|s| (*s).to_owned()).collect(),
            snapshots: vec![snapshot],
        })))
        .unwrap()
}

fn request() -> ComparisonRequest {
    ComparisonRequest::new(SCHEME).unwrap()
}

#[test]
fn renumbered_objects_match_by_identity_and_report_additions_and_removals() {
    let base = session(
        &base_source(),
        vec![
            object(&base_source(), "#10", Some("A"), "wall"),
            object(&base_source(), "#11", Some("B"), "wall"),
        ],
    );
    let revised = session(
        &revised_source(),
        vec![
            object(&revised_source(), "#77", Some("A"), "wall"),
            object(&revised_source(), "#78", Some("C"), "door"),
        ],
    );
    let comparison = compare_sessions(&base, &revised, &request());
    let summary: Vec<(&str, &str)> = comparison
        .objects()
        .iter()
        .map(|compared| {
            let change = match &compared.change {
                ObjectChange::Added { .. } => "added",
                ObjectChange::Removed { .. } => "removed",
                ObjectChange::Matched { differences, .. } if differences.is_empty() => "same",
                ObjectChange::Matched { .. } => "changed",
            };
            (compared.identity.as_str(), change)
        })
        .collect();
    assert_eq!(
        summary,
        vec![("A", "same"), ("B", "removed"), ("C", "added")]
    );
    assert!(!comparison.is_identical());
}

#[test]
fn kind_classification_property_and_relationship_changes_are_reported() {
    let base = session(
        &base_source(),
        vec![
            object(&base_source(), "#1", Some("A"), "wall")
                .with_classification(Classification::new("Uniclass", "EF_25_10").unwrap())
                .with_property(
                    Property::new("Pset", "LoadBearing", PropertyValue::Boolean(true)).unwrap(),
                ),
            object(&base_source(), "#2", Some("S1"), "storey"),
            object(&base_source(), "#3", Some("S2"), "storey"),
        ],
    );
    let mut moved = object(&revised_source(), "#9", Some("A"), "curtain_wall")
        .with_classification(Classification::new("Uniclass", "EF_25_30").unwrap())
        .with_property(
            Property::new("Pset", "LoadBearing", PropertyValue::Boolean(false)).unwrap(),
        );
    moved.relationships.insert(
        "contained_in".into(),
        vec![ObjectId::new(revised_source(), "#8").unwrap()],
    );
    let mut base_objects: Vec<Object> = base.project().objects().cloned().collect();
    base_objects[0].relationships.insert(
        "contained_in".into(),
        vec![ObjectId::new(base_source(), "#2").unwrap()],
    );
    let base = session(&base_source(), base_objects);
    let revised = session(
        &revised_source(),
        vec![
            moved,
            object(&revised_source(), "#7", Some("S1"), "storey"),
            object(&revised_source(), "#8", Some("S2"), "storey"),
        ],
    );

    let comparison = compare_sessions(&base, &revised, &request());
    let ObjectChange::Matched {
        differences,
        unresolved,
        ..
    } = &comparison.objects()[0].change
    else {
        panic!("A is matched");
    };
    assert!(unresolved.is_empty(), "{unresolved:?}");
    let rendered: Vec<String> = differences.iter().map(ToString::to_string).collect();
    assert_eq!(
        rendered,
        vec![
            "kind wall -> curtain_wall",
            "classifications -[Uniclass:EF_25_10] +[Uniclass:EF_25_30]",
            "property Pset.LoadBearing true -> false",
            // Targets are named by identity, not by renumbered local id.
            "contained_in -[S1] +[S2]",
        ]
    );
}

#[test]
fn requested_properties_are_resolved_and_resolver_failures_are_unresolved() {
    let objects = |source: &SourceId| {
        vec![
            object(source, "a", Some("A"), "door"),
            object(source, "b", Some("B"), "door"),
            object(source, "c", Some("C"), "door"),
        ]
    };
    let base = with_ratings(
        &base_source(),
        objects(&base_source()),
        &[("a", "EI30"), ("b", "EI30")],
        &[],
    );
    let revised = with_ratings(
        &revised_source(),
        objects(&revised_source()),
        &[("a", "EI60"), ("b", "EI30")],
        &["c"],
    );
    let request = request()
        .with_property(Some("Pset_Common"), "FireRating")
        .unwrap();
    let comparison = compare_sessions(&base, &revised, &request);

    let changes: Vec<_> = comparison
        .objects()
        .iter()
        .map(|compared| match &compared.change {
            ObjectChange::Matched {
                differences,
                unresolved,
                ..
            } => (differences.len(), unresolved.len()),
            other => panic!("all matched: {other:?}"),
        })
        .collect();
    assert_eq!(changes, vec![(1, 0), (0, 0), (0, 1)]);
    assert!(matches!(
        &comparison.objects()[0].change,
        ObjectChange::Matched { differences, .. }
            if matches!(&differences[0], Difference::Property { base: Some(_), revised: Some(_), .. })
    ));

    let report = comparison.report(&RuleId::new("compare").unwrap(), &Severity::Info);
    assert_eq!(report.findings().len(), 1);
    assert_eq!(report.not_evaluated().len(), 1);
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::IncompleteEvidence
    );
}

/// Without a resolver there is no way to know a requested property's value.
/// Comparing "unknown" with "unknown" as equal would hide every change.
#[test]
fn a_session_without_a_resolver_leaves_requested_properties_unresolved() {
    let base = session(
        &base_source(),
        vec![object(&base_source(), "a", Some("A"), "door")],
    );
    let revised = session(
        &revised_source(),
        vec![object(&revised_source(), "a", Some("A"), "door")],
    );
    let request = request().with_property(None, "FireRating").unwrap();
    let comparison = compare_sessions(&base, &revised, &request);
    assert!(matches!(
        &comparison.objects()[0].change,
        ObjectChange::Matched { unresolved, .. } if unresolved.len() == 1
    ));
    assert!(!comparison.is_identical());
}

#[test]
fn unidentified_and_ambiguous_objects_are_reported_not_dropped() {
    let other = SourceId::new("cad", "linked").unwrap();
    let snapshots = [
        SourceSnapshot::try_new(revised_source(), "r", "sha256:a").unwrap(),
        SourceSnapshot::try_new(other.clone(), "r", "sha256:b").unwrap(),
    ];
    let revised = EvidenceSession::try_new(
        Project::new(vec![
            object(&revised_source(), "x", Some("A"), "wall"),
            // A federated copy claims the same identity from another source.
            object(&other, "y", Some("A"), "wall"),
            object(&revised_source(), "z", None, "wall"),
        ])
        .unwrap(),
        snapshots,
    )
    .unwrap();
    let base = session(
        &base_source(),
        vec![object(&base_source(), "a", Some("A"), "wall")],
    );

    let comparison = compare_sessions(&base, &revised, &request());
    assert!(
        comparison.objects().is_empty(),
        "an ambiguous identity matches nothing"
    );
    assert_eq!(
        comparison.unidentified(),
        &[(Side::Revised, ObjectId::new(revised_source(), "z").unwrap())]
    );
    let sides: Vec<(Side, usize)> = comparison
        .ambiguous()
        .iter()
        .map(|entry| (entry.side, entry.objects.len()))
        .collect();
    assert_eq!(sides, vec![(Side::Base, 1), (Side::Revised, 2)]);

    let report = comparison.report(&RuleId::new("compare").unwrap(), &Severity::Info);
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 4);
}

#[test]
fn identical_sessions_are_identical() {
    let make = |source: &SourceId| {
        session(
            source,
            vec![object(source, "a", Some("A"), "wall").with_property(
                Property::new("Pset", "Width", PropertyValue::Decimal(0.0)).unwrap(),
            )],
        )
    };
    let revised = session(
        &revised_source(),
        vec![
            object(&revised_source(), "q", Some("A"), "wall").with_property(
                // Negative zero is the same width.
                Property::new("Pset", "Width", PropertyValue::Decimal(-0.0)).unwrap(),
            ),
        ],
    );
    assert!(compare_sessions(&make(&base_source()), &revised, &request()).is_identical());
}

#[test]
fn blank_declarations_are_refused() {
    assert!(ComparisonRequest::new(" ").is_err());
    assert!(request().with_property(Some(""), "x").is_err());
    assert!(request().with_property(None, " ").is_err());
    assert!(request().with_property_set(" ").is_err());
    assert!(ComparisonRequest::matching(Vec::new()).is_err());
    assert!(ComparisonRequest::matching(vec![Matcher::Scheme(String::new())]).is_err());
}

#[test]
fn a_property_matches_what_the_scheme_leaves_and_an_unreadable_one_decides_nothing() {
    // A keeps its guid; B's was regenerated and matches by its rating, a
    // stand-in for any identifying property. C's rating cannot be read, so
    // D, rated and unmatched, may be C and is neither added nor removed.
    let base = with_ratings(
        &base_source(),
        vec![
            object(&base_source(), "#1", Some("A"), "wall"),
            object(&base_source(), "#2", Some("B"), "wall"),
            object(&base_source(), "#3", None, "wall"),
        ],
        &[("#1", "EI30"), ("#2", "EI60")],
        &["#3"],
    );
    let revised = with_ratings(
        &revised_source(),
        vec![
            object(&revised_source(), "#7", Some("A"), "wall"),
            object(&revised_source(), "#8", Some("B-new"), "wall"),
            object(&revised_source(), "#9", None, "wall"),
        ],
        &[("#7", "EI30"), ("#8", "EI60"), ("#9", "EI90")],
        &[],
    );
    let rating = ComparedProperty::new(Some("Pset_Common"), "FireRating").unwrap();
    let request = ComparisonRequest::matching(vec![
        Matcher::Scheme(SCHEME.into()),
        Matcher::Property {
            base: rating.clone(),
            revised: rating,
        },
    ])
    .unwrap();
    let comparison = compare_sessions(&base, &revised, &request);
    let summary: Vec<(&str, &str)> = comparison
        .objects()
        .iter()
        .map(|compared| (compared.identity.as_str(), compared.matcher.as_str()))
        .collect();
    assert_eq!(
        summary,
        vec![("A", "guid"), ("EI60", "property Pset_Common.FireRating")]
    );
    let undecided: Vec<(Side, &str)> = comparison
        .undecided()
        .iter()
        .map(|entry| (entry.side, entry.object.local_id.as_str()))
        .collect();
    assert_eq!(undecided, vec![(Side::Base, "#3"), (Side::Revised, "#9")]);
    assert!(!comparison.is_identical());
}

// ---------------------------------------------------------------------------
// Spatial facets: placement, geometry and coordinate systems.
// ---------------------------------------------------------------------------

/// A frame per local id: origin and rotation about Z, in degrees; `None`
/// is an unplaced object, and a missing entry an unsupported placement.
struct Frames {
    frames: BTreeMap<String, Option<([f64; 3], f64)>>,
    snapshots: Vec<SourceSnapshot>,
}

impl ObjectFrameService for Frames {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        match self.frames.get(&object.local_id) {
            None => Err(ObjectFrameError::Unsupported("grid placement".into())),
            Some(None) => Err(ObjectFrameError::NotPlaced(object.clone())),
            Some(Some((origin, degrees))) => {
                let (sin, cos) = degrees.to_radians().sin_cos();
                let frame = MetricFrame::try_new(
                    MetricPoint::try_new(object.clone(), *origin).unwrap(),
                    MetricDirection::try_new([cos, sin, 0.0]).unwrap(),
                    MetricDirection::try_new([-sin, cos, 0.0]).unwrap(),
                    MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
                )
                .unwrap();
                ObjectFrame::try_new(
                    object.clone(),
                    frame,
                    ObjectFront::NotStated,
                    Evidence::exact(object.source.clone(), "placement"),
                )
            }
        }
    }
}

type Frame = Option<([f64; 3], f64)>;
type Body = Option<([f64; 3], [f64; 3], f64)>;

/// Bounds per local id with a chord deviation; `None` is bodiless, and a
/// missing entry an unmeasured body.
struct Bodies(BTreeMap<String, Body>);

impl ProximityService for Bodies {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        match self.0.get(&object.local_id) {
            None => Err(ProximityError::Unavailable),
            Some(None) => Err(ProximityError::NoBody),
            Some(Some((min, max, deviation))) => ObjectBounds::try_new(
                object.clone(),
                Bounds3::try_new(*min, *max)?,
                if *deviation > 0.0 {
                    GeometryFidelity::tessellated(*deviation)?
                } else {
                    GeometryFidelity::Exact
                },
            ),
        }
    }
    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        Err(ProximityError::Unavailable)
    }
}

struct Crs(Vec<SourceSnapshot>, SourceCoordinateSystem);

impl CoordinateSystemService for Crs {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }
    fn coordinate_system(
        &self,
        _: &SourceId,
    ) -> Result<SourceCoordinateSystem, CoordinateSystemError> {
        Ok(self.1.clone())
    }
}

/// Each entry: identity, frame (`None`: unsupported), body (`None`:
/// unmeasured).
fn spatial(
    source: &SourceId,
    objects: &[(&str, Option<Frame>, Option<Body>)],
    crs: Option<SourceCoordinateSystem>,
) -> EvidenceSession {
    let snapshot = SourceSnapshot::try_new(source.clone(), "r", "sha256:fixture").unwrap();
    let project = Project::new(
        objects
            .iter()
            .map(|(guid, _, _)| object(source, &format!("#{guid}"), Some(guid), "wall"))
            .collect(),
    )
    .unwrap();
    let frames = Frames {
        frames: objects
            .iter()
            .filter_map(|(guid, frame, _)| frame.map(|frame| (format!("#{guid}"), frame)))
            .collect(),
        snapshots: vec![snapshot.clone()],
    };
    let bodies = Bodies(
        objects
            .iter()
            .filter_map(|(guid, _, body)| body.map(|body| (format!("#{guid}"), body)))
            .collect(),
    );
    let mut session = EvidenceSession::try_new(project, [snapshot.clone()])
        .unwrap()
        .with_service(ObjectFrameServiceHandle::new(Arc::new(frames)))
        .unwrap()
        .with_host_service(
            ProximityServiceHandle::new(Arc::new(bodies)),
            std::slice::from_ref(&snapshot),
        )
        .unwrap();
    if let Some(crs) = crs {
        session = session
            .with_service(CoordinateSystemServiceHandle::new(Arc::new(Crs(
                vec![snapshot],
                crs,
            ))))
            .unwrap();
    }
    session
}

/// One unplaced, bodiless object, for sessions compared by source only.
const SITE: &[(&str, Option<Frame>, Option<Body>)] = &[("SITE", Some(None), Some(None))];

fn tolerance() -> ComparisonTolerance {
    ComparisonTolerance::try_new(0.001, 0.01_f64.to_radians()).unwrap()
}

/// Every difference, gap and undetermined measure of one matched identity.
fn outcome(comparison: &ModelComparison, identity: &str) -> Vec<String> {
    let compared = comparison
        .objects()
        .iter()
        .find(|compared| compared.identity == identity)
        .unwrap();
    let ObjectChange::Matched {
        differences,
        unresolved,
        undetermined,
        ..
    } = &compared.change
    else {
        panic!("{identity} is matched");
    };
    differences
        .iter()
        .map(|difference| format!("changed {difference}"))
        .chain(unresolved.iter().map(|gap| format!("unresolved {gap}")))
        .chain(
            undetermined
                .iter()
                .map(|measurement| format!("undetermined {measurement}")),
        )
        .collect()
}

#[test]
fn placement_compares_origins_and_axes_against_the_tolerance() {
    let base = spatial(
        &base_source(),
        &[
            ("MOVED", Some(Some(([0.0; 3], 0.0))), None),
            ("NUDGED", Some(Some(([0.0; 3], 0.0))), None),
            ("TURNED", Some(Some(([1.0, 1.0, 0.0], 0.0))), None),
            ("UNPLACED", Some(Some(([0.0; 3], 0.0))), None),
            ("GROUP", Some(None), None),
            ("GRID", None, None),
        ],
        None,
    );
    let revised = spatial(
        &revised_source(),
        &[
            ("MOVED", Some(Some(([0.3, 0.4, 0.0], 0.0))), None),
            ("NUDGED", Some(Some(([0.0005, 0.0, 0.0], 0.0))), None),
            ("TURNED", Some(Some(([1.0, 1.0, 0.0], 90.0))), None),
            ("UNPLACED", Some(None), None),
            ("GROUP", Some(None), None),
            ("GRID", Some(Some(([0.0; 3], 0.0))), None),
        ],
        None,
    );
    let comparison = compare_sessions(&base, &revised, &request().with_placement(tolerance()));
    assert_eq!(
        outcome(&comparison, "MOVED"),
        vec!["changed placement origin differs by 0.5000 m (tolerance 0.0010 m)"]
    );
    assert!(outcome(&comparison, "NUDGED").is_empty());
    assert_eq!(
        outcome(&comparison, "TURNED"),
        vec!["changed placement orientation differs by 90.000° (tolerance 0.010°)"]
    );
    assert_eq!(
        outcome(&comparison, "UNPLACED"),
        vec!["changed placement placement placed -> not placed"]
    );
    assert!(outcome(&comparison, "GROUP").is_empty());
    assert_eq!(
        outcome(&comparison, "GRID"),
        vec!["unresolved placement not compared: object placement unsupported: grid placement"]
    );
}

#[test]
fn a_tessellated_shift_straddling_the_tolerance_is_undetermined() {
    let body = |x: f64, deviation: f64| Some(Some(([x, 0.0, 0.0], [x + 1.0, 1.0, 3.0], deviation)));
    let base = spatial(
        &base_source(),
        &[
            ("EXACT", None, body(0.0, 0.0)),
            ("ROUND", None, body(0.0, 0.001)),
            ("ROUND_MOVED", None, body(0.0, 0.001)),
            ("SAME_ROUND", None, body(0.0, 0.0002)),
            ("VOID", None, Some(None)),
            ("GONE", None, body(0.0, 0.0)),
            ("UNMEASURED", None, None),
        ],
        None,
    );
    let revised = spatial(
        &revised_source(),
        &[
            ("EXACT", None, body(0.25, 0.0)),
            // A 1.5 mm shift of 1 mm tessellations: anywhere in 0 to 3.5 mm.
            ("ROUND", None, body(0.0015, 0.001)),
            ("ROUND_MOVED", None, body(0.5, 0.001)),
            // Within 0.4 mm of chord deviation: provably unchanged.
            ("SAME_ROUND", None, body(0.0, 0.0002)),
            ("VOID", None, Some(None)),
            ("GONE", None, Some(None)),
            ("UNMEASURED", None, body(0.0, 0.0)),
        ],
        None,
    );
    let comparison = compare_sessions(&base, &revised, &request().with_geometry(tolerance()));
    assert_eq!(
        outcome(&comparison, "EXACT"),
        vec!["changed geometry bounds differs by 0.2500 m (tolerance 0.0010 m)"]
    );
    assert_eq!(
        outcome(&comparison, "ROUND"),
        vec!["undetermined geometry bounds differs by 0.0000 m to 0.0035 m (tolerance 0.0010 m)"]
    );
    assert_eq!(
        outcome(&comparison, "ROUND_MOVED"),
        vec!["changed geometry bounds differs by 0.4980 m to 0.5020 m (tolerance 0.0010 m)"]
    );
    assert!(outcome(&comparison, "SAME_ROUND").is_empty());
    assert!(outcome(&comparison, "VOID").is_empty());
    assert_eq!(
        outcome(&comparison, "GONE"),
        vec!["changed geometry body present -> none"]
    );
    assert_eq!(
        outcome(&comparison, "UNMEASURED"),
        vec![
            "unresolved geometry not compared: proximity measurement is unavailable for the requested object"
        ]
    );

    let report = comparison.report(&RuleId::new("compare").unwrap(), &Severity::Warning);
    let rules: Vec<(String, String)> = report
        .findings()
        .iter()
        .map(|finding| {
            (
                finding.rule_id.to_string(),
                finding.object_id().unwrap().local_id.clone(),
            )
        })
        .collect();
    assert_eq!(
        rules,
        vec![
            ("compare.geometry".into(), "#EXACT".into()),
            ("compare.geometry".into(), "#GONE".into()),
            ("compare.geometry".into(), "#ROUND_MOVED".into()),
        ]
    );
    let gaps: Vec<String> = report
        .not_evaluated()
        .iter()
        .map(|gap| format!("{} {}", gap.rule_id, gap.message))
        .collect();
    assert_eq!(gaps.len(), 2, "{gaps:?}");
    assert!(gaps.iter().all(|gap| gap.starts_with("compare.geometry ")));
    assert!(!comparison.is_identical());
}

#[test]
fn a_session_without_the_service_leaves_a_requested_spatial_facet_unresolved() {
    let base = session(
        &base_source(),
        vec![object(&base_source(), "a", Some("A"), "wall")],
    );
    let revised = session(
        &revised_source(),
        vec![object(&revised_source(), "a", Some("A"), "wall")],
    );
    let request = request()
        .with_placement(tolerance())
        .with_geometry(tolerance())
        .with_coordinate_systems(tolerance());
    let comparison = compare_sessions(&base, &revised, &request);
    assert_eq!(
        outcome(&comparison, "A"),
        vec![
            "unresolved placement not compared: a session has no object-frame service",
            "unresolved geometry not compared: a session has no geometry service",
        ]
    );
    assert_eq!(comparison.sources().len(), 1);
    assert_eq!(
        comparison.sources()[0].unresolved[0].facet,
        Facet::CoordinateSystem
    );
    assert!(ComparisonTolerance::try_new(-1.0, 0.0).is_err());
    assert!(ComparisonTolerance::try_new(0.0, f64::NAN).is_err());
}

fn crs(
    source: &SourceId,
    origin: [f64; 3],
    north: Option<[f64; 2]>,
    map: Option<MapConversion>,
) -> SourceCoordinateSystem {
    let axis = |v| MetricDirection::try_new(v).unwrap();
    SourceCoordinateSystem::try_new(
        source.clone(),
        Some(
            CoordinateFrame::try_new(
                origin,
                axis([1.0, 0.0, 0.0]),
                axis([0.0, 1.0, 0.0]),
                axis([0.0, 0.0, 1.0]),
            )
            .unwrap(),
        ),
        north,
        map,
        Evidence::exact(source.clone(), "crs"),
    )
    .unwrap()
}

fn map(target: &str, offset: [f64; 3], scale: f64, unit: Option<f64>) -> MapConversion {
    MapConversion::try_new(Some(target.into()), offset, [1.0, 0.0], scale, unit).unwrap()
}

/// Differences and gaps between two sources' coordinate systems.
fn compare_crs(before: SourceCoordinateSystem, after: SourceCoordinateSystem) -> Vec<String> {
    let base = spatial(&base_source(), SITE, Some(before));
    let revised = spatial(&revised_source(), SITE, Some(after));
    let comparison = compare_sessions(
        &base,
        &revised,
        &request().with_coordinate_systems(tolerance()),
    );
    let [source] = comparison.sources() else {
        panic!("one pair of sources");
    };
    source
        .differences
        .iter()
        .map(ToString::to_string)
        .chain(source.unresolved.iter().map(ToString::to_string))
        .collect()
}

#[test]
fn coordinate_systems_compare_world_frame_north_and_map_conversion() {
    let georeferenced = |source: &SourceId| {
        crs(
            source,
            [0.0; 3],
            Some([0.0, 1.0]),
            Some(map(
                "EPSG:25832",
                [500_000.0, 5_600_000.0, 50.0],
                1.0,
                Some(1.0),
            )),
        )
    };
    assert!(
        compare_crs(
            georeferenced(&base_source()),
            georeferenced(&revised_source())
        )
        .is_empty()
    );

    let changed = crs(
        &revised_source(),
        [0.0, 0.0, 2.0],
        Some([1.0, 1.0]),
        Some(map(
            "EPSG:25833",
            [500_010.0, 5_600_000.0, 50.0],
            0.5,
            Some(1.0),
        )),
    );
    assert_eq!(
        compare_crs(georeferenced(&base_source()), changed),
        vec![
            "coordinate-system world-origin differs by 2.0000 m (tolerance 0.0010 m)",
            "coordinate-system true-north differs by 45.000° (tolerance 0.010°)",
            "coordinate-system map target EPSG:25832 -> EPSG:25833",
            "coordinate-system map-offset differs by 10.0000 m (tolerance 0.0010 m)",
            "coordinate-system map-scale differs by 0.5 (tolerance 0)",
        ]
    );

    // Offsets in an unknown unit cannot be measured, unless they are equal.
    let unknown_unit = |source: &SourceId, offset| {
        crs(
            source,
            [0.0; 3],
            Some([0.0, 1.0]),
            Some(map("EPSG:25832", offset, 1.0, None)),
        )
    };
    assert_eq!(
        compare_crs(
            georeferenced(&base_source()),
            unknown_unit(&revised_source(), [1.0, 2.0, 3.0])
        ),
        vec![
            "coordinate-system map offset not compared: the map unit is not stated exactly, so the offsets cannot be measured in metres"
        ]
    );
    assert!(
        compare_crs(
            unknown_unit(&base_source(), [1.0, 2.0, 3.0]),
            unknown_unit(&revised_source(), [1.0, 2.0, 3.0])
        )
        .is_empty()
    );

    // A map conversion stated on one side only.
    assert_eq!(
        compare_crs(
            georeferenced(&base_source()),
            crs(&revised_source(), [0.0; 3], Some([0.0, 1.0]), None)
        ),
        vec!["coordinate-system map conversion stated -> not stated"]
    );
}

#[test]
fn a_changed_coordinate_system_is_a_source_finding() {
    let base = spatial(
        &base_source(),
        SITE,
        Some(crs(&base_source(), [0.0; 3], None, None)),
    );
    let revised = spatial(
        &revised_source(),
        SITE,
        Some(crs(&revised_source(), [0.0; 3], Some([0.0, 1.0]), None)),
    );
    let comparison = compare_sessions(
        &base,
        &revised,
        &request().with_coordinate_systems(tolerance()),
    );
    let report = comparison.report(&RuleId::new("compare").unwrap(), &Severity::Warning);
    assert_eq!(report.findings().len(), 1);
    let finding = &report.findings()[0];
    assert_eq!(finding.rule_id.to_string(), "compare.coordinate-system");
    assert_eq!(finding.scope, Scope::Source(revised_source()));
    assert!(
        finding
            .message
            .contains("coordinate-system true north not stated -> stated"),
        "{}",
        finding.message
    );
}
