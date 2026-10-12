//! The distribution network of one IFC model: every port (`IfcPort` and
//! its subtypes), the element each belongs to, the flow direction an
//! `IfcDistributionPort` states, and the ports `IfcRelConnectsPorts` joins.
//!
//! A port belongs to the element nesting it (`IfcRelNests`, IFC4 and
//! IFC4X3) or the element it is connected to (`IfcRelConnectsPortToElement`,
//! the IFC2X3 form, still read where a later release declares it). Ends
//! are read through the relationship service's own `build_index`, so the
//! slots come from the release's schema and every refusal is the
//! relationship reader's. A relationship instance omitting a required end
//! refuses the network: the missing edge could be any port's. A flow
//! direction other than the four the schema enumerates is refused, never
//! read as none; an unset one is none.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axioval_engine::{DistributionNetworkService, NetworkServiceError, SourceSnapshot};
use axioval_ir::{
    DistributionNetwork, Evidence, FlowDirection, NetworkPort, ObjectId, PortConnection, SourceId,
};
use ifc_model::{EntityId, Model, Value};

use crate::relationships::{EdgeIndex, build_index};
use crate::release::Release;

pub(crate) struct IfcNetworkService {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    network: Mutex<Option<Result<DistributionNetwork, NetworkServiceError>>>,
}

impl IfcNetworkService {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        snapshots: Arc<[SourceSnapshot]>,
    ) -> Self {
        Self {
            release,
            model,
            snapshots,
            network: Mutex::new(None),
        }
    }

    fn source(&self) -> &SourceId {
        self.snapshots[0].source()
    }

    fn locator(&self, detail: impl std::fmt::Display) -> String {
        format!("ifc:{}:{detail}", self.snapshots[0].fingerprint())
    }

    fn object(&self, id: EntityId) -> Result<ObjectId, NetworkServiceError> {
        ObjectId::new(self.source().clone(), id.to_string())
            .map_err(|error| NetworkServiceError::Unavailable(error.to_string()))
    }

    /// Every edge of `relationship`, or none where the release does not
    /// declare it.
    fn edges(&self, relationship: &str) -> Result<Option<EdgeIndex>, NetworkServiceError> {
        if self.release.schema.entity(relationship).is_none() {
            return Ok(None);
        }
        let index = build_index(self.release, &self.model, relationship)
            .map_err(|error| NetworkServiceError::Unavailable(error.to_string()))?;
        if let Some(absent) = index.absent.first() {
            return Err(NetworkServiceError::Unavailable(format!(
                "{} {} has no `{}`, so a port's connection is unknown",
                absent.type_name, absent.instance, absent.attribute
            )));
        }
        Ok(Some(index))
    }

    /// The flow direction `port` states through its `FlowDirection`.
    fn flow(&self, port: EntityId) -> Result<Option<FlowDirection>, NetworkServiceError> {
        let entity = self.model.get(port).ok_or_else(|| {
            NetworkServiceError::Unavailable(format!("port {port} is not in the model"))
        })?;
        let Some(slot) = self
            .release
            .schema
            .attribute_names(&entity.type_name)
            .iter()
            .position(|name| name.eq_ignore_ascii_case("FlowDirection"))
        else {
            return Ok(None);
        };
        match entity.attribute(slot) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Enum(item)) => match item.to_ascii_uppercase().as_str() {
                "SOURCE" => Ok(Some(FlowDirection::Source)),
                "SINK" => Ok(Some(FlowDirection::Sink)),
                "SOURCEANDSINK" => Ok(Some(FlowDirection::SourceAndSink)),
                "NOTDEFINED" => Ok(Some(FlowDirection::NotDefined)),
                other => Err(NetworkServiceError::Unavailable(format!(
                    "port {port} states the flow direction `.{other}.`, which IFC does not define"
                ))),
            },
            Some(_) => Err(NetworkServiceError::Unavailable(format!(
                "port {port}'s flow direction is no enumeration value"
            ))),
        }
    }

    fn read(&self) -> Result<DistributionNetwork, NetworkServiceError> {
        let schema = self.release.schema;
        let source = self.source().clone();
        let mut port_types: Vec<&str> = schema.subtypes("IfcPort");
        port_types.push("IfcPort");
        port_types.sort_unstable();
        port_types.dedup();
        let mut ports: BTreeMap<EntityId, (Vec<ObjectId>, Vec<Evidence>)> = BTreeMap::new();
        for type_name in port_types {
            for id in self.model.ids_of_type(type_name) {
                ports.entry(*id).or_insert_with(|| {
                    (
                        Vec::new(),
                        vec![Evidence::exact(
                            source.clone(),
                            self.locator(format_args!("port:{id}")),
                        )],
                    )
                });
            }
        }
        let cite = |instance: EntityId| {
            Evidence::exact(
                source.clone(),
                self.locator(format_args!("relationship:{instance}")),
            )
        };
        let mut scanned = Vec::new();
        if let Some(nests) = self.edges("IfcRelNests")? {
            for (element, nested) in &nests.forward {
                if ports.contains_key(element) {
                    continue;
                }
                for (port, instance) in nested {
                    if let Some((elements, evidence)) = ports.get_mut(port) {
                        elements.push(self.object(*element)?);
                        evidence.push(cite(*instance));
                    }
                }
            }
            scanned.push((nests.type_name.clone(), nests.instances));
        }
        if let Some(attached) = self.edges("IfcRelConnectsPortToElement")? {
            for (port, related) in &attached.forward {
                let Some((elements, evidence)) = ports.get_mut(port) else {
                    continue;
                };
                for (element, instance) in related {
                    elements.push(self.object(*element)?);
                    evidence.push(cite(*instance));
                }
            }
            scanned.push((attached.type_name.clone(), attached.instances));
        }
        let mut connections = Vec::new();
        if let Some(connected) = self.edges("IfcRelConnectsPorts")? {
            for (relating, related) in &connected.forward {
                for (other, instance) in related {
                    if !ports.contains_key(relating) || !ports.contains_key(other) {
                        return Err(NetworkServiceError::Unavailable(format!(
                            "IfcRelConnectsPorts {instance} connects {relating} and {other}, \
                             which are not both ports"
                        )));
                    }
                    connections.push(PortConnection {
                        relating: self.object(*relating)?,
                        related: self.object(*other)?,
                        realizing: self.realizing(*instance)?,
                        evidence: cite(*instance),
                    });
                }
            }
            scanned.push((connected.type_name.clone(), connected.instances));
        }
        let ports = ports
            .into_iter()
            .map(|(id, (elements, evidence))| {
                Ok(NetworkPort {
                    port: self.object(id)?,
                    elements,
                    flow: self.flow(id)?,
                    evidence,
                })
            })
            .collect::<Result<Vec<_>, NetworkServiceError>>()?;
        let scan = scanned
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(",");
        let evidence = vec![Evidence::exact(
            source.clone(),
            self.locator(format_args!("network-scan:ports={}:{scan}", ports.len())),
        )];
        Ok(DistributionNetwork::try_new(
            source,
            ports,
            connections,
            evidence,
        )?)
    }

    /// The element an `IfcRelConnectsPorts` names as realizing it.
    fn realizing(&self, instance: EntityId) -> Result<Option<ObjectId>, NetworkServiceError> {
        let entity = self.model.get(instance).ok_or_else(|| {
            NetworkServiceError::Unavailable(format!("relationship {instance} is not in the model"))
        })?;
        let slot = self
            .release
            .schema
            .attribute_names(&entity.type_name)
            .iter()
            .position(|name| name.eq_ignore_ascii_case("RealizingElement"));
        match slot.and_then(|slot| entity.attribute(slot)) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Ref(element)) if self.model.get(*element).is_some() => {
                self.object(*element).map(Some)
            }
            Some(_) => Err(NetworkServiceError::Unavailable(format!(
                "IfcRelConnectsPorts {instance} names a realizing element that is not in the model"
            ))),
        }
    }
}

impl DistributionNetworkService for IfcNetworkService {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn network(&self, source: &SourceId) -> Result<DistributionNetwork, NetworkServiceError> {
        if source != self.source() {
            return Err(NetworkServiceError::UncoveredSource(source.clone()));
        }
        let mut network = self
            .network
            .lock()
            .map_err(|_| NetworkServiceError::Unavailable("network lock is poisoned".into()))?;
        network.get_or_insert_with(|| self.read()).clone()
    }
}
