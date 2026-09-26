//! Envelope membership derived from geometry.
//!
//! ADR 0004: this module measures. It reports which objects the model DECLARES
//! to be on the envelope and which objects geometry DERIVES as being there;
//! whether those two sets agreeing matters is the capability's decision.
//!
//! ADR 0001 compliance: the declared set arrives as engine IR (`ObjectId`),
//! supplied by whichever adapter read the model. This crate never imports an
//! IFC type, so accepting declarations as data creates no combined adapter --
//! a proprietary CAD host supplies its own declarations the same way.

use std::collections::BTreeSet;

use axioval_engine::{
    EnvelopeDerivation, EnvelopeMembershipError, EnvelopeMembershipEvidence,
    EnvelopeMembershipRequest, EnvelopeMembershipService,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::AxiolidGeometry;
use crate::planar::{plan_frame, polygon_area, projected_polygons, ring_segments};
use axiolid_core::Point2;
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, overlay};

/// How far outside the space hull an object may sit and still bound it.
///
/// Deliberately tight: this adapter reports exact evidence, so an object that
/// only touches the hull under a loose tolerance must not be laundered in.
const HULL_TOLERANCE_METRES: f64 = 1.0e-6;

/// Derives envelope membership from supplied geometry and declarations.
///
/// The host registers which objects are spaces, which spaces belong to
/// gross-area groups, and which objects the model declares external or
/// internal. An object declared neither way is undeclared: its declaration is
/// unknown, never taken as internal. Geometry then decides, per derivation,
/// which objects actually bound that envelope.
///
/// An object is on the envelope when its plan footprint overlaps the region
/// the bounding spaces cover and also reaches that region's outline, by
/// crossing it or touching it from inside. An object wholly inside, such as an
/// internal wall within a gross-area space, is not on the envelope. The
/// bounding spaces must therefore reach the envelope's outer faces, as
/// gross-area spaces do; net rooms that stop at the inner wall face do not
/// overlap the external walls at all.
pub struct AxiolidEnvelopeMembershipService {
    geometry: AxiolidGeometry,
    source: SourceId,
    spaces: BTreeSet<ObjectId>,
    gross_area_spaces: BTreeSet<ObjectId>,
    declared_external: BTreeSet<ObjectId>,
    declared_internal: BTreeSet<ObjectId>,
}

impl AxiolidEnvelopeMembershipService {
    /// Creates a service over the supplied geometry and model declarations.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self {
            geometry,
            source,
            spaces: BTreeSet::new(),
            gross_area_spaces: BTreeSet::new(),
            declared_external: BTreeSet::new(),
            declared_internal: BTreeSet::new(),
        }
    }

    /// Marks an object as a space bounding the envelope.
    #[must_use]
    pub fn with_space(mut self, space: ObjectId) -> Self {
        self.spaces.insert(space);
        self
    }

    /// Marks a space as belonging to a gross-area group.
    ///
    /// A gross-area space is also a space: registering it in both sets here
    /// stops a host silently producing an empty `AllSpaces` envelope by
    /// declaring only gross-area membership.
    #[must_use]
    pub fn with_gross_area_space(mut self, space: ObjectId) -> Self {
        self.spaces.insert(space.clone());
        self.gross_area_spaces.insert(space);
        self
    }

    /// Records that the model declares this object to be on the envelope.
    #[must_use]
    pub fn with_declared_external(mut self, object: ObjectId) -> Self {
        self.declared_external.insert(object);
        self
    }

    /// Records that the model declares this object not to be on the envelope.
    #[must_use]
    pub fn with_declared_internal(mut self, object: ObjectId) -> Self {
        self.declared_internal.insert(object);
        self
    }
}

/// Shortest distance between two plan segments.
fn segment_distance(a: (Point2, Point2), b: (Point2, Point2)) -> f64 {
    let point_to_segment = |p: Point2, (s, e): (Point2, Point2)| {
        let (dx, dy) = (e.x - s.x, e.y - s.y);
        let length = dx * dx + dy * dy;
        let t = if length > 0.0 {
            (((p.x - s.x) * dx + (p.y - s.y) * dy) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ((p.x - s.x - t * dx).powi(2) + (p.y - s.y - t * dy).powi(2)).sqrt()
    };
    let cross =
        |o: Point2, p: Point2, q: Point2| (p.x - o.x) * (q.y - o.y) - (p.y - o.y) * (q.x - o.x);
    let (d1, d2) = (cross(b.0, b.1, a.0), cross(b.0, b.1, a.1));
    let (d3, d4) = (cross(a.0, a.1, b.0), cross(a.0, a.1, b.1));
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return 0.0;
    }
    point_to_segment(a.0, b)
        .min(point_to_segment(a.1, b))
        .min(point_to_segment(b.0, a))
        .min(point_to_segment(b.1, a))
}

/// Whether any footprint edge of `triangles` comes within the hull tolerance
/// of the region outline `outline`.
fn reaches_outline(triangles: &[crate::geometry::Triangle], outline: &[(Point2, Point2)]) -> bool {
    triangles.iter().any(|[a, b, c]| {
        let corners = [
            Point2::new(a.x, a.y),
            Point2::new(b.x, b.y),
            Point2::new(c.x, c.y),
        ];
        (0..3).any(|i| {
            let edge = (corners[i], corners[(i + 1) % 3]);
            outline
                .iter()
                .any(|segment| segment_distance(edge, *segment) <= HULL_TOLERANCE_METRES)
        })
    })
}

impl EnvelopeMembershipService for AxiolidEnvelopeMembershipService {
    fn measure_envelope_membership(
        &self,
        request: &EnvelopeMembershipRequest,
    ) -> Result<EnvelopeMembershipEvidence, EnvelopeMembershipError> {
        // Each derivation names its own bounding set. An unknown derivation is
        // an error, never a silently empty envelope.
        let bounding: &BTreeSet<ObjectId> = match request.derivation() {
            EnvelopeDerivation::AllSpaces => &self.spaces,
            EnvelopeDerivation::GrossAreaGroups => &self.gross_area_spaces,
            _ => return Err(EnvelopeMembershipError::UnsupportedDerivation),
        };
        let tolerance = axiolid_core::Tolerance::new(1.0e-9, 1.0e-9)
            .map_err(|_| EnvelopeMembershipError::Unavailable)?;

        let space_triangles: Vec<crate::geometry::Triangle> = bounding
            .iter()
            .filter_map(|space| self.geometry.mesh(space).map(crate::geometry::triangles))
            .flatten()
            .collect();
        // No measurable bounding geometry means there is no envelope to compare
        // against -- whether because no space was declared, or because the
        // declared spaces carry no mesh. Reporting an empty derived set instead
        // would call every declared wall a discrepancy.
        if space_triangles.is_empty() {
            return Err(EnvelopeMembershipError::Unavailable);
        }

        // Membership is plan overlap with a bounding space. A tessellated space,
        // or a tessellated object whose true footprint could reach one, makes
        // that overlap an estimate, and this evidence is exact.
        for space in bounding {
            let Some(extent) = self.geometry.enclosing_extent(space) else {
                continue;
            };
            if self.geometry.is_tessellated(space)
                || self
                    .geometry
                    .tessellated_near(&extent, 0.0, true, |object| bounding.contains(object))
                    .is_some()
            {
                return Err(EnvelopeMembershipError::InexactEvidence);
            }
        }

        // The covered region and its outline, holes (courtyards) included.
        let region = OverlayInput {
            frame: plan_frame(),
            polygons: projected_polygons(&space_triangles),
        };
        let region = overlay(
            &region,
            &region,
            OverlayOperation::Union,
            FillRule::NonZero,
            tolerance,
        )
        .map_err(|_| EnvelopeMembershipError::Unavailable)?;
        let outline: Vec<(Point2, Point2)> = region
            .polygons
            .iter()
            .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
            .flat_map(ring_segments)
            .collect();
        let region = OverlayInput {
            frame: plan_frame(),
            polygons: region.polygons,
        };

        let mut derived = Vec::new();
        let mut undeclared = Vec::new();
        let mut evaluated = 0usize;
        for (object, mesh) in self.geometry.objects() {
            if bounding.contains(object) {
                continue;
            }
            evaluated += 1;
            if !self.declared_external.contains(object) && !self.declared_internal.contains(object)
            {
                undeclared.push(object.clone());
            }
            let candidate = crate::geometry::triangles(mesh);
            let footprint = OverlayInput {
                frame: plan_frame(),
                polygons: projected_polygons(&candidate),
            };
            if footprint.polygons.is_empty() {
                continue;
            }
            let area = |operation| -> Result<f64, EnvelopeMembershipError> {
                overlay(&footprint, &region, operation, FillRule::NonZero, tolerance)
                    .map(|result| result.polygons.iter().map(polygon_area).sum())
                    .map_err(|_| EnvelopeMembershipError::Unavailable)
            };
            if area(OverlayOperation::Intersection)? <= HULL_TOLERANCE_METRES {
                continue;
            }
            if area(OverlayOperation::Difference)? > HULL_TOLERANCE_METRES
                || reaches_outline(&candidate, &outline)
            {
                derived.push(object.clone());
            }
        }

        EnvelopeMembershipEvidence::try_new(
            *request,
            self.declared_external.iter().cloned().collect(),
            derived,
            evaluated,
            Evidence::exact(
                self.source.clone(),
                format!("envelope:{}", request.derivation().as_str()),
            ),
        )
        .map(|evidence| evidence.with_undeclared(undeclared))
    }
}
