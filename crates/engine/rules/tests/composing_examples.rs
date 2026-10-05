//! The worked examples of the book's "Composing rules" chapter: each
//! definitions and ruleset package under `docs/examples/composing` is
//! compiled against the built-in registry and run over a small in-memory
//! project, so the book shows exactly the packages tested here.
//!
//! Every example judges one object that passes, one that fails, one whose
//! measurement straddles its bound (not evaluated) and one where the source
//! states no value (`null`), to show the three-valued semantics.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, ElevationInterval, EvidenceSession, FaceNormal, FaceNormals, FacePiece,
    FacePieceSet, FacePieces, Headroom, HeadroomRequest, MetricDirection, MetricFrame, MetricPoint,
    ObjectFrame, ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, ObjectFront,
    SlopedSurface, SourceSnapshot, SurfaceFace, Tread, TreadFlight, TreadFlightRequest,
    VerticalExtent, VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle,
    WalkingLine, WalkingSurfaceError, WalkingSurfaceService, WalkingSurfaceServiceHandle, compile,
};
use axioval_ir::{
    DefinitionPackage, Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension,
    Report, RuleSetPackage, Scope,
};
use axioval_rules::register_builtins;
use common::runtime::{session_in, snapshot_in};
use common::{Model, id, source};

/// The type system the examples name their concepts in.
const IFC4X3: &str = "https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4.3";

/// One example's packages, as the book includes them.
struct Example {
    definitions: &'static str,
    ruleset: &'static str,
}

const EMBANKMENT: Example = Example {
    definitions: include_str!("../../../../docs/examples/composing/embankment.definitions.json"),
    ruleset: include_str!("../../../../docs/examples/composing/embankment.ruleset.json"),
};

const DECK: Example = Example {
    definitions: include_str!("../../../../docs/examples/composing/deck.definitions.json"),
    ruleset: include_str!("../../../../docs/examples/composing/deck.ruleset.json"),
};

const STAIR: Example = Example {
    definitions: include_str!("../../../../docs/examples/composing/stair.definitions.json"),
    ruleset: include_str!("../../../../docs/examples/composing/stair.ruleset.json"),
};

const COVER: Example = Example {
    definitions: include_str!("../../../../docs/examples/composing/cover.definitions.json"),
    ruleset: include_str!("../../../../docs/examples/composing/cover.ruleset.json"),
};

/// What the geometry services answer per object: the top face's pieces
/// and their normals, the vertical extent, the placement (every object
/// along the world axes) and a stair flight.
#[derive(Default)]
struct Geometry {
    pieces: BTreeMap<ObjectId, Vec<Vec<FaceNormal>>>,
    extents: BTreeMap<ObjectId, (f64, f64)>,
    flights: BTreeMap<ObjectId, TreadFlight>,
    snapshots: Vec<SourceSnapshot>,
}

impl Geometry {
    fn new() -> Self {
        Self {
            snapshots: vec![snapshot_in(IFC4X3)],
            ..Self::default()
        }
    }

    /// A top face of planar pieces, each falling `ratio` (rise per run)
    /// along `direction` in plan.
    fn face(mut self, object: &str, pieces: &[([f64; 2], f64)]) -> Self {
        let pieces = pieces.iter().map(|piece| vec![normal(*piece)]).collect();
        self.pieces.insert(id(object), pieces);
        self
    }

    /// A top face of one warped piece, its parts falling as `face`'s
    /// pieces do.
    fn warped(mut self, object: &str, parts: &[([f64; 2], f64)]) -> Self {
        let parts = parts.iter().map(|part| normal(*part)).collect();
        self.pieces.insert(id(object), vec![parts]);
        self
    }

    /// A body from elevation 0 to a top between `lower` and `upper`.
    fn height(mut self, object: &str, lower: f64, upper: f64) -> Self {
        self.extents.insert(id(object), (lower, upper));
        self
    }

    fn flight(mut self, flight: TreadFlight) -> Self {
        self.flights.insert(flight.object().clone(), flight);
        self
    }

    fn session(self, model: Model) -> EvidenceSession {
        let snapshots = self.snapshots.clone();
        let geometry = Arc::new(self);
        session_in(model, IFC4X3)
            .with_host_service(
                VerticalExtentServiceHandle::new(geometry.clone()),
                &snapshots,
            )
            .unwrap()
            .with_host_service(ObjectFrameServiceHandle::new(geometry.clone()), &snapshots)
            .unwrap()
            .with_host_service(WalkingSurfaceServiceHandle::new(geometry), &snapshots)
            .unwrap()
    }
}

impl VerticalExtentService for Geometry {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let (lower, upper) = *self
            .extents
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("extent:{object}"));
        evidence.exact = lower.to_bits() == upper.to_bits();
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::exact(0.0)?,
            ElevationInterval::try_new(lower, upper)?,
            evidence,
        )
    }

    fn measure_face_normals(
        &self,
        object: &ObjectId,
        face: SurfaceFace,
    ) -> Result<FaceNormals, VerticalExtentError> {
        let normals = self
            .pieces
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?
            .concat();
        let evidence = Evidence::exact(source(), format!("faces:{object}"));
        FaceNormals::try_new(object.clone(), face, normals, evidence)
    }

    /// The top face's pieces, each of area 1 m²; only the top is known.
    fn measure_face_pieces(
        &self,
        object: &ObjectId,
        set: FacePieceSet,
    ) -> Result<FacePieces, VerticalExtentError> {
        if set != FacePieceSet::Top {
            return Err(VerticalExtentError::Unavailable("only the top".into()));
        }
        let pieces = self
            .pieces
            .get(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?
            .iter()
            .map(|parts| FacePiece::try_new(parts.clone(), 1.0, 1.0))
            .collect::<Result<_, _>>()?;
        let evidence = Evidence::exact(source(), format!("pieces:{object}"));
        FacePieces::try_new(object.clone(), set, pieces, evidence)
    }
}

impl ObjectFrameService for Geometry {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(object.clone(), [0.0; 3]).unwrap(),
            MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
            MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap(),
            MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
        )
        .unwrap();
        ObjectFrame::try_new(
            object.clone(),
            frame,
            ObjectFront::NotStated,
            Evidence::exact(source(), format!("frame:{object}")),
        )
    }
}

impl WalkingSurfaceService for Geometry {
    fn measure_tread_flight(
        &self,
        request: &TreadFlightRequest,
    ) -> Result<TreadFlight, WalkingSurfaceError> {
        self.flights
            .get(request.object())
            .cloned()
            .ok_or_else(|| WalkingSurfaceError::Unavailable("no flight measured".into()))
    }

    fn measure_sloped_runs(&self, _: &ObjectId) -> Result<SlopedSurface, WalkingSurfaceError> {
        Err(WalkingSurfaceError::Unavailable("no ramps".into()))
    }

    fn measure_headroom(&self, _: &HeadroomRequest) -> Result<Headroom, WalkingSurfaceError> {
        Err(WalkingSurfaceError::Unavailable("no headroom".into()))
    }
}

/// The normal of a planar part falling `ratio` (rise per run) along
/// `direction` in plan.
fn normal(([x, y], ratio): ([f64; 2], f64)) -> FaceNormal {
    FaceNormal::exact([x * ratio, y * ratio, 1.0]).unwrap()
}

/// A straight flight of `count` equal risers and goings, every position
/// known within `margin` either way.
fn flight(object: &str, count: usize, riser: f64, going: f64, margin: f64) -> TreadFlight {
    let position = |value: f64| {
        if margin > 0.0 {
            ElevationInterval::try_new(value - margin, value + margin).unwrap()
        } else {
            ElevationInterval::exact(value).unwrap()
        }
    };
    let treads: Vec<Tread> = (0..count)
        .map(|step| {
            #[allow(clippy::cast_precision_loss)]
            let step = step as f64;
            let front = going * step;
            Tread::try_new(
                position(riser * (step + 1.0)),
                position(front),
                position(front + going),
            )
            .unwrap()
            .with_sides(position(0.0), position(1.2))
            .unwrap()
        })
        .collect();
    let top = treads.last().unwrap().elevation();
    let evidence = Evidence {
        source: source(),
        locator: format!("tread-flight:{object}"),
        exact: margin == 0.0,
    };
    TreadFlight::try_new(
        TreadFlightRequest::new(id(object)),
        WalkingLine::Straight(MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap()),
        ElevationInterval::exact(0.0).unwrap(),
        top,
        treads,
        evidence,
    )
    .unwrap()
}

/// What a rule decided: the objects with a finding and the objects left
/// not evaluated (with the reason), each sorted. Every other selected
/// object passed.
#[derive(Debug, PartialEq)]
struct Verdicts {
    found: Vec<String>,
    open: Vec<(String, NotEvaluatedReason)>,
}

/// Compiles `example` against the built-in capabilities and runs it over
/// `session`.
fn check(example: &Example, session: &EvidenceSession) -> Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let definitions: DefinitionPackage = serde_json::from_str(example.definitions).unwrap();
    let ruleset: RuleSetPackage = serde_json::from_str(example.ruleset).unwrap();
    let plan = compile(&registry, std::slice::from_ref(&definitions), &ruleset).unwrap();
    axioval_engine::Runtime::new(registry)
        .run_session(session, plan)
        .unwrap()
}

fn verdicts(report: &Report, rule: &str) -> Verdicts {
    let mut found: Vec<String> = report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(common::subject)
        .collect();
    found.sort();
    let mut open: Vec<(String, NotEvaluatedReason)> = report
        .not_evaluated
        .iter()
        .filter(|outcome| outcome.rule_id.to_string() == rule)
        .map(|outcome| match &outcome.scope {
            Scope::Object(object) => (object.local_id.clone(), outcome.reason.clone()),
            other => panic!("{rule} left {other:?} not evaluated: {}", outcome.message),
        })
        .collect();
    open.sort_by(|a, b| a.0.cmp(&b.0));
    Verdicts { found, open }
}

fn message(report: &Report, object: &str) -> String {
    report
        .findings()
        .iter()
        .find(|finding| common::subject(finding) == object)
        .unwrap_or_else(|| panic!("no finding on {object}"))
        .message
        .clone()
}

fn text(value: &str) -> PropertyValue {
    PropertyValue::String(value.into())
}

fn length(metres: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value: metres,
        dimension: QuantityDimension::Length,
    }
}

/// The steepest slope a soil class allows, from the rule's table, over
/// each piece of a single-body fill: a level crest between two batters.
#[test]
fn an_embankment_slope_is_limited_by_its_soil_class() {
    const SET: &str = "Ground";
    const CLASS: &str = "SoilClass";
    const FALL: [f64; 2] = [1.0, 0.0];
    const EAST: [f64; 2] = [1.0, 0.0];
    const WEST: [f64; 2] = [-1.0, 0.0];
    // A crest between batters falling `ratio` to either side.
    let fill = |ratio: f64| [(EAST, 0.0), (EAST, ratio), (WEST, ratio)];
    let model = [
        "steady",
        "steep",
        "uneven",
        "unclassified",
        "unlisted",
        "unread",
    ]
    .into_iter()
    .fold(Model::default(), |model, local| {
        model.object(local, "IfcEarthworksFill")
    })
    .value("steady", SET, CLASS, text("sand"))
    .value("steep", SET, CLASS, text("clay"))
    .value("uneven", SET, CLASS, text("sand"))
    .value("unclassified", SET, CLASS, PropertyValue::Null)
    .value("unlisted", SET, CLASS, text("peat"))
    .unreadable_value("unread", SET, CLASS, "IFCLABEL");
    let geometry = Geometry::new()
        // Batters of one in three in sand, which allows 1:2.5; the level
        // crest beside them leaves nothing open.
        .face("steady", &fill(0.3))
        // 1:2.5 in clay, which allows 1:4.
        .face("steep", &fill(0.4))
        // A batter warped from 1:3 to 1:2: is it within 1:2.5?
        .warped("uneven", &[(FALL, 0.3), (FALL, 0.5)])
        .face("unclassified", &[(FALL, 0.1)])
        .face("unlisted", &[(FALL, 0.1)])
        .face("unread", &[(FALL, 0.1)]);
    let report = check(&EMBANKMENT, &geometry.session(model));
    let verdicts = verdicts(&report, "embankment-slope");
    // A stated `null` soil class is no soil class: the requirement fails,
    // as does a class the table has no row for.
    assert_eq!(verdicts.found, ["steep", "unclassified", "unlisted"]);
    // A straddling slope and an unreadable class are not evaluated.
    assert_eq!(
        verdicts.open,
        [
            ("uneven".into(), NotEvaluatedReason::IncompleteEvidence),
            ("unread".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
    // A finding names the labelled subexpression that failed.
    let steep = message(&report, "steep");
    assert!(
        steep.contains("`slope within the limit` is false"),
        "{steep}"
    );
    let unclassified = message(&report, "unclassified");
    assert!(
        unclassified.contains("`soil class stated` is false"),
        "{unclassified}"
    );
    let unlisted = message(&report, "unlisted");
    assert!(
        unlisted.contains("`limit for the soil class` is false"),
        "{unlisted}"
    );
}

/// A cross-fall band over external decks; the selector decides which
/// decks are checked.
#[test]
fn an_external_deck_falls_across_within_a_band() {
    const SET: &str = "Pset_SlabCommon";
    const EXTERNAL: &str = "IsExternal";
    // The decks run along x; their cross-fall is read along y.
    const NORTH: [f64; 2] = [0.0, 1.0];
    let decks = [
        "draining", "flat", "warped", "internal", "unstated", "unread",
    ];
    let mut model = decks.into_iter().fold(Model::default(), |model, local| {
        model.object(local, "IfcSlab")
    });
    for local in ["draining", "flat", "warped"] {
        model = model.value(local, SET, EXTERNAL, PropertyValue::Boolean(true));
    }
    let model = model
        .value("internal", SET, EXTERNAL, PropertyValue::Boolean(false))
        .value("unstated", SET, EXTERNAL, PropertyValue::Null)
        .unreadable_value("unread", SET, EXTERNAL, "IFCBOOLEAN");
    // The decks the selector leaves out are level, so they would fail
    // were they checked.
    let geometry = decks
        .into_iter()
        .fold(Geometry::new(), |geometry, deck| {
            geometry.face(deck, &[(NORTH, 0.0)])
        })
        // 2 %, falling to one edge.
        .face("draining", &[(NORTH, 0.02)])
        // Half a percent: too flat to drain.
        .face("flat", &[(NORTH, 0.005)])
        // Pieces from 2.5 % to 3.5 %: the band's top lies within.
        .face("warped", &[(NORTH, 0.025), (NORTH, 0.035)]);
    let report = check(&DECK, &geometry.session(model));
    let verdicts = verdicts(&report, "deck-cross-fall");
    assert_eq!(verdicts.found, ["flat"]);
    // An internal deck and one whose `IsExternal` is `null` are not
    // selected; one whose flag cannot be read might be, so it is open.
    assert_eq!(
        verdicts.open,
        [
            ("unread".into(), NotEvaluatedReason::IncompleteEvidence),
            ("warped".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

/// Two risers and a going of every step of a flight within a band, over
/// the flight's measured `steps`.
#[test]
fn a_flight_keeps_its_step_formula() {
    let flights = ["comfortable", "steep", "rough", "single", "unmeasured"];
    let model = flights.into_iter().fold(Model::default(), |model, local| {
        model.object(local, "IfcStairFlight")
    });
    let geometry = Geometry::new()
        // 2 × 0.17 + 0.28 = 0.62 m.
        .flight(flight("comfortable", 12, 0.17, 0.28, 0.0))
        // 2 × 0.20 + 0.28 = 0.68 m, and risers over 0.19 m.
        .flight(flight("steep", 10, 0.20, 0.28, 0.0))
        // 2 × 0.18 + 0.29 = 0.65 m, every position known to 3 mm.
        .flight(flight("rough", 11, 0.18, 0.29, 0.003))
        // One riser: its only step has no going (`null`).
        .flight(flight("single", 1, 0.17, 0.28, 0.0));
    let report = check(&STAIR, &geometry.session(model));
    let verdicts = verdicts(&report, "step-formula");
    assert_eq!(verdicts.found, ["steep"]);
    assert_eq!(verdicts.open.len(), 2, "{:?}", verdicts.open);
    assert_eq!(verdicts.open[0].0, "rough");
    assert_eq!(verdicts.open[0].1, NotEvaluatedReason::IncompleteEvidence);
    assert_eq!(verdicts.open[1].0, "unmeasured");
}

/// A minimum cover by exposure class, raised when prestressed, and a core
/// left in the measured thickness.
#[test]
fn a_slab_carries_the_cover_its_exposure_class_needs() {
    const SET: &str = "Durability";
    let slabs = [
        "sheltered",
        "exposed",
        "prestressed",
        "thin",
        "unclassified",
        "uncovered",
        "unread",
    ];
    let model = slabs
        .into_iter()
        .fold(Model::default(), |model, local| {
            model.object(local, "IfcSlab")
        })
        // 30 mm needed, 35 mm given.
        .value(
            "sheltered",
            SET,
            "ExposureClass",
            text("exterior-sheltered"),
        )
        .value("sheltered", SET, "NominalCover", length(0.035))
        // 40 mm needed, 35 mm given.
        .value("exposed", SET, "ExposureClass", text("exterior-exposed"))
        .value("exposed", SET, "NominalCover", length(0.035))
        // 30 mm and 10 mm for prestressing needed, 35 mm given.
        .value(
            "prestressed",
            SET,
            "ExposureClass",
            text("exterior-sheltered"),
        )
        .value("prestressed", SET, "NominalCover", length(0.035))
        .value(
            "prestressed",
            SET,
            "Prestressed",
            PropertyValue::Boolean(true),
        )
        // Cover enough, but a thickness of 0.16 to 0.18 m leaves a core
        // of 0.09 to 0.11 m.
        .value("thin", SET, "ExposureClass", text("interior-dry"))
        .value("thin", SET, "NominalCover", length(0.035))
        // No exposure class: nothing is required.
        .value("unclassified", SET, "ExposureClass", PropertyValue::Null)
        .value("unclassified", SET, "NominalCover", length(0.01))
        // A class, but the cover is stated as `null`.
        .value("uncovered", SET, "ExposureClass", text("interior-dry"))
        .value("uncovered", SET, "NominalCover", PropertyValue::Null)
        // A cover the source cannot read.
        .value("unread", SET, "ExposureClass", text("interior-dry"))
        .unreadable_value("unread", SET, "NominalCover", "IFCPOSITIVELENGTHMEASURE");
    let geometry = slabs
        .into_iter()
        .fold(Geometry::new(), |geometry, slab| {
            geometry.height(slab, 0.25, 0.25)
        })
        .height("thin", 0.16, 0.18);
    let report = check(&COVER, &geometry.session(model));
    let verdicts = verdicts(&report, "cover-by-exposure");
    assert_eq!(verdicts.found, ["exposed", "prestressed", "uncovered"]);
    assert_eq!(
        verdicts.open,
        [
            ("thin".into(), NotEvaluatedReason::IncompleteEvidence),
            ("unread".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
    // A finding names the subexpression that failed and the values read; a
    // cover stated as `null` leaves the requirement unconfirmed, a
    // missing-information finding and never a pass.
    let uncovered = message(&report, "uncovered");
    assert!(
        uncovered.contains("cannot be confirmed: `cover meets the class` is null"),
        "{uncovered}"
    );
    let exposed = message(&report, "exposed");
    assert!(
        exposed.contains("`cover meets the class` is false"),
        "{exposed}"
    );
}
