//! The fall of a run in flow direction, `fall_in_flow`: from the placement
//! of the port flow enters an element by to the one it leaves by, as the
//! source's distribution network states them.
//!
//! The direction is the network's: the element's one `sink` port is its
//! inlet and its one `source` port its outlet, and the element must hold
//! no connection of undecided direction and lie on no cycle of flow. The
//! fall is the inlet's elevation above the outlet's, per metre of plan
//! distance between them: positive where the run falls in flow direction,
//! negative where it is laid against it. The least fall along a path is an
//! aggregate's `min` of it over `axioval:derived.flow:forward+`.

use std::sync::Arc;

use axioval_engine::{
    FlowGraphs, MeasuredProvider, Measurement, NotEvaluatedReason, ObjectFrameServiceHandle,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::MeasuredCall;
use axioval_ir::{FlowDirection, ObjectId};

use crate::measured_kinds::refused;

/// Measures `fall_in_flow`.
pub(crate) struct FallMeasures;

const FALL_IN_FLOW: &str = "fall_in_flow";

impl MeasuredProvider for FallMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[FALL_IN_FLOW]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let refuse = refused(call.name(), object);
        let undecided = |why: String| refuse((NotEvaluatedReason::IncompleteEvidence, why));
        let graphs = context.services.get::<Arc<FlowGraphs>>().ok_or_else(|| {
            refuse((
                NotEvaluatedReason::MissingService,
                "no distribution network service is registered".into(),
            ))
        })?;
        let frames = context
            .services
            .get::<ObjectFrameServiceHandle>()
            .ok_or_else(|| {
                refuse((
                    NotEvaluatedReason::MissingService,
                    "object-frame service is not registered".into(),
                ))
            })?;
        let graph = graphs
            .graph(&object.source)
            .map_err(|why| refuse((NotEvaluatedReason::BackendUnavailable, why)))?;
        graph.settled(object).map_err(|why| {
            undecided(format!(
                "the flow direction at the element is undecided: {why}"
            ))
        })?;
        let (inlet, outlet) = ends(graph.ports(object)).map_err(undecided)?;
        let mut locators = Vec::new();
        let mut position = |port: &ObjectId| {
            let frame = frames
                .object_frame(port)
                .map_err(|error| undecided(format!("port {port}: {error}")))?;
            if !frame.evidence().exact {
                return Err(undecided(format!(
                    "port {port}'s placement is not stated exactly"
                )));
            }
            locators.push(frame.evidence().locator.clone());
            Ok(frame.frame().origin().coordinates_metres())
        };
        let (from, to) = (position(&inlet)?, position(&outlet)?);
        let run = (to[0] - from[0]).hypot(to[1] - from[1]);
        if run == 0.0 {
            return Err(undecided(format!(
                "its inlet {inlet} lies plumb above or below its outlet {outlet}, so it has no \
                 fall"
            )));
        }
        let fall = (from[2] - to[2]) / run;
        // Each of the five operations rounds by at most half an ulp of its
        // result; eight ulps of the quotient hold them all.
        let margin = fall.abs() * 8.0 * f64::EPSILON;
        let (lower, upper) = if fall == 0.0 {
            (0.0, 0.0)
        } else {
            ((fall - margin).next_down(), (fall + margin).next_up())
        };
        let mut graph_evidence = graph.evidence().iter().map(|item| item.locator.clone());
        let locator = std::iter::once(format!("{FALL_IN_FLOW}:{inlet}->{outlet}"))
            .chain(locators)
            .chain(graph_evidence.next())
            .collect::<Vec<_>>()
            .join(";");
        Ok(Measurement::Rounded {
            lower,
            upper,
            dimension: None,
            locator,
        })
    }
}

/// The one inlet and one outlet among `ports`, or why there is none.
fn ends(ports: &[(ObjectId, Option<FlowDirection>)]) -> Result<(ObjectId, ObjectId), String> {
    let of = |wanted: FlowDirection| -> Vec<&ObjectId> {
        ports
            .iter()
            .filter(|(_, flow)| *flow == Some(wanted))
            .map(|(port, _)| port)
            .collect()
    };
    let others: Vec<String> = ports
        .iter()
        .filter(|(_, flow)| !matches!(flow, Some(FlowDirection::Sink | FlowDirection::Source)))
        .map(|(port, flow)| match flow {
            Some(flow) => format!("{port} is {}", flow.name()),
            None => format!("{port} states no flow direction"),
        })
        .collect();
    match (
        of(FlowDirection::Sink).as_slice(),
        of(FlowDirection::Source).as_slice(),
    ) {
        ([inlet], [outlet]) if others.is_empty() => Ok(((*inlet).clone(), (*outlet).clone())),
        (inlets, outlets) => {
            let mut why = format!(
                "the element states {} inlet(s) and {} outlet(s), not one of each",
                inlets.len(),
                outlets.len()
            );
            if !others.is_empty() {
                why.push_str("; ");
                why.push_str(&others.join(", "));
            }
            Err(why)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("t", "m").unwrap(), local).unwrap()
    }

    #[test]
    fn a_run_has_one_inlet_and_one_outlet() {
        let sink = Some(FlowDirection::Sink);
        let source = Some(FlowDirection::Source);
        assert_eq!(
            ends(&[(id("a"), sink), (id("b"), source)]),
            Ok((id("a"), id("b")))
        );
        let why = ends(&[(id("a"), sink), (id("b"), sink), (id("c"), source)]).unwrap_err();
        assert!(why.contains("2 inlet(s) and 1 outlet(s)"), "{why}");
        let why = ends(&[
            (id("a"), sink),
            (id("b"), Some(FlowDirection::SourceAndSink)),
        ])
        .unwrap_err();
        assert!(why.contains("b is source-and-sink"), "{why}");
        assert!(ends(&[]).is_err());
    }
}
