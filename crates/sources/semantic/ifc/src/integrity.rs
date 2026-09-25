//! Integrity issues for one IFC model.
//!
//! Relationship ends are scanned with the same reader as relationship
//! selection, so a warning here and a refusal there always describe the same
//! instances. Spatial and zone cardinality violations come from `ifc-systems`,
//! which reads them against the file's own release; only anomalies that are
//! pure schema violations are taken from it, because dangling references are
//! already reported by the relationship scan and would otherwise appear twice.

use std::sync::Arc;

use crate::relationships::{EdgeIndex, ends_of, read_instance};
use crate::release::Release;
use axioval_engine::{
    IntegrityError, IntegrityIssue, IntegritySeverity, SourceIntegrityService, SourceSnapshot,
};
use axioval_ir::{Evidence, SourceId};
use ifc_model::Model;
use ifc_systems::{SystemAnomaly, spatial_placements, zones};

/// Code for a relationship instance that omits an end its schema requires.
pub const ABSENT_REQUIRED_END: &str = "relationship.absent-required-end";
/// Code for a relationship instance whose end is not a resolvable reference.
pub const MALFORMED_RELATIONSHIP: &str = "relationship.malformed";
/// Code for an element contained by two spatial structures.
///
/// `ContainedInStructure` is `SET [0:1]`; the file states two homes that
/// cannot both be true. Relationship selection still reports both, as the
/// file does, so this warning is the only place the conflict surfaces.
pub const CONTAINED_TWICE: &str = "spatial.contained-twice";
/// Code for an `IfcZone` member that the zone's WR1 rule does not permit.
pub const ZONE_MEMBER_NOT_SPATIAL: &str = "zone.member-not-spatial";

pub(crate) struct IfcIntegrity {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
}

impl IfcIntegrity {
    pub(crate) fn new(
        release: Release,
        model: Arc<Model>,
        snapshots: Arc<[SourceSnapshot]>,
    ) -> Self {
        Self {
            release,
            model,
            snapshots,
        }
    }
}

impl SourceIntegrityService for IfcIntegrity {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn issues(&self, source: &SourceId) -> Result<Vec<IntegrityIssue>, IntegrityError> {
        let snapshot = &self.snapshots[0];
        if snapshot.source() != source {
            return Err(IntegrityError::UncoveredSource(source.clone()));
        }
        let locator = |detail: String| {
            Evidence::exact(
                source.clone(),
                format!("ifc:{}:{detail}", snapshot.fingerprint()),
            )
        };
        let schema = self.release.schema;
        let mut types: Vec<&str> = schema.subtypes("IfcRelationship");
        types.sort_unstable();
        types.dedup();
        let mut issues = Vec::new();
        for type_name in types {
            let instances = self.model.ids_of_type(type_name);
            if instances.is_empty() {
                continue;
            }
            // A type whose ends are not plain object references is not
            // readable as edges at all; that is a property of the schema,
            // not an irregularity of this file, so it is not reported.
            let Ok(ends) = ends_of(schema, type_name) else {
                continue;
            };
            for id in instances {
                let mut index = EdgeIndex::default();
                if let Err(error) = read_instance(&self.model, *id, &ends, &mut index) {
                    issues.push(IntegrityIssue {
                        code: MALFORMED_RELATIONSHIP.into(),
                        severity: IntegritySeverity::Error,
                        message: error.to_string(),
                        evidence: locator(format!("relationship:{id}")),
                    });
                }
                issues.extend(index.absent.into_iter().map(|absent| IntegrityIssue {
                    code: ABSENT_REQUIRED_END.into(),
                    severity: IntegritySeverity::Warning,
                    message: format!(
                        "{} {} has no `{}`, which {} requires; it contributes no edge \
                         through that end",
                        absent.type_name, absent.instance, absent.attribute, self.release.label
                    ),
                    evidence: locator(format!(
                        "relationship-absent-end:{}:{}",
                        absent.instance, absent.attribute
                    )),
                }));
            }
        }
        issues.extend(self.schema_violations(&locator));
        Ok(issues)
    }
}

impl IfcIntegrity {
    /// Cardinality and membership violations stated by the file.
    fn schema_violations(&self, locator: &impl Fn(String) -> Evidence) -> Vec<IntegrityIssue> {
        let (_, placement_anomalies) = spatial_placements(&self.model);
        let (_, zone_anomalies) = zones(&self.model);
        placement_anomalies
            .into_iter()
            .chain(zone_anomalies)
            .filter_map(|anomaly| match anomaly {
                SystemAnomaly::ContainedTwice {
                    element,
                    first,
                    second,
                } => Some(IntegrityIssue {
                    code: CONTAINED_TWICE.into(),
                    severity: IntegritySeverity::Warning,
                    message: format!(
                        "{element} is contained by both {first} and {second}; an element has \
                         at most one containing spatial structure"
                    ),
                    evidence: locator(format!(
                        "spatial-contained-twice:{element}:{first}:{second}"
                    )),
                }),
                SystemAnomaly::ZoneMemberNotSpatial {
                    relation,
                    zone,
                    member,
                    type_name,
                } => Some(IntegrityIssue {
                    code: ZONE_MEMBER_NOT_SPATIAL.into(),
                    severity: IntegritySeverity::Warning,
                    message: format!(
                        "{relation} assigns {member} ({type_name}) to zone {zone}; a zone may \
                         only group zones, spaces and spatial zones"
                    ),
                    evidence: locator(format!("zone-member:{relation}:{member}")),
                }),
                // Dangling references are the relationship scan's to report;
                // the rest describe systems and ports, not source integrity.
                _ => None,
            })
            .collect()
    }
}
