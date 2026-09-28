use std::sync::Arc;

use axioval_engine::{
    ClassificationServiceHandle, CompletePropertyAbsenceEvidence, CoordinateSystemServiceHandle,
    EvidenceSession, EvidenceSessionError, ObjectFrameServiceHandle, PropertyEnumeration,
    PropertyEnumerationRequest, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, RelationshipSelectionServiceHandle,
    ResolvedProperty, ResourceServiceHandle, SourceIntegrityServiceHandle, SourceSnapshot,
    TypeHierarchyError, TypeHierarchyService, TypeHierarchyServiceHandle, UnreadableValue,
};
use axioval_ir::{
    Evidence, ExternalId, IrError, Object, ObjectId, Project, Property, PropertyTableRow,
    PropertyValue, SourceId, is_reserved_set,
};
use ifc_model::{Codec, EntityId, Model};
use ifc_properties::{
    ExactLogical, ExactProperty, ExactPropertyEntry, ExactPropertyError, ExactPropertySetEntry,
    ExactResolution, ExactSource, ExactTableValue, ExactTypedValue, ExactValue,
    exact_material_properties_where, exact_material_property, exact_material_property_sets_where,
    exact_properties_where, exact_property, exact_property_sets_where,
};
use ifc_step::StepCodec;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::attributes::Attributes;
use crate::classifications::IfcClassificationService;
use crate::coordinates::IfcCoordinateSystem;
use crate::frames::IfcObjectFrames;
use crate::identity::{GlobalIds, IFC_GLOBAL_ID};
use crate::integrity::IfcIntegrity;
use crate::measure::si_value;
use crate::relationships::IfcRelationshipService;
use crate::release::Release;
use crate::resources::{IfcResources, is_object};
use crate::temporal;

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

struct IfcPropertyService {
    release: Release,
    model: Arc<Model>,
    snapshots: Arc<[SourceSnapshot]>,
    attributes: Attributes,
    levels: crate::levels::Levels,
}

impl IfcPropertyService {
    fn entity_id(object: &ObjectId) -> Result<EntityId, PropertyResolutionError> {
        let local = object
            .local_id
            .strip_prefix('#')
            .unwrap_or(&object.local_id);
        local
            .parse::<u64>()
            .map(EntityId)
            .map_err(|_| PropertyResolutionError::InvalidRequest)
    }

    /// The property `exact` resolved under `name`, with its declared type
    /// and occurrence or type provenance.
    fn property(
        &self,
        exact: &ExactProperty,
        name: &str,
    ) -> Result<Property, PropertyResolutionError> {
        let provenance = match exact.source {
            ExactSource::Occurrence => "occurrence".to_owned(),
            ExactSource::Type(type_id) => format!("type:{type_id}"),
            ExactSource::Material(material) => format!("material:{material}"),
            _ => return Err(PropertyResolutionError::InexactEvidence),
        };
        // A complex property or quantity groups its members and is no
        // value of any type, so it declares none. Its members are not
        // read: no rule compares one.
        let (value, data_type) = match &exact.value {
            ExactValue::Complex(_) if exact.value_type.is_none() && exact.unit_id.is_none() => {
                (PropertyValue::Complex, None)
            }
            _ => self.pset_value(exact)?,
        };
        let mut property = Property::new(exact.property_set.as_ref(), name, value)
            .map_err(|_| PropertyResolutionError::InvalidRequest)?;
        if let Some(data_type) = data_type {
            property = property
                .with_data_type(data_type)
                .map_err(|_| PropertyResolutionError::InexactEvidence)?;
        }
        // Each column of a table declares one type (the library checks
        // it), which the table's own type cannot state when they differ.
        if let (ExactValue::Table(table), PropertyValue::Table(_)) = (&exact.value, &property.value)
            && let Some(row) = table.rows.first()
        {
            property = property
                .with_column_types(
                    row.defining.value_type.to_ascii_uppercase(),
                    row.defined.value_type.to_ascii_uppercase(),
                )
                .map_err(|_| PropertyResolutionError::InexactEvidence)?;
        }
        Ok(property.with_evidence(Evidence::exact(
            self.snapshots[0].source().clone(),
            self.locator(format_args!(
                "{provenance}:{}/{}",
                exact.set_id, exact.property_id
            )),
        )))
    }

    fn locator(&self, detail: impl std::fmt::Display) -> String {
        format!("ifc:{}:{detail}", self.snapshots[0].fingerprint())
    }

    /// Why a present property's value could not be read, as an answer: a
    /// single stated value (a number or text, never `$` or a composite
    /// value) of a declared type the file states is `UnreadableValue`, so
    /// its type and presence are still decided. Anything else stays the
    /// `Incomplete` refusal it was.
    fn unreadable(
        &self,
        request: &PropertyRequest,
        exact: &ExactProperty,
        reason: String,
    ) -> PropertyResolutionError {
        let stated = matches!(
            exact.value,
            ExactValue::Real(_) | ExactValue::Integer(_) | ExactValue::Text(_)
        );
        let provenance = match exact.source {
            ExactSource::Occurrence => "occurrence".to_owned(),
            ExactSource::Type(type_id) => format!("type:{type_id}"),
            ExactSource::Material(material) => format!("material:{material}"),
            _ => return PropertyResolutionError::InexactEvidence,
        };
        let (true, Some(data_type)) = (stated, exact.value_type.as_deref()) else {
            return PropertyResolutionError::Incomplete(reason);
        };
        let evidence = Evidence::exact(
            self.snapshots[0].source().clone(),
            self.locator(format_args!(
                "{provenance}:{}/{}",
                exact.set_id, exact.property_id
            )),
        );
        match UnreadableValue::try_new(
            request.clone(),
            data_type.to_ascii_uppercase(),
            evidence,
            reason.clone(),
        ) {
            Ok(unreadable) => PropertyResolutionError::UnreadableValue(Box::new(unreadable)),
            Err(_) => PropertyResolutionError::Incomplete(reason),
        }
    }
}

impl IfcPropertyService {
    /// Whether a value of the declared defined type maps onto a property
    /// value without loss.
    ///
    /// Decided by the type's base in this release's schema, so every
    /// string-based type (`IfcLabel`, `IfcDate`, `IfcDuration`, ...) and every
    /// integer-based one (`IfcTimeStamp`, `IfcCountMeasure` as written) is
    /// carried with its declared type. Real-valued measures are not carried
    /// here: `scalar_value` converts them through their unit. `IFCREAL` and
    /// dimensionless `NUMBER` types have no unit. An enumeration constant of
    /// a predefined set's attribute is carried as its text.
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
            ExactValue::Enum(_) => true,
            _ => false,
        }
    }

    /// The value of a property, a quantity or a predefined set's attribute,
    /// measures converted to SI, with its declared type.
    ///
    /// A single value, a quantity and an attribute are one scalar (see
    /// `scalar_value`). The composite kinds map onto the IR's composite
    /// values, every scalar in them converted as a single value of its own
    /// declared type with the unit the kind states for it:
    ///
    /// - an enumerated value is its selected item, a list of them when
    ///   several are selected, and null when none is;
    /// - a list value is a list, null when it states no element;
    /// - a bounded value is a bounded value, null when it states neither
    ///   bound nor set point;
    /// - a table value is a table, null when it has no row.
    ///
    /// The declared type is the one type every scalar declares; a table
    /// whose two columns declare different types has none. A reference
    /// value names an entity the IR cannot carry and is refused, and so is
    /// a complex property here (`property` reads it).
    fn pset_value(
        &self,
        exact: &ExactProperty,
    ) -> Result<(PropertyValue, Option<String>), PropertyResolutionError> {
        let scalars = |values: &[ExactTypedValue], unit| {
            values
                .iter()
                .map(|typed| self.scalar_value(&typed.value, Some(&typed.value_type), unit))
                .collect::<Result<Vec<_>, _>>()
        };
        let boxed = |typed: &Option<ExactTypedValue>| {
            typed
                .as_ref()
                .map(|typed| {
                    self.scalar_value(&typed.value, Some(&typed.value_type), exact.unit_id)
                        .map(Box::new)
                })
                .transpose()
        };
        let (value, types): (PropertyValue, Vec<&str>) = match &exact.value {
            ExactValue::Enumerated(enumerated) => {
                let mut items = scalars(&enumerated.values, exact.unit_id)?;
                let mut types: Vec<&str> = enumerated
                    .values
                    .iter()
                    .map(|typed| typed.value_type.as_ref())
                    .collect();
                if let Some(enumeration) = &enumerated.enumeration {
                    types.extend(
                        enumeration
                            .values
                            .iter()
                            .map(|typed| typed.value_type.as_ref()),
                    );
                }
                let value = match items.len() {
                    0 => PropertyValue::Null,
                    1 => items.remove(0),
                    _ => PropertyValue::List(items),
                };
                (value, types)
            }
            ExactValue::List(values) => {
                let items = scalars(values, exact.unit_id)?;
                let value = if items.is_empty() {
                    PropertyValue::Null
                } else {
                    PropertyValue::List(items)
                };
                (
                    value,
                    values
                        .iter()
                        .map(|typed| typed.value_type.as_ref())
                        .collect(),
                )
            }
            ExactValue::Bounded(bounded) => {
                let parts = [&bounded.lower, &bounded.upper, &bounded.set_point];
                let types = parts
                    .iter()
                    .filter_map(|part| part.as_ref())
                    .map(|typed| typed.value_type.as_ref())
                    .collect();
                let value = if parts.iter().all(|part| part.is_none()) {
                    PropertyValue::Null
                } else {
                    PropertyValue::Bounded {
                        lower: boxed(&bounded.lower)?,
                        upper: boxed(&bounded.upper)?,
                        set_point: boxed(&bounded.set_point)?,
                    }
                };
                (value, types)
            }
            ExactValue::Table(table) => self.table_value(table)?,
            // A reference value referencing nothing (`PropertyReference`
            // `$`, optional since IFC4) states no value, as an enumerated
            // value selecting nothing; one naming an entity is refused.
            ExactValue::Reference(reference) if reference.target.is_none() => {
                (PropertyValue::Null, Vec::new())
            }
            ExactValue::Reference(_) | ExactValue::Entity(_) => {
                return Err(PropertyResolutionError::InexactEvidence);
            }
            // A logical unknown states no truth value: the property holds
            // no value, exactly as `$` (and as an attribute's `.U.`), with
            // its declared type. An element of a composite value cannot be
            // null, so there `scalar_value` refuses it.
            ExactValue::Logical(ExactLogical::Unknown) if exact.unit_id.is_none() => {
                return Ok((
                    PropertyValue::Null,
                    exact.value_type.as_deref().map(str::to_ascii_uppercase),
                ));
            }
            scalar => {
                let value =
                    self.scalar_value(scalar, exact.value_type.as_deref(), exact.unit_id)?;
                return Ok((
                    value,
                    exact.value_type.as_deref().map(str::to_ascii_uppercase),
                ));
            }
        };
        // STEP writes type names upper case; report them that way whatever
        // case the file used.
        let mut types = types.into_iter().map(str::to_ascii_uppercase);
        let first = types.next();
        let declared = match first {
            Some(first) if types.all(|other| other == first) => Some(first),
            _ => None,
        };
        Ok((value, declared))
    }

    /// A table value's rows, each column in its own unit, and every cell's
    /// declared type.
    fn table_value<'t>(
        &self,
        table: &'t ExactTableValue,
    ) -> Result<(PropertyValue, Vec<&'t str>), PropertyResolutionError> {
        let mut rows = Vec::with_capacity(table.rows.len());
        let mut types = Vec::with_capacity(2 * table.rows.len());
        for row in &table.rows {
            rows.push(PropertyTableRow {
                defining: self.scalar_value(
                    &row.defining.value,
                    Some(&row.defining.value_type),
                    table.defining_unit,
                )?,
                defined: self.scalar_value(
                    &row.defined.value,
                    Some(&row.defined.value_type),
                    table.defined_unit,
                )?,
            });
            types.push(row.defining.value_type.as_ref());
            types.push(row.defined.value_type.as_ref());
        }
        let value = if rows.is_empty() {
            PropertyValue::Null
        } else {
            PropertyValue::Table(rows)
        };
        Ok((value, types))
    }

    /// One scalar value of declared type `value_type` with the effective
    /// explicit `unit`, measures converted to SI.
    ///
    /// A value its declared type carries exactly (see `carries_exactly`) is
    /// read as stated and must carry no unit; a date or time type is read as
    /// a date or date-time (see `temporal`); `$` of a predefined set's
    /// optional attribute is null; an `IfcLogical` true or false is a
    /// boolean, and its unknown is refused here (a single value reads it as
    /// null in `pset_value`). Any other number must be a measure whose
    /// effective unit resolves exactly.
    fn scalar_value(
        &self,
        value: &ExactValue,
        value_type: Option<&str>,
        unit: Option<EntityId>,
    ) -> Result<PropertyValue, PropertyResolutionError> {
        if let Some(value_type) = value_type {
            let raw = match value {
                ExactValue::Text(text) => temporal::Raw::Text(text),
                ExactValue::Integer(seconds) => temporal::Raw::Integer(*seconds),
                _ => temporal::Raw::Other,
            };
            if let Some(value) = temporal::read(value_type, &raw) {
                return if unit.is_some() {
                    Err(PropertyResolutionError::InexactEvidence)
                } else {
                    value
                };
            }
        }
        let plain = match (value, value_type) {
            (ExactValue::Null, _) => Some(PropertyValue::Null),
            (ExactValue::Logical(ExactLogical::True), _) => Some(PropertyValue::Boolean(true)),
            (ExactValue::Logical(ExactLogical::False), _) => Some(PropertyValue::Boolean(false)),
            (value, Some(value_type)) if self.carries_exactly(value, value_type) => match value {
                ExactValue::Bool(value) => Some(PropertyValue::Boolean(*value)),
                ExactValue::Integer(value) => Some(PropertyValue::Integer(*value)),
                ExactValue::Real(value) => Some(PropertyValue::Decimal(*value)),
                ExactValue::Text(value) | ExactValue::Enum(value) => {
                    Some(PropertyValue::String(value.to_string()))
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(value) = plain {
            return if unit.is_some() {
                Err(PropertyResolutionError::InexactEvidence)
            } else {
                Ok(value)
            };
        }
        let number = match value {
            ExactValue::Real(value) => *value,
            #[allow(clippy::cast_precision_loss)]
            ExactValue::Integer(value) if value.unsigned_abs() <= 1 << 53 => *value as f64,
            _ => return Err(PropertyResolutionError::InexactEvidence),
        };
        let Some(value_type) = value_type else {
            return Err(PropertyResolutionError::InexactEvidence);
        };
        si_value(&self.model, value_type, unit, number)?
            .ok_or(PropertyResolutionError::InexactEvidence)
    }

    fn resolve_attribute(
        &self,
        request: &PropertyRequest,
        object: EntityId,
        set: &str,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        if self.model.get(object).is_none() {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        let source = self.snapshots[0].source().clone();
        match self
            .attributes
            .resolve((&source, &self.model), object, set, request.property())?
        {
            Some(found) => {
                let property = Property::new(set, request.property(), found.value)
                    .map_err(|_| PropertyResolutionError::InvalidRequest)?
                    .with_evidence(Evidence::exact(source, self.locator(found.detail)));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            None => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(
                        source,
                        self.locator(format_args!(
                            "absence:{object}:{set}:{}",
                            request.property()
                        )),
                    ),
                )?,
            )),
        }
    }
}

impl IfcPropertyService {
    /// The measured values an IFC file states rather than its geometry:
    /// `level_height`, a storey's height to the next storey. Every other
    /// measured name is the engine's and never answered here.
    fn resolve_measured(
        &self,
        request: &PropertyRequest,
        object: EntityId,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        if !request
            .property()
            .eq_ignore_ascii_case(axioval_ir::MEASURED_LEVEL_HEIGHT)
            || self.model.get(object).is_none()
        {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        let source = self.snapshots[0].source().clone();
        match self.levels.height(&self.model, object) {
            Ok(Some((metres, detail))) => {
                let property = Property::new(
                    axioval_ir::MEASURED_SET,
                    request.property(),
                    PropertyValue::Quantity {
                        value: metres,
                        dimension: axioval_ir::QuantityDimension::Length,
                    },
                )
                .map_err(|_| PropertyResolutionError::InvalidRequest)?
                .with_evidence(Evidence::exact(source, self.locator(detail)));
                Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                    request.clone(),
                    property,
                )?))
            }
            Ok(None) => Ok(PropertyResolution::Absent(
                CompletePropertyAbsenceEvidence::try_new(
                    request.clone(),
                    Evidence::exact(
                        source,
                        self.locator(format_args!("level-height:{object}:none")),
                    ),
                )?,
            )),
            Err(why) => Err(PropertyResolutionError::Unavailable(format!(
                "the level height of {object} cannot be read: {why}"
            ))),
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
        let object = Self::entity_id(request.object_id())?;
        if request.property_set() == Some(axioval_ir::MEASURED_SET) {
            return self.resolve_measured(request, object);
        }
        if let Some(set) = request.property_set().filter(|set| is_reserved_set(set)) {
            return self.resolve_attribute(request, object, set);
        }
        if self.model.get(object).is_none() {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        match self.exact_property(object, request.property_set(), request.property()) {
            Ok(ExactResolution::Present(exact)) => {
                match self.property(&exact, request.property()) {
                    Ok(property) => Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                        request.clone(),
                        property,
                    )?)),
                    Err(PropertyResolutionError::Incomplete(reason)) => {
                        Err(self.unreadable(request, &exact, reason))
                    }
                    Err(error) => Err(error),
                }
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
            Err(error) => match request.property_set() {
                Some(set) if self.only_empty(object, set, &error) => Ok(
                    PropertyResolution::Absent(CompletePropertyAbsenceEvidence::try_new(
                        request.clone(),
                        Evidence::exact(
                            self.snapshots[0].source().clone(),
                            self.locator(format_args!(
                                "absence:{object}:{set}:{}:empty-set",
                                request.property()
                            )),
                        ),
                    )?),
                ),
                _ => Err(map_resolution_error(&error)),
            },
        }
    }

    /// Every property, quantity and predefined-set attribute of the object
    /// the request selects, through `exact_properties_where`: the traversal
    /// and refusals of `exact_property`, so an empty answer is as exact as
    /// its absence. A set of a reserved name is never selected, and a
    /// selected value the IR cannot carry refuses the whole answer.
    fn enumerate(
        &self,
        request: &PropertyEnumerationRequest,
    ) -> Result<PropertyEnumeration, PropertyResolutionError> {
        let source = self.snapshots[0].source().clone();
        if request.object_id().source != source {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        let object = Self::entity_id(request.object_id())?;
        if self.model.get(object).is_none() {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        let selected = |set: &str| !is_reserved_set(set) && request.property_set().matches(set);
        let empty = self.empty_sets(object, selected)?;
        let entries = self
            .exact_properties_where(
                object,
                |set| selected(set) && !empty.iter().any(|name| name == set),
                |name| request.property().matches(name),
            )
            .map_err(|error| map_resolution_error(&error))?;
        let properties = entries
            .iter()
            .map(|entry| self.property(&entry.property, &entry.name))
            .collect::<Result<Vec<_>, _>>()?;
        PropertyEnumeration::try_new(
            request.clone(),
            properties,
            Evidence::exact(
                source,
                self.locator(format_args!(
                    "enumeration:{object}:{}:{}",
                    request.property_set(),
                    request.property()
                )),
            ),
        )?
        .with_empty_sets(empty)
    }
}

impl IfcPropertyService {
    /// Whether `object` is read through the material property readers: an
    /// instance that is no session object (`is_object`). A material
    /// definition's own sets (`ifc-properties` 0.5.3, openbimrs/ifc#218)
    /// resolve there; any other resource is `InvalidQueryObject`, refused.
    fn is_resource(&self, object: EntityId) -> bool {
        self.model
            .get(object)
            .is_some_and(|entity| !is_object(self.release, &entity.type_name))
    }

    /// `exact_property`, or for a resource `exact_material_property`.
    fn exact_property(
        &self,
        object: EntityId,
        set: Option<&str>,
        name: &str,
    ) -> Result<ExactResolution, ExactPropertyError> {
        if self.is_resource(object) {
            exact_material_property(&self.model, object, set, name)
        } else {
            exact_property(&self.model, object, set, name)
        }
    }

    /// `exact_properties_where`, or for a resource its material
    /// counterpart.
    fn exact_properties_where(
        &self,
        object: EntityId,
        select_set: impl Fn(&str) -> bool,
        select_property: impl Fn(&str) -> bool,
    ) -> Result<Vec<ExactPropertyEntry>, ExactPropertyError> {
        if self.is_resource(object) {
            exact_material_properties_where(&self.model, object, select_set, select_property)
        } else {
            exact_properties_where(&self.model, object, select_set, select_property)
        }
    }

    /// `exact_property_sets_where`, or for a resource its material
    /// counterpart.
    fn exact_property_sets_where(
        &self,
        object: EntityId,
        select: impl Fn(&str) -> bool,
    ) -> Result<Vec<ExactPropertySetEntry>, ExactPropertyError> {
        if self.is_resource(object) {
            exact_material_property_sets_where(&self.model, object, select)
        } else {
            exact_property_sets_where(&self.model, object, select)
        }
    }

    /// The names of the sets `select` picks on `object` of which every set
    /// is empty (`HasProperties` or `Quantities` of `()` or `$`). Such a set
    /// exists and holds nothing, which a rule requiring a property in it
    /// must see. A name an empty set shares with a set holding members is
    /// not listed, so `exact_properties_where` still refuses it.
    fn empty_sets(
        &self,
        object: EntityId,
        select: impl Fn(&str) -> bool,
    ) -> Result<Vec<String>, PropertyResolutionError> {
        let sets = self
            .exact_property_sets_where(object, &select)
            .map_err(|error| map_resolution_error(&error))?;
        let mut empty: Vec<String> = sets
            .iter()
            .filter(|entry| entry.members == 0)
            .map(|entry| entry.name.to_string())
            .filter(|name| {
                sets.iter()
                    .all(|entry| *entry.name != **name || entry.members == 0)
            })
            .collect();
        empty.sort();
        empty.dedup();
        Ok(empty)
    }

    /// Whether `error`, refusing a property of set `set`, is only the
    /// refusal of that set's empty member list: every set of that name is
    /// empty and `error` names one of them. Such a set holds no property,
    /// so the property is absent from it.
    fn only_empty(&self, object: EntityId, set: &str, error: &ExactPropertyError) -> bool {
        let ExactPropertyError::MalformedAggregate { entity, .. } = error else {
            return false;
        };
        self.exact_property_sets_where(object, |name| name == set)
            .is_ok_and(|sets| {
                sets.iter().all(|entry| entry.members == 0)
                    && sets.iter().any(|entry| entry.set_id == *entity)
            })
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
        | ExactPropertyError::InvalidTypeTarget { .. }
        | ExactPropertyError::ComplexCycle { .. } => {
            PropertyResolutionError::Incomplete(error.to_string())
        }
        ExactPropertyError::MultipleTypeAssignments { .. }
        | ExactPropertyError::DuplicateMatchingSets { .. }
        | ExactPropertyError::DuplicateMatchingProperties { .. }
        | ExactPropertyError::InconsistentValues { .. } => {
            PropertyResolutionError::Conflicting(error.to_string())
        }
        // Neither an occurrence nor a type object (a resource, a
        // relationship): refused, never read as absent.
        ExactPropertyError::InvalidQueryObject { type_name, .. } => {
            PropertyResolutionError::Unavailable(format!(
                "a {type_name} has no properties the IFC property library resolves"
            ))
        }
        ExactPropertyError::UnsupportedDefinition { .. }
        | ExactPropertyError::UnsupportedProperty { .. }
        | ExactPropertyError::UnsupportedValue { .. }
        | ExactPropertyError::UnsupportedUnit { .. }
        | ExactPropertyError::NonFiniteReal { .. } => PropertyResolutionError::InexactEvidence,
        // `ComplexTooDeep` and `ComplexBudgetExceeded` (a complex nested
        // beyond what the library follows) among them.
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
        // Occurrences (`IfcObject`), in IFC4 contexts such as `IfcProject`
        // (`IfcContext`, an `IfcObject` in IFC2X3), and type objects
        // (`IfcTypeObject`, IFC2X3 `IfcDoorStyle` included) are checked:
        // each answers its own properties exactly, a type object its own
        // `HasPropertySets` (`ifc-properties` ≥ 0.5.1). Every other
        // instance is a resource object, listed by class only when a rule
        // names it (`resources.rs`).
        .filter(|(_, entity)| is_object(release, &entity.type_name))
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
    let metadata = crate::metadata::read(release, &model);
    let model = Arc::new(model);
    let service = PropertyResolutionServiceHandle::new(Arc::new(IfcPropertyService {
        release,
        model: model.clone(),
        snapshots: snapshots.clone(),
        attributes: Attributes::new(release),
        levels: crate::levels::Levels::new(release),
    }));
    let resources = ResourceServiceHandle::new(Arc::new(IfcResources::new(
        release,
        model.clone(),
        global_ids.clone(),
        snapshots.clone(),
    )));
    let integrity = SourceIntegrityServiceHandle::new(Arc::new(IfcIntegrity::new(
        release,
        model.clone(),
        global_ids,
        snapshots.clone(),
    )));
    let classifications = ClassificationServiceHandle::new(Arc::new(
        IfcClassificationService::new(release, model.clone(), snapshots.clone()),
    ));
    let frames = ObjectFrameServiceHandle::new(Arc::new(IfcObjectFrames::new(
        release,
        model.clone(),
        snapshots.clone(),
    )));
    let coordinates = CoordinateSystemServiceHandle::new(Arc::new(IfcCoordinateSystem::new(
        release,
        model.clone(),
        snapshots.clone(),
    )));
    let relationships = RelationshipSelectionServiceHandle::new(Arc::new(
        IfcRelationshipService::new(release, model, snapshots.clone()),
    ));
    let hierarchy =
        TypeHierarchyServiceHandle::new(Arc::new(IfcTypeHierarchy { release, snapshots }));
    EvidenceSession::try_new(project, [snapshot])
        .map_err(|error| session_error(&error))?
        .with_service(service)
        .and_then(|session| session.with_service(relationships))
        .and_then(|session| session.with_service(hierarchy))
        .and_then(|session| session.with_service(integrity))
        .and_then(|session| session.with_service(classifications))
        .and_then(|session| session.with_service(frames))
        .and_then(|session| session.with_service(coordinates))
        .and_then(|session| session.with_service(resources))
        .and_then(|session| session.with_source_metadata(&source, metadata))
        .map_err(|error| session_error(&error))
}

fn session_error(error: &EvidenceSessionError) -> IfcSessionError {
    IfcSessionError::Session(error.to_string())
}
