//! Whether the tops of supports hold a clearance footprint, from real
//! geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry};
use axioval_engine::{
    BoxClearance, ClearanceShape, CylinderClearance, FreeSpaceError, FreeSpaceService,
    FreeSpaceServiceHandle, MetricDirection, MetricFrame, MetricPoint, SupportCoverageOutcome,
    SupportCoverageRequest,
};
use axioval_ir::{ObjectId, SourceId};
use std::sync::Arc;

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented axis-aligned box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    TriMesh::new(
        vec![
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
            Point3::new(x0, y0, z1),
            Point3::new(x1, y0, z1),
            Point3::new(x1, y1, z1),
            Point3::new(x0, y1, z1),
        ],
        vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ],
    )
}

fn frame_at(x: f64, y: f64) -> MetricFrame {
    MetricFrame::try_new(
        MetricPoint::try_new(id("door"), [x, y, 0.0]).expect("valid point"),
        MetricDirection::try_new([1.0, 0.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 1.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 0.0, 1.0]).expect("valid direction"),
    )
    .expect("valid frame")
}

fn square(side: f64) -> ClearanceShape {
    ClearanceShape::Box(BoxClearance::try_new(side, side, 2.0).expect("valid box"))
}

/// A slab x 0..4, y 0..2 whose top lies at 0, a landing x 0..4, y 2..3
/// whose top lies at -0.15, and a ramp-like step.
fn ground() -> AxiolidFreeSpaceService {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), cuboid([0.0, 0.0, -0.2], [4.0, 2.0, 0.0]))
        .with_mesh(id("landing"), cuboid([0.0, 2.0, -0.35], [4.0, 3.0, -0.15]))
        .with_no_body(id("zone"));
    AxiolidFreeSpaceService::new(geometry, source())
}

fn assess(
    frame: MetricFrame,
    shape: ClearanceShape,
    supports: &[&str],
    band: (f64, f64),
) -> Result<SupportCoverageOutcome, FreeSpaceError> {
    let request = SupportCoverageRequest::try_new(
        frame,
        shape,
        supports.iter().map(|support| id(support)).collect(),
        band.0,
        band.1,
    )?;
    FreeSpaceServiceHandle::new(Arc::new(ground())).assess_support_coverage(&request)
}

#[test]
fn a_footprint_on_the_slab_is_supported() {
    assert!(matches!(
        assess(frame_at(1.0, 1.0), square(1.0), &["slab"], (-0.02, 0.02)),
        Ok(SupportCoverageOutcome::Supported(_))
    ));
}

#[test]
fn a_footprint_over_the_slab_edge_is_unsupported() {
    // y 1.5 to 2.5: half over the landing, 0.15 m lower.
    assert!(matches!(
        assess(
            frame_at(1.0, 2.0),
            square(1.0),
            &["slab", "landing"],
            (-0.02, 0.02)
        ),
        Ok(SupportCoverageOutcome::Unsupported(_))
    ));
    // Within a band reaching the landing's top it is supported.
    assert!(matches!(
        assess(
            frame_at(1.0, 2.0),
            square(1.0),
            &["slab", "landing"],
            (-0.2, 0.02)
        ),
        Ok(SupportCoverageOutcome::Supported(_))
    ));
}

#[test]
fn only_upward_faces_in_the_band_support() {
    // The slab's underside lies at -0.2: a band around it finds no top.
    assert!(matches!(
        assess(frame_at(1.0, 1.0), square(1.0), &["slab"], (-0.25, -0.15)),
        Ok(SupportCoverageOutcome::Unsupported(_))
    ));
    // No supports hold nothing.
    assert!(matches!(
        assess(frame_at(1.0, 1.0), square(1.0), &[], (-0.02, 0.02)),
        Ok(SupportCoverageOutcome::Unsupported(_))
    ));
}

#[test]
fn a_disc_by_the_slab_edge_is_refused() {
    // A disc of radius 0.5 whose edge touches the slab's edge: the
    // circumscribed polygon overhangs, the inscribed one does not.
    let disc = ClearanceShape::Cylinder(CylinderClearance::try_new(0.5, 2.0).unwrap());
    assert!(matches!(
        assess(frame_at(1.0, 1.5), disc, &["slab"], (-0.02, 0.02)),
        Err(FreeSpaceError::Unavailable(_))
    ));
}

#[test]
fn missing_and_tessellated_supports_are_refused() {
    assert!(matches!(
        assess(frame_at(1.0, 1.0), square(1.0), &["zone"], (-0.02, 0.02)),
        Err(FreeSpaceError::MissingGeometry(_))
    ));
    let geometry = AxiolidGeometry::new().with_tessellated_mesh(
        id("slab"),
        cuboid([0.0, 0.0, -0.2], [4.0, 2.0, 0.0]),
        0.001,
    );
    let request = SupportCoverageRequest::try_new(
        frame_at(1.0, 1.0),
        square(1.0),
        vec![id("slab")],
        -0.02,
        0.02,
    )
    .unwrap();
    assert!(matches!(
        AxiolidFreeSpaceService::new(geometry, source()).assess_support_coverage(&request),
        Err(FreeSpaceError::Unavailable(_))
    ));
    assert!(matches!(
        SupportCoverageRequest::try_new(frame_at(1.0, 1.0), square(1.0), Vec::new(), 0.1, 0.0),
        Err(FreeSpaceError::InvalidElevationBand)
    ));
}
