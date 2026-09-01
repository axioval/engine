use std::sync::Arc;

use axioval_engine::{
    CompletePropertyAbsenceEvidence, EvidenceSession, EvidenceSessionError, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, ResolvedProperty, SourceSnapshot,
};
use axioval_ir::{Evidence, IrError, Object, ObjectId, Project, Property, PropertyValue, SourceId};
use ifc_model::{Codec, EntityId, Model};
use ifc_properties::{
    ExactPropertyError, ExactResolution, ExactSource, ExactValue, exact_property,
};
use ifc_schema::{SchemaVersion, ifc4};
use ifc_step::StepCodec;
use sha2::{Digest, Sha256};
use thiserror::Error;

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
    /// The file did not declare exactly one supported IFC4 schema.
    #[error("exact IFC sessions require one IFC4 schema declaration, found {0:?}")]
    UnsupportedSchema(Vec<String>),
    /// Source-neutral project construction failed.
    #[error("failed to construct source-neutral project: {0}")]
    Project(String),
    /// Immutable snapshot/session binding failed.
    #[error("failed to construct evidence session: {0}")]
    Session(String),
}

#[derive(Clone)]
struct IfcPropertyService {
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
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
                    (ExactValue::Bool(_), Some(value_type)) => {
                        value_type.eq_ignore_ascii_case("IFCBOOLEAN")
                    }
                    (ExactValue::Integer(_), Some(value_type)) => {
                        value_type.eq_ignore_ascii_case("IFCINTEGER")
                    }
                    (ExactValue::Real(_), Some(value_type)) => {
                        value_type.eq_ignore_ascii_case("IFCREAL")
                    }
                    (ExactValue::Text(_), Some(value_type)) => {
                        ["IFCTEXT", "IFCLABEL", "IFCIDENTIFIER"]
                            .iter()
                            .any(|candidate| value_type.eq_ignore_ascii_case(candidate))
                    }
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
                let property =
                    Property::new(exact.property_set.as_ref(), request.property(), value)
                        .map_err(|_| PropertyResolutionError::InvalidRequest)?
                        .with_evidence(Evidence::exact(
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
            Ok(ExactResolution::Absent) => Ok(PropertyResolution::Absent(
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
            )),
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
    if !matches!(schemas.as_slice(), [schema] if SchemaVersion::from_header_token(schema) == Some(SchemaVersion::Ifc4))
    {
        return Err(IfcSessionError::UnsupportedSchema(schemas));
    }

    let fingerprint: Arc<str> = Arc::from(format!("sha256:{:x}", Sha256::digest(bytes)));
    let objects = model
        .iter()
        .filter(|(_, entity)| ifc4().is_a(&entity.type_name, "IFCOBJECT"))
        .map(|(id, entity)| {
            ObjectId::new(source.clone(), id.to_string())
                .map(|object_id| Object::new(object_id, entity.type_name.to_string()))
        })
        .collect::<Result<Vec<_>, IrError>>()
        .map_err(|error| IfcSessionError::Project(error.to_string()))?;
    let project =
        Project::new(objects).map_err(|error| IfcSessionError::Project(error.to_string()))?;
    let snapshot =
        SourceSnapshot::try_new(source.clone(), fingerprint.clone(), fingerprint.clone())
            .and_then(|snapshot| snapshot.with_schema("IFC4"))
            .map_err(|error| session_error(&error))?;
    let service = PropertyResolutionServiceHandle::new(Arc::new(IfcPropertyService {
        model: Arc::new(model),
        snapshots: Arc::from([snapshot.clone()]),
    }));
    EvidenceSession::try_new(project, [snapshot])
        .map_err(|error| session_error(&error))?
        .with_service(service)
        .map_err(|error| session_error(&error))
}

fn session_error(error: &EvidenceSessionError) -> IfcSessionError {
    IfcSessionError::Session(error.to_string())
}
