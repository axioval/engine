//! The distribution network an IFC model states through its ports, read by
//! the session's network service and joined into the engine's flow graph.
#![allow(missing_docs)]

use std::collections::BTreeSet;

use axioval_engine::{
    DistributionNetworkServiceHandle, EvidenceSession, FlowGraph, NetworkServiceError,
    TraversalDirection,
};
use axioval_ifc::import_ifc_session;
use axioval_ir::{FlowDirection, ObjectId, SourceId};

fn step(schema: &str, data: &str) -> Vec<u8> {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('{schema}'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
    .into_bytes()
}

fn session(schema: &str, data: &str) -> EvidenceSession {
    import_ifc_session("model.ifc", &step(schema, data)).unwrap()
}

fn source() -> SourceId {
    SourceId::new("ifc-step", "model.ifc").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn ids(locals: &[&str]) -> BTreeSet<ObjectId> {
    locals.iter().map(|local| id(local)).collect()
}

/// WC #10 and basin #11 drain into pipe #20, which drains into manhole #30;
/// the basin's outlet #111 and pipe inlet #201 are joined through a
/// fitting #40. Shower #12's outlet #120 is connected to nothing.
const DRAINAGE: &str = "\
#1=IFCPROJECT('p',$,'P',$,$,$,$,$,$);
#10=IFCSANITARYTERMINAL('t1',$,'WC',$,$,$,$,$,.TOILETPAN.);
#11=IFCSANITARYTERMINAL('t2',$,'Basin',$,$,$,$,$,.WASHHANDBASIN.);
#12=IFCSANITARYTERMINAL('t3',$,'Shower',$,$,$,$,$,.SHOWER.);
#20=IFCPIPESEGMENT('s1',$,'Pipe',$,$,$,$,$,.RIGIDSEGMENT.);
#30=IFCDISTRIBUTIONCHAMBERELEMENT('m1',$,'Manhole',$,$,$,$,$,.MANHOLE.);
#40=IFCPIPEFITTING('f1',$,'Bend',$,$,$,$,$,.BEND.);
#100=IFCDISTRIBUTIONPORT('q1',$,$,$,$,$,$,.SOURCE.,.PIPE.,.DRAINAGE.);
#111=IFCDISTRIBUTIONPORT('q2',$,$,$,$,$,$,.SOURCE.,.PIPE.,.DRAINAGE.);
#120=IFCDISTRIBUTIONPORT('q3',$,$,$,$,$,$,.SOURCE.,.PIPE.,.DRAINAGE.);
#200=IFCDISTRIBUTIONPORT('q4',$,$,$,$,$,$,.SINK.,.PIPE.,.DRAINAGE.);
#201=IFCDISTRIBUTIONPORT('q5',$,$,$,$,$,$,.SINK.,.PIPE.,.DRAINAGE.);
#202=IFCDISTRIBUTIONPORT('q6',$,$,$,$,$,$,.SOURCE.,.PIPE.,.DRAINAGE.);
#300=IFCDISTRIBUTIONPORT('q7',$,$,$,$,$,$,.SINK.,.PIPE.,.DRAINAGE.);
#500=IFCRELNESTS('n1',$,$,$,#10,(#100));
#501=IFCRELNESTS('n2',$,$,$,#11,(#111));
#502=IFCRELNESTS('n3',$,$,$,#12,(#120));
#503=IFCRELNESTS('n4',$,$,$,#20,(#200,#201,#202));
#504=IFCRELNESTS('n5',$,$,$,#30,(#300));
#600=IFCRELCONNECTSPORTS('c1',$,$,$,#100,#200,$);
#601=IFCRELCONNECTSPORTS('c2',$,$,$,#111,#201,#40);
#602=IFCRELCONNECTSPORTS('c3',$,$,$,#202,#300,$);
";

fn network(
    session: &EvidenceSession,
) -> Result<axioval_ir::DistributionNetwork, NetworkServiceError> {
    session
        .service::<DistributionNetworkServiceHandle>()
        .expect("the IFC session registers a network service")
        .network(&source())
}

#[test]
fn ports_nested_in_elements_are_read_with_their_flow_and_connections() {
    let network = network(&session("IFC4", DRAINAGE)).unwrap();
    let port = |local: &str| {
        network
            .ports()
            .iter()
            .find(|port| port.port == id(local))
            .unwrap()
    };
    assert_eq!(network.ports().len(), 7);
    assert_eq!(port("#201").elements, [id("#20")]);
    assert_eq!(port("#201").flow, Some(FlowDirection::Sink));
    assert_eq!(port("#120").flow, Some(FlowDirection::Source));
    assert_eq!(network.connections().len(), 3);
    let fitted = network
        .connections()
        .iter()
        .find(|connection| connection.relating == id("#111"))
        .unwrap();
    assert_eq!(fitted.realizing, Some(id("#40")));
    assert!(
        network.evidence()[0].locator.contains(
            "network-scan:ports=7:IfcRelNests=5,IfcRelConnectsPortToElement=0,IfcRelConnectsPorts=3"
        ),
        "{:?}",
        network.evidence()
    );

    let graph = FlowGraph::build(&network);
    let (upstream, _) = graph
        .reach(&id("#20"), TraversalDirection::Backward, true)
        .unwrap();
    assert_eq!(upstream, ids(&["#10", "#11", "#40"]));
    let (downstream, _) = graph
        .reach(&id("#11"), TraversalDirection::Forward, true)
        .unwrap();
    assert_eq!(downstream, ids(&["#20", "#30", "#40"]));
    let (nowhere, evidence) = graph
        .reach(&id("#12"), TraversalDirection::Forward, true)
        .unwrap();
    assert!(nowhere.is_empty());
    assert!(
        evidence
            .iter()
            .any(|item| item.locator.ends_with("open-end:ifc-step:model.ifc/#120")),
        "{evidence:?}"
    );
}

#[test]
fn ifc2x3_ports_are_attached_by_connection_to_their_element() {
    let data = "\
#1=IFCPROJECT('p',$,'P',$,$,$,$,$,$);
#10=IFCFLOWTERMINAL('t1',$,'WC',$,$,$,$,$);
#20=IFCFLOWSEGMENT('s1',$,'Pipe',$,$,$,$,$);
#100=IFCDISTRIBUTIONPORT('q1',$,$,$,$,$,$,.SOURCE.);
#200=IFCDISTRIBUTIONPORT('q2',$,$,$,$,$,$,$);
#500=IFCRELCONNECTSPORTTOELEMENT('a1',$,$,$,#100,#10);
#501=IFCRELCONNECTSPORTTOELEMENT('a2',$,$,$,#200,#20);
#600=IFCRELCONNECTSPORTS('c1',$,$,$,#100,#200,$);
";
    let network = network(&session("IFC2X3", data)).unwrap();
    assert_eq!(network.ports()[0].elements, [id("#10")]);
    assert_eq!(network.ports()[1].flow, None);
    let graph = FlowGraph::build(&network);
    // The pipe's port states no direction: upstream is undecided, the
    // connection itself is not.
    let why = graph
        .reach(&id("#20"), TraversalDirection::Backward, true)
        .unwrap_err();
    assert!(why.contains("#200 states no flow direction"), "{why}");
    let (connected, _) = graph
        .reach(&id("#20"), TraversalDirection::Either, true)
        .unwrap();
    assert_eq!(connected, ids(&["#10"]));
}

#[test]
fn a_connection_without_its_related_port_refuses_the_network() {
    let data = DRAINAGE.replace("#100,#200,$", "#100,$,$");
    let error = network(&session("IFC4", &data)).unwrap_err();
    assert!(
        matches!(&error, NetworkServiceError::Unavailable(why) if why.contains("RelatedPort")),
        "{error}"
    );
}

#[test]
fn a_model_without_ports_states_an_empty_network() {
    let network = network(&session("IFC4", "#1=IFCPROJECT('p',$,'P',$,$,$,$,$,$);\n")).unwrap();
    assert!(network.ports().is_empty() && network.connections().is_empty());
    assert!(!network.evidence().is_empty());
}
