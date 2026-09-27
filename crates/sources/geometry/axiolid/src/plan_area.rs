//! Plan-projected footprint and overlap areas.
//!
//! ADR 0004: this module measures areas; whether a storey's spaces cover
//! enough of it, or a space lies within a compartment, is a rule's decision.
//!
//! A planar mesh is the object's shape, so its footprint measures exactly. A
//! tessellated mesh lies within its declared chord deviation `d` of the true
//! surface, so the true footprint boundary lies within `d` of the measured
//! one. The two regions then differ only inside a band of width `2d` along
//! the measured boundary, whose area is at most `2·P·d + π·d²` for a
//! boundary of length `P`. The area is reported as that interval, never as a
//! point.
//!
//! A bodiless group the host declares (a zone) has the union of its members'
//! footprints. Membership is semantic, so the host states it; a member without
//! a body, an unmeasured member or undecided membership refuses, never zero.
//!
//! An uncovered area is the footprint less its overlap with the union of the
//! cover's footprints grown by a radius `r`. A disc has no exact polygon, so
//! the grown cover is bracketed between the cover grown by a regular polygon
//! inside the disc and one around it: the uncovered area lies between the
//! two, and both reach exactly `r` along the plan axes, so only corners can
//! differ. A tessellated cover with deviation `d` is bracketed the same way
//! between growth by `r − d` and `r + d` (every point within `r − d` of the
//! measured cover lies within `r` of the true one, and every point within `r`
//! of the true cover within `r + d` of the measured one). When `r < d` the
//! inner growth is the measured cover itself, widened by its band. The
//! subject's own band widens both ends.
//!
//! A band between two footprints is the convex hull of both, cut to the
//! positions along the band's direction that both reach. The hull's
//! vertices are footprint vertices, but the two cuts are computed, so each
//! cut is moved by `CUT_MARGIN` (0.1 µm, more for large coordinates) inwards for the
//! surely covered part and outwards for the possibly covered one: the area
//! outside the bands lies between the two. A band bounded by a tessellated
//! footprint refuses; its hull is not bounded here.

use std::collections::BTreeMap;

use axiolid_core::Point2;
use axiolid_mesh::TriMesh;
use axiolid_overlay::{Polygon, Ring};
use axioval_engine::{
    CoverageEvidence, CoverageRequest, ElevationCover, ElevationRequest, GeometryFidelity,
    PlanArea, PlanAreaError, PlanAreaService, PlanBand,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::planar::{
    Disc, footprint_measure, grown_polygons, hull_of, plan_overlap_area, polygons_overlap_area,
    projected_polygons, ring_area,
};

/// How far a computed band cut is moved, in metres, plus `CUT_SCALE` of the
/// largest coordinate it is computed from: far above the rounding of a dot
/// product and of a point on a hull edge, and above the overlay's linear
/// tolerance, so a cut never lands on a hull vertex it would be refused
/// beside.
const CUT_MARGIN: f64 = 1e-7;
const CUT_SCALE: f64 = 1e-12;

/// Overlay tolerance: tight, because exact evidence must not be laundered
/// through a loose one.
const LINEAR_TOLERANCE: f64 = 1e-9;
const ANGULAR_TOLERANCE: f64 = 1e-9;

/// A measured footprint and the triangles it was measured from.
pub(crate) struct Footprint {
    pub(crate) soup: Vec<Triangle>,
    pub(crate) area: f64,
    pub(crate) perimeter: f64,
    pub(crate) deviation: f64,
}

/// Measures plan areas of registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidPlanAreaService {
    geometry: AxiolidGeometry,
    source: SourceId,
    /// Exact voids of bodiless openings, or `None` when unmeasured.
    voids: BTreeMap<ObjectId, Option<TriMesh>>,
}

impl AxiolidPlanAreaService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self {
            geometry,
            source,
            voids: BTreeMap::new(),
        }
    }

    /// The exact void of an opening the geometry declares bodiless, so an
    /// effect can continue through it into a connected space.
    #[must_use]
    pub fn with_opening_void(mut self, opening: ObjectId, mesh: TriMesh) -> Self {
        self.voids.insert(opening, Some(mesh));
        self
    }

    /// An opening whose void has no exact mesh: an effect is never continued
    /// through it, and a coverage it may widen keeps its upper bound at the
    /// whole footprint.
    #[must_use]
    pub fn with_unmeasured_opening_void(mut self, opening: ObjectId) -> Self {
        self.voids.insert(opening, None);
        self
    }

    /// The exact void of a bodiless opening, or why there is none.
    pub(crate) fn void(&self, opening: &ObjectId) -> Result<&TriMesh, String> {
        match self.voids.get(opening) {
            Some(Some(mesh)) => Ok(mesh),
            Some(None) => Err(format!("the void of {opening} was not measured")),
            None => Err(format!("{opening} has neither a body nor a void")),
        }
    }

    /// The source every measurement's evidence names.
    pub(crate) fn source(&self) -> &SourceId {
        &self.source
    }

    /// Whether the host declared the object bodiless.
    pub(crate) fn has_no_body(&self, object: &ObjectId) -> bool {
        self.geometry.has_no_body(object)
    }

    fn deviation(&self, object: &ObjectId) -> Result<f64, PlanAreaError> {
        match self.geometry.fidelity(object) {
            Ok(GeometryFidelity::Exact) => Ok(0.0),
            Ok(GeometryFidelity::Tessellated {
                chord_deviation_metres,
            }) => Ok(chord_deviation_metres),
            Err(_) => Err(PlanAreaError::InvalidMeasurement),
        }
    }

    /// The object's plan triangles, footprint area and perimeter, and the
    /// largest chord deviation among the meshes they came from.
    pub(crate) fn measure(&self, object: &ObjectId) -> Result<Footprint, PlanAreaError> {
        let mut soup = Vec::new();
        let mut deviation = 0.0_f64;
        self.collect(object, None, &mut Vec::new(), &mut soup, &mut deviation)?;
        let (area, perimeter) = footprint_measure(&soup, tolerance()?).ok_or_else(|| {
            PlanAreaError::Unavailable(format!("the footprint of {object} cannot be computed"))
        })?;
        Ok(Footprint {
            soup,
            area,
            perimeter,
            deviation,
        })
    }

    /// Gathers the triangles whose plan union is `object`'s footprint.
    ///
    /// A declared group contributes its members' triangles: the overlay
    /// unions them, so members that overlap count once. The union's true
    /// boundary lies within the largest member deviation of the measured one,
    /// so that deviation bounds the whole group's band.
    fn collect(
        &self,
        object: &ObjectId,
        group: Option<&ObjectId>,
        visiting: &mut Vec<ObjectId>,
        soup: &mut Vec<Triangle>,
        deviation: &mut f64,
    ) -> Result<(), PlanAreaError> {
        let named = group.map_or_else(
            || object.to_string(),
            |group| format!("{object} (a member of {group})"),
        );
        if let Some(members) = self.geometry.group_members(object) {
            let members = members.map_err(|reason| {
                PlanAreaError::Unavailable(format!(
                    "the members of {named} are undecided: {reason}"
                ))
            })?;
            if visiting.contains(object) {
                return Err(PlanAreaError::Unavailable(format!(
                    "{object} is a member of itself, so it has no footprint"
                )));
            }
            // An empty group is not an empty footprint: nothing states where it is.
            if members.is_empty() {
                return Err(PlanAreaError::Unavailable(format!(
                    "{named} groups nothing, so it has no footprint"
                )));
            }
            visiting.push(object.clone());
            for member in members {
                self.collect(member, Some(object), visiting, soup, deviation)?;
            }
            visiting.pop();
            return Ok(());
        }
        if self.geometry.has_no_body(object) {
            // A declared bodiless object (a storey) covers nothing, exactly.
            // A group member without a body leaves the group's extent unknown.
            return match group {
                None => Ok(()),
                Some(_) => Err(PlanAreaError::Unavailable(format!(
                    "{named} has no body to give it a footprint"
                ))),
            };
        }
        // An unmeasured body exists with an unknown extent: never zero.
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == object)
        {
            return Err(PlanAreaError::Unavailable(format!(
                "{named} has a body that was not measured: {reason}"
            )));
        }
        let mesh = self.geometry.mesh(object).ok_or_else(|| match group {
            None => PlanAreaError::UnknownObject(object.clone()),
            Some(_) => PlanAreaError::Unavailable(format!("{named} has no described geometry")),
        })?;
        soup.extend(triangles(mesh));
        *deviation = deviation.max(self.deviation(object)?);
        Ok(())
    }

    fn area(
        &self,
        measured: f64,
        slack: f64,
        cap: f64,
        locator: String,
    ) -> Result<PlanArea, PlanAreaError> {
        if slack == 0.0 {
            return PlanArea::try_new(
                measured,
                measured,
                Evidence::exact(self.source.clone(), locator),
            );
        }
        let evidence = Evidence {
            source: self.source.clone(),
            locator,
            exact: false,
        };
        PlanArea::try_new(
            (measured - slack).max(0.0),
            (measured + slack).min(cap),
            evidence,
        )
    }
}

pub(crate) fn tolerance() -> Result<axiolid_core::Tolerance, PlanAreaError> {
    axiolid_core::Tolerance::new(LINEAR_TOLERANCE, ANGULAR_TOLERANCE)
        .map_err(|_| PlanAreaError::Unavailable("invalid overlay tolerance".into()))
}

/// The part of the convex polygon `hull` (counter-clockwise) whose position
/// along the unit `direction` lies in `[low, high]`, as fan triangles.
fn cut(hull: &[(f64, f64)], direction: [f64; 2], low: f64, high: f64) -> Vec<Polygon> {
    let along = |(x, y): (f64, f64)| x.mul_add(direction[0], y * direction[1]);
    let clip = |points: Vec<(f64, f64)>, keep: &dyn Fn(f64) -> f64| {
        let mut kept = Vec::new();
        for (index, &current) in points.iter().enumerate() {
            let next = points[(index + 1) % points.len()];
            let (a, b) = (keep(along(current)), keep(along(next)));
            if a >= 0.0 {
                kept.push(current);
            }
            if (a >= 0.0) != (b >= 0.0) {
                let t = a / (a - b);
                kept.push((
                    (next.0 - current.0).mul_add(t, current.0),
                    (next.1 - current.1).mul_add(t, current.1),
                ));
            }
        }
        kept
    };
    let mut clipped = clip(hull.to_vec(), &|position| position - low);
    if clipped.len() >= 3 {
        clipped = clip(clipped, &|position| high - position);
    }
    // A cut through a hull vertex repeats it, which the overlay refuses.
    clipped.dedup_by(|a, b| (a.0 - b.0).hypot(a.1 - b.1) < CUT_MARGIN);
    while clipped.len() > 1 {
        let (first, last) = (clipped[0], clipped[clipped.len() - 1]);
        if (first.0 - last.0).hypot(first.1 - last.1) >= CUT_MARGIN {
            break;
        }
        clipped.pop();
    }
    let Some(&apex) = clipped.first() else {
        return Vec::new();
    };
    clipped[1..]
        .windows(2)
        .filter_map(|pair| {
            let ring = Ring {
                points: [apex, pair[0], pair[1]]
                    .into_iter()
                    .map(|(x, y)| Point2::new(x, y))
                    .collect(),
            };
            (ring_area(&ring) > f64::EPSILON).then_some(Polygon {
                outer: ring,
                holes: Vec::new(),
            })
        })
        .collect()
}

/// Area of the band of width `2d` along a boundary of length `perimeter`.
pub(crate) fn band(perimeter: f64, deviation: f64) -> f64 {
    2.0 * perimeter * deviation + std::f64::consts::PI * deviation * deviation
}

impl PlanAreaService for AxiolidPlanAreaService {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let footprint = self.measure(object)?;
        self.area(
            footprint.area,
            band(footprint.perimeter, footprint.deviation),
            f64::INFINITY,
            format!("footprint:{object}"),
        )
    }

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        let one = self.measure(first)?;
        let other = self.measure(second)?;
        let overlap = plan_overlap_area(&one.soup, &other.soup, tolerance()?).ok_or_else(|| {
            PlanAreaError::Unavailable(format!(
                "the overlap of {first} and {second} cannot be computed"
            ))
        })?;
        // The overlap's boundary runs along both footprints' boundaries, so
        // either one's band can move it.
        let slack = band(one.perimeter, one.deviation) + band(other.perimeter, other.deviation);
        // An overlap never exceeds either footprint.
        let cap = (one.area + band(one.perimeter, one.deviation))
            .min(other.area + band(other.perimeter, other.deviation));
        self.area(
            overlap.min(one.area).min(other.area),
            slack,
            cap,
            format!("plan-overlap:{first}:{second}"),
        )
    }

    fn measure_uncovered_area(
        &self,
        object: &ObjectId,
        cover: &[ObjectId],
        growth_metres: f64,
    ) -> Result<PlanArea, PlanAreaError> {
        if !growth_metres.is_finite() || growth_metres < 0.0 {
            return Err(PlanAreaError::Unavailable(format!(
                "a growth of {growth_metres} m is not a non-negative length"
            )));
        }
        let subject = self.measure(object)?;
        let mut soup = Vec::new();
        let mut deviation = 0.0_f64;
        for member in cover {
            let footprint = self.measure(member)?;
            soup.extend(footprint.soup);
            deviation = deviation.max(footprint.deviation);
        }
        let tolerance = tolerance()?;
        let overlap = |polygons| {
            polygons_overlap_area(projected_polygons(&subject.soup), polygons, tolerance)
                .map(|area| area.min(subject.area))
                .ok_or_else(|| {
                    PlanAreaError::Unavailable(format!("the cover of {object} cannot be computed"))
                })
        };
        // Surely covered: the cover grown by less than the true growth, or
        // the measured cover less its band.
        let (inner, inner_band) = if growth_metres >= deviation {
            (
                overlap(grown_polygons(
                    &soup,
                    growth_metres - deviation,
                    Disc::Inscribed,
                ))?,
                0.0,
            )
        } else {
            let (_, perimeter) = footprint_measure(&soup, tolerance).ok_or_else(|| {
                PlanAreaError::Unavailable(format!("the cover of {object} cannot be computed"))
            })?;
            (
                overlap(projected_polygons(&soup))?,
                band(perimeter, deviation),
            )
        };
        // Possibly covered: the cover grown by more than the true growth.
        let outer = overlap(grown_polygons(
            &soup,
            growth_metres + deviation,
            Disc::Circumscribed,
        ))?;
        let subject_band = band(subject.perimeter, subject.deviation);
        let upper = (subject.area - inner + subject_band + inner_band)
            .min(subject.area + subject_band)
            .max(0.0);
        let lower = (subject.area - outer - subject_band).max(0.0).min(upper);
        let locator = format!(
            "uncovered-area:{object}:{growth_metres}:{}",
            cover
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        PlanArea::try_new(
            lower,
            upper,
            Evidence {
                source: self.source.clone(),
                locator,
                exact,
            },
        )
    }

    fn measure_outside_bands(
        &self,
        object: &ObjectId,
        bands: &[PlanBand],
    ) -> Result<PlanArea, PlanAreaError> {
        let subject = self.measure(object)?;
        let (mut inner, mut outer) = (Vec::new(), Vec::new());
        for band in bands {
            let [first, second] = band.objects();
            let direction = band.direction();
            let mut points = Vec::new();
            let mut reach = (f64::NEG_INFINITY, f64::INFINITY);
            for member in [first, second] {
                let footprint = self.measure(member)?;
                if footprint.deviation > 0.0 {
                    return Err(PlanAreaError::Unavailable(format!(
                        "{member} is tessellated, so the band it bounds is not measured"
                    )));
                }
                let vertices: Vec<(f64, f64)> = footprint
                    .soup
                    .iter()
                    .flatten()
                    .map(|point| (point.x, point.y))
                    .collect();
                if vertices.is_empty() {
                    return Err(PlanAreaError::Unavailable(format!(
                        "{member} has no footprint to bound a band"
                    )));
                }
                let positions = vertices
                    .iter()
                    .map(|(x, y)| x.mul_add(direction[0], y * direction[1]));
                let low = positions.clone().fold(f64::INFINITY, f64::min);
                let high = positions.fold(f64::NEG_INFINITY, f64::max);
                reach = (reach.0.max(low), reach.1.min(high));
                points.extend(vertices);
            }
            let size = points
                .iter()
                .fold(0.0_f64, |size, (x, y)| size.max(x.abs()).max(y.abs()));
            let margin = CUT_SCALE.mul_add(size, CUT_MARGIN);
            let hull = hull_of(points);
            inner.extend(cut(&hull, direction, reach.0 + margin, reach.1 - margin));
            outer.extend(cut(&hull, direction, reach.0 - margin, reach.1 + margin));
        }
        let tolerance = tolerance()?;
        let overlap = |polygons: Vec<Polygon>| {
            polygons_overlap_area(projected_polygons(&subject.soup), polygons, tolerance)
                .map(|area| area.min(subject.area))
                .ok_or_else(|| {
                    PlanAreaError::Unavailable(format!(
                        "the bands over {object} cannot be computed"
                    ))
                })
        };
        let (inner, outer) = (overlap(inner)?, overlap(outer)?);
        let subject_band = band(subject.perimeter, subject.deviation);
        let upper = (subject.area - inner + subject_band).max(0.0);
        let lower = (subject.area - outer - subject_band).max(0.0).min(upper);
        let locator = format!(
            "outside-bands:{object}:{}",
            bands
                .iter()
                .map(|band| {
                    let [first, second] = band.objects();
                    let [x, y] = band.direction();
                    format!("{first}|{second}|({x:.6},{y:.6})")
                })
                .collect::<Vec<_>>()
                .join(",")
        );
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        PlanArea::try_new(
            lower,
            upper,
            Evidence {
                source: self.source.clone(),
                locator,
                exact,
            },
        )
    }

    fn measure_coverage(
        &self,
        request: &CoverageRequest,
    ) -> Result<CoverageEvidence, PlanAreaError> {
        crate::coverage::measure(self, request)
    }

    fn measure_elevation_cover(
        &self,
        request: &ElevationRequest,
    ) -> Result<ElevationCover, PlanAreaError> {
        crate::elevation::measure(self, request)
    }
}
