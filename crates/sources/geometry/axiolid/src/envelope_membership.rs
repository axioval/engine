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
use crate::planar::{plan_frame, polygon_area, projected_polygons};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, overlay};

/// How far outside the space hull an object may sit and still bound it.
///
/// Deliberately tight: this adapter reports exact evidence, so an object that
/// only touches the hull under a loose tolerance must not be laundered in.
const HULL_TOLERANCE_METRES: f64 = 1.0e-6;

/// Derives envelope membership from supplied geometry and declarations.
///
/// The host registers which objects are spaces, which spaces belong to
/// gross-area groups, and which objects the model declares external. Geometry
/// then decides, per derivation, which objects actually bound that envelope.
pub struct AxiolidEnvelopeMembershipService {
    geometry: AxiolidGeometry,
    source: SourceId,
    spaces: BTreeSet<ObjectId>,
    gross_area_spaces: BTreeSet<ObjectId>,
    declared_external: BTreeSet<ObjectId>,
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
}

/// Plan area shared by two objects' footprints.
///
/// Zero when either projects to nothing in plan, so an object seen edge-on
/// cannot bound an envelope it merely grazes.
fn shared_plan_area(
    subject: &[crate::geometry::Triangle],
    other: &[crate::geometry::Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Result<f64, EnvelopeMembershipError> {
    let frame = plan_frame();
    let subject_input = OverlayInput {
        frame,
        polygons: projected_polygons(subject),
    };
    let other_input = OverlayInput {
        frame,
        polygons: projected_polygons(other),
    };
    if subject_input.polygons.is_empty() || other_input.polygons.is_empty() {
        return Ok(0.0);
    }
    let result = overlay(
        &subject_input,
        &other_input,
        OverlayOperation::Intersection,
        FillRule::NonZero,
        tolerance,
    )
    .map_err(|_| EnvelopeMembershipError::Unavailable)?;
    Ok(result.polygons.iter().map(polygon_area).sum())
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

        let space_triangles: Vec<Vec<crate::geometry::Triangle>> = bounding
            .iter()
            .filter_map(|space| self.geometry.mesh(space).map(crate::geometry::triangles))
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

        let mut derived = Vec::new();
        let mut evaluated = 0usize;
        for (object, mesh) in self.geometry.objects() {
            if bounding.contains(object) {
                continue;
            }
            evaluated += 1;
            let candidate = crate::geometry::triangles(mesh);
            for space in &space_triangles {
                if shared_plan_area(&candidate, space, tolerance)? > HULL_TOLERANCE_METRES {
                    derived.push(object.clone());
                    break;
                }
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
    }
}
