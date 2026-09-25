//! Integrity warnings for one IFC model's objectified relationships.
//!
//! This scan uses the same end reader as relationship selection, so a warning
//! here and a refusal there always describe the same instances.

use std::sync::Arc;

use crate::relationships::{EdgeIndex, ends_of, read_instance};
use crate::release::Release;
use axioval_engine::{
    IntegrityError, IntegrityIssue, IntegritySeverity, SourceIntegrityService, SourceSnapshot,
};
use axioval_ir::{Evidence, SourceId};
use ifc_model::Model;

/// Code for a relationship instance that omits an end its schema requires.
pub const ABSENT_REQUIRED_END: &str = "relationship.absent-required-end";
/// Code for a relationship instance whose end is not a resolvable reference.
pub const MALFORMED_RELATIONSHIP: &str = "relationship.malformed";

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
        Ok(issues)
    }
}
