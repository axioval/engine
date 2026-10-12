//! `fall_in_flow`: a run's fall from its inlet port to its outlet port, in
//! the direction its source's distribution network states.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    DistributionNetworkService, DistributionNetworkServiceHandle, FlowGraphs, MetricDirection,
    MetricFrame, MetricPoint, NetworkServiceError, ObjectFrame, ObjectFrameError,
    ObjectFrameService, ObjectFrameServiceHandle, ObjectFront, ServiceRegistry, SourceSnapshot,
};
use axioval_ir::{
    DistributionNetwork, Evidence, FlowDirection, NetworkPort, Object, ObjectId, PortConnection,
    Project, SourceId,
};
use common::{id, source};

/// Port placements, each stated exactly or not.
struct Frames {
    snapshots: Vec<SourceSnapshot>,
    origins: BTreeMap<ObjectId, ([f64; 3], bool)>,
}

impl ObjectFrameService for Frames {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let (origin, exact) = *self
            .origins
            .get(object)
            .ok_or_else(|| ObjectFrameError::NotPlaced(object.clone()))?;
        let axis = |vector| MetricDirection::try_new(vector).unwrap();
        let frame = MetricFrame::try_new(
            MetricPoint::try_new(object.clone(), origin).unwrap(),
            axis([1.0, 0.0, 0.0]),
            axis([0.0, 1.0, 0.0]),
            axis([0.0, 0.0, 1.0]),
        )
        .unwrap();
        let mut evidence = Evidence::exact(source(), format!("placement:{}", object.local_id));
        evidence.exact = exact;
        ObjectFrame::try_new(object.clone(), frame, ObjectFront::NotStated, evidence)
    }
}

struct Listed(DistributionNetwork);

impl DistributionNetworkService for Listed {
    fn network(&self, asked: &SourceId) -> Result<DistributionNetwork, NetworkServiceError> {
        if asked == self.0.source() {
            Ok(self.0.clone())
        } else {
            Err(NetworkServiceError::UncoveredSource(asked.clone()))
        }
    }
}

/// Pipe `p` from inlet `i` at `from` to outlet `o` at `to`, its outlet
/// stating `outlet`, fed by basin `b`; `exact` says whether the outlet's
/// placement is stated exactly.
fn run(
    from: [f64; 3],
    to: [f64; 3],
    outlet: Option<FlowDirection>,
    exact: bool,
) -> (ServiceRegistry, Project) {
    let port = |port: &str, element: &str, flow| NetworkPort {
        port: id(port),
        elements: vec![id(element)],
        flow,
        evidence: vec![Evidence::exact(source(), format!("port:{port}"))],
    };
    let network = DistributionNetwork::try_new(
        source(),
        vec![
            port("bo", "b", Some(FlowDirection::Source)),
            port("i", "p", Some(FlowDirection::Sink)),
            port("o", "p", outlet),
        ],
        vec![PortConnection {
            relating: id("bo"),
            related: id("i"),
            realizing: None,
            evidence: Evidence::exact(source(), "connection:bo-i"),
        }],
        vec![Evidence::exact(source(), "network-scan")],
    )
    .unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(Arc::new(FlowGraphs::new(
            DistributionNetworkServiceHandle::new(Arc::new(Listed(network))),
        )))
        .unwrap();
    services
        .register(ObjectFrameServiceHandle::new(Arc::new(Frames {
            snapshots: vec![SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()],
            origins: BTreeMap::from([
                (id("bo"), (from, true)),
                (id("i"), (from, true)),
                (id("o"), (to, exact)),
            ]),
        })))
        .unwrap();
    let project = Project::new(
        ["b", "p", "bo", "i", "o"]
            .into_iter()
            .map(|local| Object::new(id(local), "element"))
            .collect(),
    )
    .unwrap();
    (services, project)
}

fn fall(services: &ServiceRegistry, project: &Project) -> Result<(f64, f64), String> {
    common::measured_cited(services, project, &id("p"), "fall_in_flow")
        .map(|value| value.expect("a fall is a value").0)
}

#[test]
fn a_pipe_falls_from_its_inlet_to_its_outlet() {
    let source = Some(FlowDirection::Source);
    let (services, project) = run([0.0, 0.0, 1.0], [3.0, 4.0, 0.9], source, true);
    let measured = common::measured_cited(&services, &project, &id("p"), "fall_in_flow")
        .unwrap()
        .unwrap();
    let ((lower, upper), exact) = measured;
    assert!(
        lower <= 0.02 && 0.02 <= upper && upper - lower < 1e-15,
        "{measured:?}"
    );
    assert!(exact);
    // Laid against the flow: it rises 1 in 50, a negative fall.
    let (services, project) = run([0.0, 0.0, 0.9], [3.0, 4.0, 1.0], source, true);
    let (lower, upper) = fall(&services, &project).unwrap();
    assert!(lower <= -0.02 && -0.02 <= upper, "{lower}..{upper}");
}

#[test]
fn an_undecided_direction_or_plumb_run_has_no_fall() {
    let both = Some(FlowDirection::SourceAndSink);
    let (services, project) = run([0.0, 0.0, 1.0], [3.0, 4.0, 0.9], both, true);
    let why = fall(&services, &project).unwrap_err();
    assert!(why.contains("source-and-sink"), "{why}");
    let (services, project) = run([0.0, 0.0, 1.0], [3.0, 4.0, 0.9], None, true);
    let why = fall(&services, &project).unwrap_err();
    assert!(why.contains("states no flow direction"), "{why}");
    let source = Some(FlowDirection::Source);
    let (services, project) = run([0.0, 0.0, 1.0], [0.0, 0.0, 0.0], source, true);
    let why = fall(&services, &project).unwrap_err();
    assert!(why.contains("plumb"), "{why}");
    // Without a network there is nothing to measure.
    let (_, project) = run([0.0, 0.0, 1.0], [3.0, 4.0, 0.9], source, true);
    let why = fall(&ServiceRegistry::new(), &project).unwrap_err();
    assert!(why.contains("no distribution network service"), "{why}");
}

/// A port placement cited approximate is refused: no fall is ever measured
/// from an estimate.
#[test]
fn a_placement_cited_approximate_is_refused() {
    let source = Some(FlowDirection::Source);
    let (services, project) = run([0.0, 0.0, 1.0], [3.0, 4.0, 0.9], source, false);
    let why = fall(&services, &project).unwrap_err();
    assert!(why.contains("evidence is not exact"), "{why}");
}
