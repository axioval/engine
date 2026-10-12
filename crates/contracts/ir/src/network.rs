//! Distribution networks as a source states them: the ports of its
//! distribution elements, which element each belongs to, the direction
//! flow takes through it, and which ports are connected.
//!
//! A source adapter maps its own connection structures onto these types
//! (for IFC, `IfcDistributionPort`s nested in or connected to their
//! elements and joined by `IfcRelConnectsPorts`); the engine builds the
//! connectivity graph of elements from them and never sees the source's
//! types. Nothing here decides a direction the source leaves open: a port
//! without a stated direction is `None`, never a guess.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Evidence, ObjectId, SourceId};

/// The direction flow takes through a port, relative to its element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlowDirection {
    /// Flow leaves the element through the port (an outlet).
    Source,
    /// Flow enters the element through the port (an inlet).
    Sink,
    /// Flow may pass the port either way.
    SourceAndSink,
    /// The source states a direction it does not define.
    NotDefined,
}

impl FlowDirection {
    /// The direction's name as reports show it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Sink => "sink",
            Self::SourceAndSink => "source-and-sink",
            Self::NotDefined => "not-defined",
        }
    }
}

/// One port of a distribution network.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkPort {
    /// The port's own identity.
    pub port: ObjectId,
    /// Every element the source states the port belongs to, sorted: one for
    /// a well-formed port, none or several where the source leaves it
    /// unattached or attaches it twice.
    pub elements: Vec<ObjectId>,
    /// The direction flow takes through it; `None` where the source states
    /// none.
    pub flow: Option<FlowDirection>,
    /// Where the source states the port and its attachments.
    pub evidence: Vec<Evidence>,
}

/// Two connected ports, and the element realizing the connection, if any
/// (a fitting the source names on the connection itself).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortConnection {
    pub relating: ObjectId,
    pub related: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realizing: Option<ObjectId>,
    pub evidence: Evidence,
}

/// Every port and connection one source states, complete or not given.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistributionNetwork {
    source: SourceId,
    ports: Vec<NetworkPort>,
    connections: Vec<PortConnection>,
    evidence: Vec<Evidence>,
}

/// Why a network listing is no complete, exact statement of one source.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NetworkError {
    #[error("port `{0}` is listed twice")]
    DuplicatePort(Box<ObjectId>),
    #[error("a connection names `{0}`, which is no listed port")]
    UnknownPort(Box<ObjectId>),
    #[error("`{0}` is not of the network's source")]
    ForeignObject(Box<ObjectId>),
    #[error("network evidence is missing, inexact or blank")]
    InexactEvidence,
}

impl DistributionNetwork {
    /// A source's network, its ports sorted by identity and its connections
    /// in the order of their ports.
    ///
    /// # Errors
    ///
    /// A port listed twice, a connection naming an unlisted port, an object
    /// of another source, and missing, inexact or blank evidence (the
    /// completeness proof included) are refused.
    pub fn try_new(
        source: SourceId,
        mut ports: Vec<NetworkPort>,
        mut connections: Vec<PortConnection>,
        evidence: Vec<Evidence>,
    ) -> Result<Self, NetworkError> {
        let exact = |items: &[Evidence]| {
            !items.is_empty()
                && items
                    .iter()
                    .all(|item| item.exact && !item.locator.trim().is_empty())
        };
        if !exact(&evidence) {
            return Err(NetworkError::InexactEvidence);
        }
        ports.sort_by(|left, right| left.port.cmp(&right.port));
        let mut listed = BTreeSet::new();
        for port in &mut ports {
            port.elements.sort();
            port.elements.dedup();
            if !listed.insert(port.port.clone()) {
                return Err(NetworkError::DuplicatePort(Box::new(port.port.clone())));
            }
            for object in std::iter::once(&port.port).chain(&port.elements) {
                if object.source != source {
                    return Err(NetworkError::ForeignObject(Box::new(object.clone())));
                }
            }
            if !exact(&port.evidence) {
                return Err(NetworkError::InexactEvidence);
            }
        }
        for connection in &connections {
            for port in [&connection.relating, &connection.related] {
                if !listed.contains(port) {
                    return Err(NetworkError::UnknownPort(Box::new(port.clone())));
                }
            }
            if let Some(realizing) = &connection.realizing
                && realizing.source != source
            {
                return Err(NetworkError::ForeignObject(Box::new(realizing.clone())));
            }
            if !exact(std::slice::from_ref(&connection.evidence)) {
                return Err(NetworkError::InexactEvidence);
            }
        }
        connections.sort_by(|left, right| {
            (&left.relating, &left.related, &left.evidence.locator).cmp(&(
                &right.relating,
                &right.related,
                &right.evidence.locator,
            ))
        });
        Ok(Self {
            source,
            ports,
            connections,
            evidence,
        })
    }

    /// The source the network belongs to.
    #[must_use]
    pub fn source(&self) -> &SourceId {
        &self.source
    }

    /// Every port, sorted by identity.
    #[must_use]
    pub fn ports(&self) -> &[NetworkPort] {
        &self.ports
    }

    /// Every connection, in the order of their ports.
    #[must_use]
    pub fn connections(&self) -> &[PortConnection] {
        &self.connections
    }

    /// The proof that every port and connection of the source is listed.
    #[must_use]
    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("t", "m").unwrap(), local).unwrap()
    }

    fn proof(locator: &str) -> Evidence {
        Evidence::exact(SourceId::new("t", "m").unwrap(), locator)
    }

    fn port(local: &str, element: &str) -> NetworkPort {
        NetworkPort {
            port: id(local),
            elements: vec![id(element)],
            flow: Some(FlowDirection::Source),
            evidence: vec![proof(local)],
        }
    }

    #[test]
    fn a_network_is_sorted_and_refuses_what_it_cannot_state() {
        let source = SourceId::new("t", "m").unwrap();
        let connection = PortConnection {
            relating: id("p2"),
            related: id("p1"),
            realizing: None,
            evidence: proof("c"),
        };
        let network = DistributionNetwork::try_new(
            source.clone(),
            vec![port("p2", "b"), port("p1", "a")],
            vec![connection.clone()],
            vec![proof("scan")],
        )
        .unwrap();
        assert_eq!(network.ports()[0].port, id("p1"));
        assert_eq!(
            DistributionNetwork::try_new(
                source.clone(),
                vec![port("p1", "a"), port("p1", "a")],
                Vec::new(),
                vec![proof("scan")],
            ),
            Err(NetworkError::DuplicatePort(Box::new(id("p1"))))
        );
        assert_eq!(
            DistributionNetwork::try_new(
                source.clone(),
                vec![port("p1", "a")],
                vec![connection],
                vec![proof("scan")],
            ),
            Err(NetworkError::UnknownPort(Box::new(id("p2"))))
        );
        assert_eq!(
            DistributionNetwork::try_new(source, Vec::new(), Vec::new(), Vec::new()),
            Err(NetworkError::InexactEvidence)
        );
    }
}
