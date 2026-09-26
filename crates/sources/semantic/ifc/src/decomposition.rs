//! The wholes an IFC object is part of, walked as IDS walks them.
//!
//! Each step reads one relationship type through the relationship service's
//! edge indexes, so ends come from the file's release schema and a malformed
//! instance refuses the answer. The walks:
//!
//! - aggregation and nesting follow `IfcRelAggregates` / `IfcRelNests`
//!   upwards, nearest whole first;
//! - containment is the direct `IfcRelContainedInSpatialStructure` only;
//! - grouping is the one `IfcRelAssignsToGroup` group;
//! - voiding is the element an opening voids, or, for an element filling an
//!   opening, the element that opening voids;
//! - any follows the nearest whole of any kind (container, aggregate, nest,
//!   filled opening, voided element, then group) upwards.
//!
//! Where a step finds two wholes of one kind, which one a requirement means
//! is not determined and the answer is refused as ambiguous.

use std::collections::BTreeSet;
use std::sync::Arc;

use axioval_engine::{
    Decomposition, DecompositionError, DecompositionService, ResolvedWholes, SourceSnapshot, Whole,
};
use axioval_ir::{Evidence, ObjectId};
use ifc_model::{EntityId, Model};

use crate::attributes::IfcAttributeService;
use crate::relationships::IfcRelationshipService;
use crate::release::Release;

const AGGREGATES: &str = "IfcRelAggregates";
const NESTS: &str = "IfcRelNests";
const CONTAINED: &str = "IfcRelContainedInSpatialStructure";
const GROUPS: &str = "IfcRelAssignsToGroup";
const VOIDS: &str = "IfcRelVoidsElement";
const FILLS: &str = "IfcRelFillsElement";

pub(crate) struct IfcDecompositionService {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    relationships: Arc<IfcRelationshipService>,
    attributes: Arc<IfcAttributeService>,
}

impl IfcDecompositionService {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        snapshots: Arc<[SourceSnapshot]>,
        relationships: Arc<IfcRelationshipService>,
        attributes: Arc<IfcAttributeService>,
    ) -> Self {
        Self {
            release,
            model,
            snapshots,
            relationships,
            attributes,
        }
    }

    /// The one whole of `part` through `relationship`, if any.
    fn one(
        &self,
        relationship: &str,
        part: EntityId,
    ) -> Result<Option<EntityId>, DecompositionError> {
        let wholes = self
            .relationships
            .relating_of(relationship, part)
            .map_err(|error| DecompositionError::Unreadable(error.to_string()))?;
        let unique: BTreeSet<EntityId> = wholes.iter().copied().collect();
        match unique.len() {
            0 => Ok(None),
            1 => Ok(unique.into_iter().next()),
            _ => Err(DecompositionError::Ambiguous(format!(
                "#{} has {} wholes through {relationship}",
                part.0,
                unique.len()
            ))),
        }
    }

    /// The nearest whole of any kind, in the order IDS tries them.
    fn parent(&self, part: EntityId) -> Result<Option<EntityId>, DecompositionError> {
        for relationship in [CONTAINED, AGGREGATES, NESTS, FILLS, VOIDS, GROUPS] {
            if let Some(whole) = self.one(relationship, part)? {
                return Ok(Some(whole));
            }
        }
        Ok(None)
    }

    /// Follows `step` upwards from `part`, refusing a cycle.
    fn chain(
        part: EntityId,
        step: impl Fn(EntityId) -> Result<Option<EntityId>, DecompositionError>,
    ) -> Result<Vec<EntityId>, DecompositionError> {
        let mut seen = BTreeSet::from([part]);
        let mut chain = Vec::new();
        let mut current = part;
        while let Some(whole) = step(current)? {
            if !seen.insert(whole) {
                return Err(DecompositionError::Unreadable(format!(
                    "#{} is its own whole through a cycle",
                    whole.0
                )));
            }
            chain.push(whole);
            current = whole;
        }
        Ok(chain)
    }

    fn whole(&self, id: EntityId) -> Result<Whole, DecompositionError> {
        let entity = self.model.get(id).ok_or_else(|| {
            DecompositionError::Unreadable(format!("#{} is not in the model", id.0))
        })?;
        let class = entity.type_name.to_ascii_uppercase();
        let (predefined_type, _) = self
            .attributes
            .predefined_of(id)
            .map_err(|error| DecompositionError::Unreadable(error.to_string()))?;
        let object = self
            .release
            .schema
            .is_a(&class, "IFCOBJECT")
            .then(|| ObjectId::new(self.snapshots[0].source().clone(), format!("#{}", id.0)))
            .transpose()
            .map_err(|error| DecompositionError::Unreadable(error.to_string()))?;
        Ok(Whole {
            class,
            predefined_type,
            object,
        })
    }
}

impl DecompositionService for IfcDecompositionService {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn wholes(
        &self,
        object: &ObjectId,
        decomposition: Decomposition,
    ) -> Result<ResolvedWholes, DecompositionError> {
        let id = object
            .local_id
            .strip_prefix('#')
            .and_then(|digits| digits.parse::<u64>().ok())
            .map(EntityId)
            .filter(|id| self.model.get(*id).is_some())
            .ok_or_else(|| DecompositionError::UnknownObject(object.clone()))?;
        let ids = match decomposition {
            Decomposition::Aggregation => Self::chain(id, |part| self.one(AGGREGATES, part))?,
            Decomposition::Nesting => Self::chain(id, |part| self.one(NESTS, part))?,
            Decomposition::Containment => self.one(CONTAINED, id)?.into_iter().collect(),
            Decomposition::Grouping => self.one(GROUPS, id)?.into_iter().collect(),
            Decomposition::Voiding => {
                let opening = if self.release.schema.is_a(
                    &self
                        .model
                        .get(id)
                        .map(|e| e.type_name.to_ascii_uppercase())
                        .unwrap_or_default(),
                    "IFCOPENINGELEMENT",
                ) {
                    Some(id)
                } else {
                    self.one(FILLS, id)?
                };
                match opening {
                    Some(opening) => self.one(VOIDS, opening)?.into_iter().collect(),
                    None => Vec::new(),
                }
            }
            Decomposition::Any => Self::chain(id, |part| self.parent(part))?,
        };
        let wholes = ids
            .into_iter()
            .map(|whole| self.whole(whole))
            .collect::<Result<_, _>>()?;
        Ok(ResolvedWholes {
            wholes,
            evidence: Evidence::exact(
                self.snapshots[0].source().clone(),
                format!(
                    "ifc:{}:wholes:{:?}:#{}",
                    self.snapshots[0].fingerprint(),
                    decomposition,
                    id.0
                ),
            ),
        })
    }
}
