use std::sync::Arc;

use axioval_engine::{
    AttributeServiceHandle, ClassificationServiceHandle, CompletePropertyAbsenceEvidence,
    DecompositionServiceHandle, EvidenceSession, EvidenceSessionError, MaterialServiceHandle,
    PropertyRequest, PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, RelationshipSelectionServiceHandle, ResolvedProperty,
    SourceIntegrityServiceHandle, SourceSnapshot, TypeHierarchyError, TypeHierarchyService,
    TypeHierarchyServiceHandle,
};
use axioval_ir::{
    Evidence, ExternalId, IrError, Object, ObjectId, Project, Property, PropertyValue, SourceId,
};
use ifc_model::{Codec, EntityId, Model};
use ifc_properties::{
    ExactPropertyError, ExactResolution, ExactSource, ExactValue, exact_property,
};
use ifc_step::StepCodec;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::attributes::IfcAttributeService;
use crate::classifications::IfcClassificationService;
use crate::decomposition::IfcDecompositionService;
use crate::identity::{GlobalIds, IFC_GLOBAL_ID};
use crate::integrity::IfcIntegrity;
use crate::materials::IfcMaterialService;
use crate::relationships::IfcRelationshipService;
use crate::release::Release;
use crate::unread::UnreadDefinitions;

/// Production IFC import/session construction failure.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum IfcSessionError {
    /// Source document identity was blank or otherwise invalid.
    #[error("invalid IFC source identity: {0}")]
    Identity(String),
    /// Strict STEP parsing failed.
    #[error("IFC STEP parse failed: {0}")]
    Parse(String),
    /// The parser recovered with diagnostics, so the snapshot is incomplete.
    #[error("IFC model is incomplete: {diagnostics} parser diagnostics")]
    IncompleteModel {
        /// Number of source diagnostics retained by the parser.
        diagnostics: usize,
    },
    /// The file did not declare exactly one supported schema (IFC2X3 or IFC4).
    #[error("exact IFC sessions require one IFC2X3 or IFC4 schema declaration, found {0:?}")]
    UnsupportedSchema(Vec<String>),
    /// Source-neutral project construction failed.
    #[error("failed to construct source-neutral project: {0}")]
    Project(String),
    /// Immutable snapshot/session binding failed.
    #[error("failed to construct evidence session: {0}")]
    Session(String),
}

/// Entity inheritance of the session's release, from its bundled normative schema.
struct IfcTypeHierarchy {
    release: Release,
    snapshots: Arc<[SourceSnapshot]>,
}

impl TypeHierarchyService for IfcTypeHierarchy {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn is_a(&self, kind: &str, ancestor: &str) -> Result<bool, TypeHierarchyError> {
        let schema = self.release.schema;
        // `is_a` answers false for a name the schema does not declare, which
        // would make an unknown kind look like a proven non-member.
        for name in [kind, ancestor] {
            if schema.entity(name).is_none() {
                return Err(TypeHierarchyError::UnknownType(format!(
                    "`{name}` is not an {} entity",
                    self.release.label
                )));
            }
        }
        Ok(schema.is_a(kind, ancestor))
    }
}

#[derive(Clone)]
struct IfcPropertyService {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    unread: Arc<UnreadDefinitions>,
}

impl IfcPropertyService {
    fn entity_id(request: &PropertyRequest) -> Result<EntityId, PropertyResolutionError> {
        let local = request
            .object_id()
            .local_id
            .strip_prefix('#')
            .unwrap_or(&request.object_id().local_id);
        local
            .parse::<u64>()
            .map(EntityId)
            .map_err(|_| PropertyResolutionError::InvalidRequest)
    }

    fn locator(&self, detail: impl std::fmt::Display) -> String {
        format!("ifc:{}:{detail}", self.snapshots[0].fingerprint())
    }
}

impl IfcPropertyService {
    /// Whether a value of the declared defined type maps onto a property
    /// value without loss.
    ///
    /// Decided by the type's base in this release's schema, so every
    /// string-based type (`IfcLabel`, `IfcDate`, `IfcDuration`, ...) and every
    /// integer-based one (`IfcTimeStamp`, `IfcCountMeasure` as written) is
    /// carried with its declared type. Real-valued measures stay refused: their
    /// number means nothing without the unit context, which is not read yet.
    /// `IFCREAL` and dimensionless `NUMBER` types have no unit.
    fn carries_exactly(&self, value: &ExactValue, value_type: &str) -> bool {
        let base = self
            .release
            .schema
            .resolve_defined(value_type)
            .to_ascii_uppercase();
        let base = base.split('(').next().unwrap_or_default().trim();
        match value {
            ExactValue::Bool(_) => base == "BOOLEAN",
            ExactValue::Integer(_) => base == "INTEGER" || base == "NUMBER",
            ExactValue::Real(_) => value_type.eq_ignore_ascii_case("IFCREAL") || base == "NUMBER",
            ExactValue::Text(_) => base == "STRING",
            _ => false,
        }
    }
}

impl PropertyResolutionService for IfcPropertyService {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        if request.object_id().source != *self.snapshots[0].source() {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        let object = Self::entity_id(request)?;
        match exact_property(
            &self.model,
            object,
            request.property_set(),
            request.property(),
        ) {
            Ok(ExactResolution::Present(exact)) => {
                let provenance = match exact.source {
                    ExactSource::Occurrence => "occurrence".to_owned(),
                    ExactSource::Type(type_id) => format!("type:{type_id}"),
                    _ => return Err(PropertyResolutionError::InexactEvidence),
                };
                if exact.unit_id.is_some() {
                    return Err(PropertyResolutionError::InexactEvidence);
                }
                let compatible_type = match (&exact.value, exact.value_type.as_deref()) {
                    (ExactValue::Null, None) => true,
                    (value, Some(value_type)) => self.carries_exactly(value, value_type),
                    _ => false,
                };
                if !compatible_type {
                    return Err(PropertyResolutionError::InexactEvidence);
                }
                let value = match exact.value {
                    ExactValue::Null => PropertyValue::Null,
                    ExactValue::Bool(value) => PropertyValue::Boolean(value),
                    ExactValue::Integer(value) => PropertyValue::Integer(value),
                    ExactValue::Real(value) => PropertyValue::Decimal(value),
                    ExactValue::Text(value) => PropertyValue::String(value.to_string()),
                    _ => return Err(PropertyResolutionError::InexactEvidence),
                };
                let mut property =
                    Property::new(exact.property_set.as_ref(), request.property(), value)
                        .map_err(|_| PropertyResolutionError::InvalidRequest)?;
                // STEP writes type names upper case; report them that way
                // whatever case the file used.
                if let Some(value_type) = exact.value_type.as_deref() {
                    property = property
                        .with_data_type(value_type.to_ascii_uppercase())
                        .map_err(|_| PropertyResolutionError::InexactEvidence)?;
                }
                let property = property.with_evidence(Evidence::exact(
                    self.snapshots[0].source().clone(),
                    self.locator(format_args!(
                        "{provenance}:{}/{}",
                        exact.set_id, exact.property_id
                    )),
                ));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            Ok(ExactResolution::Absent) => {
                // Upstream proves absence from property sets only.
                if let Some(reason) = self
                    .unread
                    .obscures(request.property_set(), request.property())
                {
                    return Err(PropertyResolutionError::Incomplete(reason));
                }
                Ok(PropertyResolution::Absent(
                    CompletePropertyAbsenceEvidence::try_new(
                        request.clone(),
                        Evidence::exact(
                            self.snapshots[0].source().clone(),
                            self.locator(format_args!(
                                "absence:{object}:{}:{}",
                                request.property_set().unwrap_or("*"),
                                request.property()
                            )),
                        ),
                    )?,
                ))
            }
            Ok(_) => Err(PropertyResolutionError::InexactEvidence),
            Err(error) => Err(map_resolution_error(&error)),
        }
    }
}

fn map_resolution_error(error: &ExactPropertyError) -> PropertyResolutionError {
    match error {
        ExactPropertyError::IncompleteModel { .. }
        | ExactPropertyError::MissingReference { .. }
        | ExactPropertyError::MalformedEntitySlots { .. }
        | ExactPropertyError::MalformedAggregate { .. }
        | ExactPropertyError::DuplicateAggregateMember { .. }
        | ExactPropertyError::MalformedName { .. }
        | ExactPropertyError::MissingValueSlot { .. }
        | ExactPropertyError::InvalidOccurrenceTarget { .. }
        | ExactPropertyError::InvalidTypeTarget { .. } => {
            PropertyResolutionError::Incomplete(error.to_string())
        }
        ExactPropertyError::MultipleTypeAssignments { .. }
        | ExactPropertyError::DuplicateMatchingSets { .. }
        | ExactPropertyError::DuplicateMatchingProperties { .. } => {
            PropertyResolutionError::Conflicting(error.to_string())
        }
        ExactPropertyError::UnsupportedDefinition { .. }
        | ExactPropertyError::UnsupportedProperty { .. }
        | ExactPropertyError::UnsupportedValue { .. }
        | ExactPropertyError::UnsupportedUnit { .. }
        | ExactPropertyError::NonFiniteReal { .. } => PropertyResolutionError::InexactEvidence,
        _ => PropertyResolutionError::Unavailable(error.to_string()),
    }
}

/// Parses strict IFC STEP bytes and binds an immutable exact-evidence session.
///
/// # Errors
///
/// Returns [`IfcSessionError`] when identity, strict parsing, schema validation,
/// source-neutral project construction, snapshot binding, or service registration fails.
pub fn import_ifc_session(
    document: impl Into<String>,
    bytes: &[u8],
) -> Result<EvidenceSession, IfcSessionError> {
    let source = SourceId::new("ifc-step", document.into())
        .map_err(|error| IfcSessionError::Identity(error.to_string()))?;
    let model = StepCodec
        .read_bytes(bytes)
        .map_err(|error| IfcSessionError::Parse(error.to_string()))?;
    if !model.diagnostics().is_empty() {
        return Err(IfcSessionError::IncompleteModel {
            diagnostics: model.diagnostics().len(),
        });
    }
    let schemas = model.header().schema.clone();
    let Some(release) = Release::from_header(&schemas) else {
        return Err(IfcSessionError::UnsupportedSchema(schemas));
    };

    let fingerprint: Arc<str> = Arc::from(format!("sha256:{:x}", Sha256::digest(bytes)));
    let global_ids = Arc::new(GlobalIds::read(release, &model));
    let objects = model
        .iter()
        .filter(|(_, entity)| release.schema.is_a(&entity.type_name, "IFCOBJECT"))
        .map(|(id, entity)| {
            let object = Object::new(
                ObjectId::new(source.clone(), id.to_string())?,
                entity.type_name.to_string(),
            );
            Ok(match global_ids.of(id) {
                Some(global_id) => {
                    object.with_external_id(ExternalId::new(IFC_GLOBAL_ID, global_id)?)
                }
                None => object,
            })
        })
        .collect::<Result<Vec<_>, IrError>>()
        .map_err(|error| IfcSessionError::Project(error.to_string()))?;
    let project =
        Project::new(objects).map_err(|error| IfcSessionError::Project(error.to_string()))?;
    let snapshot =
        SourceSnapshot::try_new(source.clone(), fingerprint.clone(), fingerprint.clone())
            .and_then(|snapshot| snapshot.with_schema(release.label))
            .and_then(|snapshot| snapshot.with_type_system(release.type_system))
            .map_err(|error| session_error(&error))?;
    let snapshots: Arc<[SourceSnapshot]> = Arc::from([snapshot.clone()]);
    let model = Arc::new(model);
    let service = PropertyResolutionServiceHandle::new(Arc::new(IfcPropertyService {
        release,
        model: model.clone(),
        snapshots: snapshots.clone(),
        unread: Arc::new(UnreadDefinitions::read(release, &model)),
    }));
    let integrity = SourceIntegrityServiceHandle::new(Arc::new(IfcIntegrity::new(
        release,
        model.clone(),
        global_ids,
        snapshots.clone(),
    )));
    let attribute_service = Arc::new(IfcAttributeService::new(
        release,
        model.clone(),
        snapshots.clone(),
    ));
    let attributes = AttributeServiceHandle::new(attribute_service.clone());
    let materials = MaterialServiceHandle::new(Arc::new(IfcMaterialService::new(
        release,
        model.clone(),
        snapshots.clone(),
    )));
    let classifications = ClassificationServiceHandle::new(Arc::new(
        IfcClassificationService::new(model.clone(), snapshots.clone()),
    ));
    let relationship_service = Arc::new(IfcRelationshipService::new(
        release,
        model.clone(),
        snapshots.clone(),
    ));
    let decomposition = DecompositionServiceHandle::new(Arc::new(IfcDecompositionService::new(
        release,
        model,
        snapshots.clone(),
        relationship_service.clone(),
        attribute_service,
    )));
    let relationships = RelationshipSelectionServiceHandle::new(relationship_service);
    let hierarchy =
        TypeHierarchyServiceHandle::new(Arc::new(IfcTypeHierarchy { release, snapshots }));
    EvidenceSession::try_new(project, [snapshot])
        .map_err(|error| session_error(&error))?
        .with_service(service)
        .and_then(|session| session.with_service(relationships))
        .and_then(|session| session.with_service(hierarchy))
        .and_then(|session| session.with_service(integrity))
        .and_then(|session| session.with_service(classifications))
        .and_then(|session| session.with_service(attributes))
        .and_then(|session| session.with_service(materials))
        .and_then(|session| session.with_service(decomposition))
        .map_err(|error| session_error(&error))
}

fn session_error(error: &EvidenceSessionError) -> IfcSessionError {
    IfcSessionError::Session(error.to_string())
}
