//! Vendor-neutral entity-relation and per-source cardinality plans.

use serde::{Deserialize, Serialize};

use crate::rule::assertion::Severity;
use crate::rule::execution::ElementScopeSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityRelationSpec {
    /// Every direct `IfcRelAggregates` whole/child edge.
    Aggregation,
    /// `IfcRelContainedInSpatialStructure`.
    SpatialContainment,
    /// `IfcRelSpaceBoundary`.
    SpaceBoundary,
    /// `IfcRelAssignsToGroup`, optionally traversing nested groups.
    GroupMembership { recursive: bool },
    /// Element-only decomposition corresponding to native decomposition links.
    Decomposition,
    /// `IfcRelVoidsElement` host/opening.
    OpeningVoid,
    /// `IfcRelFillsElement` opening/filler.
    OpeningFill,
    /// Provider-derived nearest-space relation. This cannot be inferred from
    /// IFC relationship records and must be supplied by the model provider.
    NearestSpaces,
    /// Provider spatial-reference relation (`SContainsReferenced` in the provider).
    SpatialReference,
    /// Exact provider-derived geometry containment (`SContains` / nearest-space
    /// geometry mode). This is never synthesized from envelopes or IFC props.
    ExactSpaceContainment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationDirection {
    Forward,
    Reverse,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationSourceDomain {
    #[default]
    Selection,
    ProviderRelationKeys,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationCardinalitySpec {
    pub minimum: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationResultPolicy {
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationPlanSpec {
    #[serde(default)]
    pub source_domain: RelationSourceDomain,
    pub sources: ElementScopeSpec,
    pub source_fallback_type: String,
    pub targets: ElementScopeSpec,
    pub target_fallback_type: String,
    pub relation: EntityRelationSpec,
    pub direction: RelationDirection,
    pub cardinality: RelationCardinalitySpec,
    pub result: RelationResultPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceComponentCountRowSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_classification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_classification: Option<String>,
    pub cardinality: RelationCardinalitySpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceComponentCountPlanSpec {
    pub space_classification_scheme: String,
    pub component_classification_scheme: String,
    pub rows: Vec<SpaceComponentCountRowSpec>,
    pub unavailable_severity: Severity,
}

impl SpaceComponentCountPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.space_classification_scheme.trim().is_empty()
            || self.component_classification_scheme.trim().is_empty()
        {
            return Err("space-component count classification scheme is blank".into());
        }
        if self.rows.is_empty() {
            return Err("space-component count plan has no rows".into());
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row
                .cardinality
                .maximum
                .is_some_and(|max| max < row.cardinality.minimum)
            {
                return Err(format!(
                    "space-component count row {index} maximum is below minimum"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionCardinalityPlanSpec {
    pub selection: ElementScopeSpec,
    pub fallback_type: String,
    pub cardinality: RelationCardinalitySpec,
    pub result: RelationResultPolicy,
}

pub fn component_related_to_space_plan(severity: Severity) -> RelationPlanSpec {
    component_to_space_plan(severity, EntityRelationSpec::SpatialReference)
}

pub fn component_contained_in_space_plan(severity: Severity) -> RelationPlanSpec {
    component_to_space_plan(severity, EntityRelationSpec::ExactSpaceContainment)
}

fn component_to_space_plan(severity: Severity, relation: EntityRelationSpec) -> RelationPlanSpec {
    RelationPlanSpec {
        source_domain: RelationSourceDomain::ProviderRelationKeys,
        sources: ElementScopeSpec {
            candidate_types: vec!["IFCPRODUCT".into()],
            ..Default::default()
        },
        source_fallback_type: "IFCPRODUCT".into(),
        targets: ElementScopeSpec {
            candidate_types: vec!["IFCSPACE".into()],
            ..Default::default()
        },
        target_fallback_type: "IFCSPACE".into(),
        relation,
        direction: RelationDirection::Reverse,
        cardinality: RelationCardinalitySpec {
            minimum: 1,
            maximum: None,
        },
        result: RelationResultPolicy {
            severity,
            unavailable_severity: Severity::Warning,
        },
    }
}

impl RelationPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.source_fallback_type.trim().is_empty() {
            return Err("relation source fallback type is blank".into());
        }
        if self.target_fallback_type.trim().is_empty() {
            return Err("relation target fallback type is blank".into());
        }
        if self.cardinality.minimum == 0 && self.cardinality.maximum.is_none() {
            return Err("relation cardinality is inert".into());
        }
        if self
            .cardinality
            .maximum
            .is_some_and(|maximum| maximum < self.cardinality.minimum)
        {
            return Err("relation cardinality maximum is below minimum".into());
        }
        Ok(())
    }
}

impl SelectionCardinalityPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.fallback_type.trim().is_empty() {
            return Err("selection fallback type is blank".into());
        }
        if self.cardinality.minimum == 0 && self.cardinality.maximum.is_none() {
            return Err("selection cardinality is inert".into());
        }
        if self
            .cardinality
            .maximum
            .is_some_and(|maximum| maximum < self.cardinality.minimum)
        {
            return Err("selection cardinality maximum is below minimum".into());
        }
        Ok(())
    }
}
