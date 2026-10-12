//! Setback areas in front of external walls (#326), measured over a
//! plan-area service of axis-aligned rectangles, outward faces stated per
//! wall and a level terrain known to 5 cm.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, ContextZones, ElevationInterval, FacadeArea, FacadeAreaError,
    FacadeAreaService, FacadeAreaServiceHandle, FacadeFace, FacadeFaceSide, FacadeFaces,
    FaceNormal, FootprintOutline, PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle,
    PropertyResolution, ServiceRegistry, SetbackArea, SetbackRequest, SuppliedZones, TerrainFrame,
    TerrainKind, TerrainServiceHandle, TerrainSurface, TinTerrain, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle, measured_value,
};
use axioval_ir::{Evidence, ObjectId, Project, PropertyValue};
use common::{Model, id, source};

/// A footprint `[x0, y0, x1, y1]`.
type Rectangle = [f64; 4];

fn size(r: Rectangle) -> f64 {
    (r[2] - r[0]) * (r[3] - r[1])
}

fn overlap(a: Rectangle, b: Rectangle) -> f64 {
    let width = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let depth = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    width * depth
}

/// The strip in front of `r` along the axis direction `[x, y]`, `depth`
/// deep.
fn strip(r: Rectangle, [x, y]: [f64; 2], depth: f64) -> Rectangle {
    if x < -0.5 {
        [r[0] - depth, r[1], r[0], r[3]]
    } else if x > 0.5 {
        [r[2], r[1], r[2] + depth, r[3]]
    } else if y < -0.5 {
        [r[0], r[1] - depth, r[2], r[1]]
    } else {
        [r[0], r[3], r[2], r[3] + depth]
    }
}

/// A face looking outside: its normal, area and side.
type Face = ([f64; 3], f64, FacadeFaceSide);

/// Footprints, outward faces and extents of the walls and roofs; zones
/// are read by their outer ring's bounding box.
struct Site {
    footprints: BTreeMap<ObjectId, Rectangle>,
    /// Each wall's outward faces, and whether they are of the exact shape.
    faces: BTreeMap<ObjectId, (Vec<Face>, bool)>,
    /// Each object's bottom and top.
    extents: BTreeMap<ObjectId, (f64, f64)>,
}

fn evidence(locator: &str) -> Evidence {
    Evidence::exact(source(), locator)
}

impl Site {
    fn footprint(&self, object: &ObjectId) -> Result<Rectangle, PlanAreaError> {
        self.footprints
            .get(object)
            .copied()
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))
    }

    /// Each strip at the least (sure directions, least depth) and most.
    fn strips(
        &self,
        strip_of: &axioval_engine::SetbackStrip,
    ) -> Result<(Vec<Rectangle>, Vec<Rectangle>), PlanAreaError> {
        let footprint = self.footprint(strip_of.object())?;
        let (least, most) = strip_of.depth_metres();
        let mut sure = Vec::new();
        let mut every = Vec::new();
        for direction in strip_of.directions() {
            if direction.is_sure() {
                sure.push(strip(footprint, direction.direction(), least));
            }
            every.push(strip(footprint, direction.direction(), most));
        }
        Ok((sure, every))
    }
}

impl PlanAreaService for Site {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let area = size(self.footprint(object)?);
        PlanArea::try_new(area, area, evidence("footprint"))
    }

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        let area = overlap(self.footprint(first)?, self.footprint(second)?);
        PlanArea::try_new(area, area, evidence("overlap"))
    }

    fn measure_footprint_outline(
        &self,
        objects: &[ObjectId],
    ) -> Result<FootprintOutline, PlanAreaError> {
        let rings = objects
            .iter()
            .map(|object| {
                let r = self.footprint(object)?;
                Ok(vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]])
            })
            .collect::<Result<_, PlanAreaError>>()?;
        FootprintOutline::try_new(objects.to_vec(), rings, 0.0, evidence("outline"))
    }

    fn measure_setback(&self, request: &SetbackRequest) -> Result<SetbackArea, PlanAreaError> {
        let (sure, every) = self.strips(request.strip())?;
        let sum = |rectangles: &[Rectangle]| rectangles.iter().copied().map(size).sum::<f64>();
        let area = (sum(&sure), sum(&every));
        let outside = request.zone().map(|(zone, accuracy)| {
            let outer = zone[0].outer();
            let fold = |pick: fn(f64, f64) -> f64, axis: usize, start: f64| {
                outer.iter().map(|point| point[axis]).fold(start, pick)
            };
            let bounds = [
                fold(f64::min, 0, f64::INFINITY),
                fold(f64::min, 1, f64::INFINITY),
                fold(f64::max, 0, f64::NEG_INFINITY),
                fold(f64::max, 1, f64::NEG_INFINITY),
            ];
            let a = accuracy;
            let inner = [bounds[0] + a, bounds[1] + a, bounds[2] - a, bounds[3] - a];
            let outer = [bounds[0] - a, bounds[1] - a, bounds[2] + a, bounds[3] + a];
            let least: f64 = sure.iter().map(|r| size(*r) - overlap(*r, outer)).sum();
            let most: f64 = every.iter().map(|r| size(*r) - overlap(*r, inner)).sum();
            (least, most.max(least))
        });
        let mut overlapping = Vec::new();
        let overlap_area = if request.others().is_empty() {
            None
        } else {
            let (mut least, mut most) = (0.0, 0.0);
            for other in request.others() {
                let (other_sure, other_every) = self.strips(other.strip())?;
                let touching: f64 = every
                    .iter()
                    .flat_map(|a| other_every.iter().map(move |b| overlap(*a, *b)))
                    .sum();
                if touching > 0.0 {
                    overlapping.push(other.strip().object().clone());
                }
                most += touching;
                if other.is_sure() {
                    least += sure
                        .iter()
                        .flat_map(|a| other_sure.iter().map(move |b| overlap(*a, *b)))
                        .sum::<f64>();
                }
            }
            Some((least, most))
        };
        #[allow(clippy::float_cmp)]
        let exact = [Some(area), outside, overlap_area]
            .iter()
            .flatten()
            .all(|(lower, upper)| lower == upper);
        let mut evidence = evidence("setback");
        evidence.exact = exact;
        SetbackArea::try_new(
            request.strip().object().clone(),
            (Vec::new(), Vec::new()),
            area,
            outside,
            overlap_area,
            overlapping,
            evidence,
        )
    }
}

impl FacadeAreaService for Site {
    fn measure_facade_area(&self, _: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        Err(FacadeAreaError::Unavailable("not measured here".into()))
    }

    fn measure_facade_faces(&self, object: &ObjectId) -> Result<FacadeFaces, FacadeAreaError> {
        let (faces, exact) = self
            .faces
            .get(object)
            .ok_or_else(|| FacadeAreaError::UnknownObject(object.clone()))?;
        let faces = faces
            .iter()
            .map(|(normal, area, side)| {
                FacadeFace::try_new(
                    FaceNormal::exact(*normal).unwrap(),
                    (*area, *area),
                    [0.0; 3],
                    *side,
                )
            })
            .collect::<Result<_, _>>()?;
        let mut evidence = evidence("faces");
        evidence.exact = *exact;
        FacadeFaces::try_new(object.clone(), faces, evidence)
    }
}

impl VerticalExtentService for Site {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let (bottom, top) = self
            .extents
            .get(object)
            .copied()
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::exact(bottom)?,
            ElevationInterval::exact(top)?,
            evidence("extent"),
        )
    }
}

/// A level terrain at height 0 over x and y -100..100, stated to 5 cm.
fn terrain() -> TerrainServiceHandle {
    let surface = TerrainSurface::try_new(
        TerrainKind::Original,
        "Ground",
        TerrainFrame::Model,
        None,
        0.05,
        ("ground.xml", "0".repeat(64)),
        &[
            [-100.0, -100.0, 0.0],
            [100.0, -100.0, 0.0],
            [100.0, 100.0, 0.0],
            [-100.0, 100.0, 0.0],
        ],
        vec![[0, 1, 2], [0, 2, 3]],
    )
    .unwrap();
    TerrainServiceHandle::new(Arc::new(TinTerrain::try_new(vec![surface]).unwrap()))
}

/// Parcel `P1`, x -5..15 and y -5..5.
const ZONES: &str = r#"{"type": "FeatureCollection",
  "crs": {"type": "name", "properties": {"name": "axioval:model"}},
  "accuracy": 0,
  "features": [
    {"type": "Feature", "id": "P1", "properties": {"kind": "parcel"},
     "geometry": {"type": "Polygon", "coordinates": [[[-5, -5], [15, -5], [15, 5], [-5, 5], [-5, -5]]]}}
  ]}"#;

const SOUTH: [f64; 3] = [0.0, -1.0, 0.0];
const NORTH: [f64; 3] = [0.0, 1.0, 0.0];
const UP: [f64; 3] = [0.0, 0.0, 1.0];

/// Wall `south` (x 0..10, y 0..0.3, 6 m high) looks south, with a roof
/// `roof` over it rising to 9 m; wall `facing` of a second building (x
/// 4..14, y -8.3..-8) looks north at it; `inner` looks outside nowhere,
/// `free` both ways, and `round` is a tessellation.
fn session(terrain_supplied: bool) -> (Project, ServiceRegistry) {
    use FacadeFaceSide::Outside;
    let model = Model::default()
        .object("south", "wall")
        .object("facing", "wall")
        .object("inner", "wall")
        .object("free", "screen")
        .object("round", "screen")
        .object("roof", "roof");
    let site = Site {
        footprints: BTreeMap::from([
            (id("south"), [0.0, 0.0, 10.0, 0.3]),
            (id("facing"), [4.0, -8.3, 14.0, -8.0]),
            (id("inner"), [0.0, 4.0, 10.0, 4.3]),
            (id("free"), [30.0, 0.0, 40.0, 0.3]),
            (id("round"), [50.0, 0.0, 60.0, 0.3]),
            (id("roof"), [-0.5, -0.5, 10.5, 9.0]),
        ]),
        faces: BTreeMap::from([
            (
                id("south"),
                (vec![(SOUTH, 60.0, Outside), (UP, 3.0, Outside)], true),
            ),
            (id("facing"), (vec![(NORTH, 60.0, Outside)], true)),
            (id("inner"), (vec![(UP, 3.0, Outside)], true)),
            (
                id("free"),
                (vec![(SOUTH, 30.0, Outside), (NORTH, 30.0, Outside)], true),
            ),
            (id("round"), (vec![(SOUTH, 30.0, Outside)], false)),
        ]),
        extents: BTreeMap::from([
            (id("south"), (0.0, 6.0)),
            (id("facing"), (0.0, 6.0)),
            (id("inner"), (0.0, 6.0)),
            (id("free"), (0.0, 3.0)),
            (id("round"), (0.0, 3.0)),
            (id("roof"), (6.0, 9.0)),
        ]),
    };
    let site = Arc::new(site);
    let mut services = ServiceRegistry::new();
    services
        .register(PlanAreaServiceHandle::new(site.clone()))
        .unwrap();
    services
        .register(FacadeAreaServiceHandle::new(site.clone()))
        .unwrap();
    services
        .register(VerticalExtentServiceHandle::new(site))
        .unwrap();
    if terrain_supplied {
        services.register(terrain()).unwrap();
    }
    let file = ContextZones::read_geojson(ZONES.as_bytes(), "zones.geojson").unwrap();
    services
        .register(SuppliedZones::try_from_files(vec![file]).unwrap())
        .unwrap();
    let project = model.project();
    axioval_rules::register_builtins(CapabilityRegistry::new())
        .unwrap()
        .install_measured(&mut services, &project);
    (project, services)
}

/// `name` read on `object`: its bounds and whether its evidence is exact.
fn value(
    (project, services): &(Project, ServiceRegistry),
    object: &str,
    name: &str,
) -> Result<(f64, f64, bool), String> {
    match measured_value(services, project, &id(object), name) {
        Ok(PropertyResolution::Present(resolved)) => {
            let property = resolved.into_property();
            let exact = property.evidence.iter().all(|evidence| evidence.exact);
            match property.value {
                PropertyValue::Quantity { value, .. } | PropertyValue::Decimal(value) => {
                    Ok((value, value, exact))
                }
                PropertyValue::Measured { lower, upper, .. } => Ok((lower, upper, exact)),
                other => Err(format!("{other:?}")),
            }
        }
        Ok(PropertyResolution::Absent(_)) => Err("absent".into()),
        Err(error) => Err(error.to_string()),
    }
}

/// Holds `expected` within `width`, and no wider.
fn holds((lower, upper, _): (f64, f64, bool), expected: f64, width: f64) -> bool {
    lower <= expected + 1e-9 && expected - 1e-9 <= upper && upper - lower <= width + 1e-6
}

const DEEP: &str = "terrain=original;factor=1";

#[test]
fn the_depth_is_the_factor_of_the_height_above_the_terrain_at_least_the_minimum() {
    let site = session(true);
    // 6 m above a terrain known to 5 cm.
    let depth = value(&site, "south", &format!("setback_depth;{DEEP}")).unwrap();
    assert!(holds(depth, 6.0, 0.1), "{depth:?}");
    assert!(!depth.2, "the terrain's accuracy makes it inexact");
    let depth = value(&site, "south", "setback_depth;terrain=original;factor=0.4").unwrap();
    assert!(holds(depth, 2.4, 0.04), "{depth:?}");
    let depth = value(
        &site,
        "south",
        "setback_depth;terrain=original;factor=0.4;minimum=3",
    )
    .unwrap();
    assert!(holds(depth, 3.0, 0.0), "{depth:?}");
    // A third of the roof's 3 m rise above the wall adds 1 m.
    let depth = value(
        &site,
        "south",
        "setback_depth;terrain=original;factor=1;roof=roof;roof_share=0.3333333333333333",
    )
    .unwrap();
    assert!(holds(depth, 7.0, 0.1), "{depth:?}");
}

#[test]
fn every_external_wall_has_its_setback_measured() {
    let site = session(true);
    // 10 m along the wall, 6 ± 0.05 m deep.
    let area = value(&site, "south", &format!("setback_area;{DEEP}")).unwrap();
    assert!(holds(area, 60.0, 1.0), "{area:?}");
    let area = value(&site, "facing", &format!("setback_area;{DEEP}")).unwrap();
    assert!(holds(area, 60.0, 1.0), "{area:?}");
    // No steep face of an internal wall looks outside: no setback, exactly.
    let area = value(&site, "inner", &format!("setback_area;{DEEP}")).unwrap();
    assert_eq!(area, (0.0, 0.0, true));
}

#[test]
fn a_setback_crossing_its_parcel_measures_the_area_beyond_it() {
    let site = session(true);
    // 6 m deep, the parcel's edge 5 m in front: 10 m² beyond it.
    let outside = value(
        &site,
        "south",
        &format!("setback_outside_area;{DEEP};zone=P1"),
    )
    .unwrap();
    assert!(holds(outside, 10.0, 1.0), "{outside:?}");
    let kept = value(
        &site,
        "south",
        "setback_outside_area;terrain=original;factor=0.4;zone=P1",
    )
    .unwrap();
    assert!(holds(kept, 0.0, 0.0), "{kept:?}");
    let why = value(
        &site,
        "south",
        &format!("setback_outside_area;{DEEP};zone=P9"),
    )
    .unwrap_err();
    assert!(why.contains("no supplied context zone is `P9`"), "{why}");
}

#[test]
fn overlapping_setbacks_of_two_buildings_are_measured() {
    let site = session(true);
    // Each 6 ± 0.05 m deep across an 8 m gap overlap 4 m deep over x
    // 4..10.
    let name = format!("setback_overlap_area;{DEEP};others=wall");
    let overlap = value(&site, "south", &name).unwrap();
    assert!(holds(overlap, 24.0, 1.2), "{overlap:?}");
    let overlap = value(&site, "facing", &name).unwrap();
    assert!(holds(overlap, 24.0, 1.2), "{overlap:?}");
    // At 0.4 H the two keep apart; the internal wall has no setback.
    let apart = value(
        &site,
        "south",
        "setback_overlap_area;terrain=original;factor=0.4;others=wall",
    )
    .unwrap();
    assert!(holds(apart, 0.0, 0.0), "{apart:?}");
    // A compared wall whose outward side is undecided leaves it open.
    let why = value(
        &site,
        "south",
        &format!("setback_overlap_area;{DEEP};others=wall,screen"),
    )
    .unwrap_err();
    assert!(why.contains("which side faces out is undecided"), "{why}");
}

#[test]
fn an_undecided_side_a_tessellation_or_no_terrain_is_not_evaluated() {
    let site = session(true);
    let why = value(&site, "free", &format!("setback_area;{DEEP}")).unwrap_err();
    assert!(why.contains("which side faces out is undecided"), "{why}");
    let why = value(&site, "round", &format!("setback_area;{DEEP}")).unwrap_err();
    assert!(why.contains("tessellation"), "{why}");
    let why = value(
        &site,
        "south",
        "setback_depth;terrain=original;factor=1;roof=roof",
    )
    .unwrap_err();
    assert!(why.contains("without `roof_share`"), "{why}");
    let bare = session(false);
    let why = value(&bare, "south", &format!("setback_area;{DEEP}")).unwrap_err();
    assert!(why.contains("terrain"), "{why}");
}

#[test]
fn a_setback_measured_over_an_approximate_terrain_is_inexact() {
    let site = session(true);
    // The terrain is known to 5 cm: no setback value over it is exact,
    // whatever its interval.
    for name in [
        format!("setback_depth;{DEEP}"),
        format!("setback_area;{DEEP}"),
        format!("setback_outside_area;{DEEP};zone=P1"),
        format!("setback_overlap_area;{DEEP};others=wall"),
    ] {
        let (lower, upper, exact) = value(&site, "south", &name).unwrap();
        assert!(!exact, "{name}");
        assert!(lower <= upper, "{name}");
    }
}
