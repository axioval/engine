//! Measured extents of the objects a report names, for fitting BCF cameras.
//!
//! The geometry bridge registers a proximity service per session; this
//! reads it back. It lives in the CLI because BCF is a sink the bridge
//! never links.

use std::collections::{BTreeMap, BTreeSet};

use axioval::bcf;
use axioval::engine::{EvidenceSession, LocalOrigins, ProximityServiceHandle};
use axioval::ir::{ObjectId, Report};

/// The measured extent of every object `report` names, for fitting BCF
/// cameras: each session's proximity service answers for its own objects.
///
/// An object no session measured is left out, so its viewpoint gets no
/// camera rather than a guessed one. The extent is the box that encloses the
/// true body (the mesh box grown by its chord deviation).
pub fn bounds(sessions: &[&EvidenceSession], report: &Report) -> BTreeMap<ObjectId, bcf::Bounds> {
    let mut named: BTreeSet<&ObjectId> = BTreeSet::new();
    for finding in report.findings() {
        named.extend(finding.object_id());
        named.extend(&finding.related);
    }
    for outcome in report.not_evaluated() {
        named.extend(outcome.object_id());
    }
    let mut bounds = BTreeMap::new();
    for object in named {
        let measured = sessions
            .iter()
            .filter(|session| session.project().object(object).is_some())
            .find_map(|session| {
                let measured = session
                    .service::<ProximityServiceHandle>()?
                    .bounds(object)
                    .ok()?;
                // In model coordinates, wherever the bodies are measured.
                let origin = session
                    .service::<LocalOrigins>()
                    .map(|origins| origins.of(&object.source))
                    .unwrap_or_default();
                let extent = measured.enclosing();
                let grow = origin.rounding(
                    extent
                        .min()
                        .into_iter()
                        .chain(extent.max())
                        .fold(0.0_f64, |largest, value| largest.max(value.abs())),
                );
                let [x0, y0, z0] = origin.point_to_model(extent.min());
                let [x1, y1, z1] = origin.point_to_model(extent.max());
                bcf::Bounds::new([x0 - grow, y0 - grow, z0], [x1 + grow, y1 + grow, z1])
            });
        if let Some(measured) = measured {
            bounds.insert(object.clone(), measured);
        }
    }
    bounds
}
