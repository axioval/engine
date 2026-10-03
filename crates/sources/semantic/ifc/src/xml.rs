//! ifcXML: the XML serialization of IFC, read into the STEP reader's model.
//!
//! The document is parsed by `ifc-xml`'s `XmlCodec` into the same
//! `ifc_model::Model` the STEP codec produces, then bound by the same
//! session path as a STEP file ([`crate::ifc::session`]), so every service
//! answers an ifcXML model exactly as its STEP form.
//!
//! XML attribute values are untyped text, and `ifc-xml` reads the dialect
//! its own writer produces: attributes named by the release schema or by
//! position (`a<i>`), and every value an attribute cannot spell as a child
//! element with an explicit `kind`. A document in another arrangement, such
//! as the buildingSMART XSD configuration with nested entity elements and
//! `ref` attributes, would be read into wrong values without an error. So a
//! read is accepted only when it is provably the model the file states:
//!
//! - no start tag holds more than [`MAX_ATTRIBUTES`] attributes or
//!   [`MAX_NAMESPACES`] namespace declarations, checked before the codec
//!   reads, so a hostile document's cost stays bounded;
//! - the root declares one supported schema, and the document is read again
//!   with that release's attribute names;
//! - every attribute and attribute element of an entity is named by the
//!   release or by a position the entity has (`ifc-xml` appends a value of
//!   any other name after the named ones, into whichever slot comes next);
//! - every entity is declared by the release, and holds no more values than
//!   the release declares attributes (a value no attribute names would be
//!   placed in the wrong slot); trailing attributes the document omits are
//!   unset, as `ifc-xml` documents;
//! - every value conforms to its attribute's declared type
//!   (`ifc_validate::type_check`).
//!
//! Anything else is refused ([`IfcSessionError::Xml`]), never read in part.
//! Entity names are compared and stored upper case, as STEP writes them.

use std::sync::{Arc, OnceLock};

use axioval_engine::EvidenceSession;
use axioval_ir::SourceId;
use ifc_model::{Codec, Entity, Model, Value};
use ifc_schema::Schema;
use ifc_validate::{Budget, Report, Severity};
use ifc_xml::{SchemaReading, XmlCodec};

use crate::ifc::{IfcSessionError, session};
use crate::release::Release;

/// The file extension of an ifcXML document, without the dot.
pub const IFC_XML_EXTENSION: &str = "ifcxml";

/// Whether `bytes` start as an XML document does. A STEP file never does.
#[must_use]
pub fn is_ifc_xml(bytes: &[u8]) -> bool {
    ifc_xml::reader::looks_like_xml(bytes)
}

fn refused(message: impl Into<String>) -> IfcSessionError {
    IfcSessionError::Xml(message.into())
}

/// Each release's schema as the codec takes it, by release label.
type Schemas = std::sync::Mutex<Vec<(&'static str, Arc<Schema>)>>;

/// The release's schema as the codec takes it, built once per release.
fn shared(release: Release) -> Arc<Schema> {
    static SCHEMAS: OnceLock<Schemas> = OnceLock::new();
    let cache = SCHEMAS.get_or_init(Default::default);
    let mut cache = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, schema)) = cache.iter().find(|(label, _)| *label == release.label) {
        return schema.clone();
    }
    let schema = Arc::new(release.schema.clone());
    cache.push((release.label, schema.clone()));
    schema
}

/// The most attributes one start tag may hold. An IFC entity has fewer
/// than a hundred attributes.
pub(crate) const MAX_ATTRIBUTES: usize = 256;

/// The most namespace declarations one start tag may hold.
pub(crate) const MAX_NAMESPACES: usize = 8;

/// Refuses a document a start tag of which holds more attributes or
/// namespace declarations than the bounds, before the codec parses it.
fn bounded(bytes: &[u8]) -> Result<(), IfcSessionError> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| refused(error.to_string()))?
        {
            Event::Eof => return Ok(()),
            Event::Start(element) | Event::Empty(element) => {
                let mut attributes = 0_usize;
                let mut namespaces = 0_usize;
                for attribute in element.attributes().with_checks(false) {
                    let attribute = attribute.map_err(|error| refused(error.to_string()))?;
                    attributes += 1;
                    let key = attribute.key.as_ref();
                    if key == "xmlns" || key.starts_with("xmlns:") {
                        namespaces += 1;
                    }
                    if attributes > MAX_ATTRIBUTES || namespaces > MAX_NAMESPACES {
                        return Err(refused(format!(
                            "a start tag holds more than {MAX_ATTRIBUTES} attributes or {MAX_NAMESPACES} namespace declarations"
                        )));
                    }
                }
            }
            _ => {}
        }
        buffer.clear();
    }
}

/// Refuses a document stating a value under a name no attribute of its
/// entity has. Entities are the root's children with an `id`; their XML
/// attributes and child elements name attributes.
fn unnamed_values(bytes: &[u8], release: Release) -> Result<(), IfcSessionError> {
    use quick_xml::events::{BytesStart, Event};

    let schema = release.schema;
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    // The open entity's upper-case name and attribute names.
    let mut entity: Option<(String, Vec<String>)> = None;
    let known = |names: &[String], name: &str| {
        names.iter().any(|known| known == name)
            || name
                .strip_prefix('a')
                .and_then(|slot| slot.parse::<usize>().ok())
                .is_some_and(|slot| slot < names.len())
    };
    let open =
        |element: &BytesStart<'_>| -> Result<Option<(String, Vec<String>)>, IfcSessionError> {
            let mut has_id = false;
            let mut stated = Vec::new();
            for attribute in element.attributes() {
                let attribute = attribute.map_err(|error| refused(error.to_string()))?;
                let name = attribute.key.local_name().as_ref().to_owned();
                if name == "id" {
                    has_id = true;
                } else if attribute.key.prefix().is_none() {
                    stated.push(name);
                }
            }
            if !has_id {
                return Ok(None);
            }
            let name = element
                .local_name()
                .as_ref()
                .to_string()
                .to_ascii_uppercase();
            let names: Vec<String> = schema
                .attribute_names(&name)
                .into_iter()
                .map(str::to_owned)
                .collect();
            if let Some(unknown) = stated.iter().find(|stated| !known(&names, stated)) {
                return Err(refused(format!(
                    "a {name} states `{unknown}`, which names no attribute of it"
                )));
            }
            Ok(Some((name, names)))
        };
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| refused(error.to_string()))?;
        match event {
            Event::Eof => return Ok(()),
            Event::Start(element) => {
                depth += 1;
                match depth {
                    2 => entity = open(&element)?,
                    3 => {
                        if let Some((name, names)) = &entity {
                            let child = element.local_name().as_ref().to_owned();
                            if !known(names, &child) {
                                return Err(refused(format!(
                                    "a {name} states `{child}`, which names no attribute of it"
                                )));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Empty(element) => match depth + 1 {
                2 => {
                    open(&element)?;
                }
                3 => {
                    if let Some((name, names)) = &entity {
                        let child = element.local_name().as_ref().to_owned();
                        if !known(names, &child) {
                            return Err(refused(format!(
                                "a {name} states `{child}`, which names no attribute of it"
                            )));
                        }
                    }
                }
                _ => {}
            },
            Event::End(_) => {
                if depth == 2 {
                    entity = None;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        buffer.clear();
    }
}

/// Reads an ifcXML document into the model its STEP form parses to, and
/// the release it declares.
fn read(bytes: &[u8]) -> Result<(Release, Model), IfcSessionError> {
    bounded(bytes)?;
    // The root names the schema; attribute names depend on it.
    let declared = XmlCodec::default()
        .read_bytes(bytes)
        .map_err(|error| refused(error.to_string()))?;
    let schemas = declared.header().schema.clone();
    let Some(release) = Release::from_header(&schemas) else {
        return Err(IfcSessionError::UnsupportedSchema(schemas));
    };
    unnamed_values(bytes, release)?;
    // The pre-0.4 read, which the checks above and below prove.
    let read = XmlCodec::with_schema(shared(release))
        .with_reading(SchemaReading::Lenient)
        .read_bytes(bytes)
        .map_err(|error| refused(error.to_string()))?;
    if !read.diagnostics().is_empty() {
        return Err(IfcSessionError::IncompleteModel {
            diagnostics: read.diagnostics().len(),
        });
    }
    let schema = release.schema;
    let mut model = Model::new();
    *model.header_mut() = read.header().clone();
    for (id, entity) in read.iter() {
        let name = entity.type_name.to_ascii_uppercase();
        if schema.entity(&name).is_none() {
            return Err(refused(format!(
                "{id} is a {}, which {} does not declare",
                entity.type_name, release.label
            )));
        }
        let declared = schema.attribute_names(&name).len();
        let mut values = entity.attributes.clone();
        if values.len() > declared {
            return Err(refused(format!(
                "{id} ({name}) states {} values for {declared} attributes; a value no attribute names cannot be placed",
                values.len()
            )));
        }
        values.resize(declared, Value::Null);
        model.insert(id, Entity::new(name, values));
    }
    let mut report = Report::new();
    ifc_validate::type_check::check(&model, schema, Budget::DEFAULT, &mut report);
    let violations: Vec<String> = report
        .findings()
        .iter()
        .filter(|finding| {
            matches!(
                finding.severity,
                Severity::Error | Severity::EvaluationError
            )
        })
        .take(3)
        .map(ToString::to_string)
        .collect();
    if !violations.is_empty() {
        return Err(refused(format!(
            "values do not conform to {}, so the document was not read as it states: {}",
            release.label,
            violations.join("; ")
        )));
    }
    Ok((release, model))
}

/// Reads an ifcXML document into the IFC model its STEP form parses to.
///
/// For hosts that read the model again themselves (to mesh it): the same
/// reader, and the same refusals, as [`import_ifc_xml_session`].
///
/// # Errors
///
/// Returns [`IfcSessionError::Xml`] when the document cannot be read, or is
/// read into values its schema does not declare or admit, and
/// [`IfcSessionError::UnsupportedSchema`] for a schema the adapter does not
/// read.
pub fn read_ifc_xml(bytes: &[u8]) -> Result<Model, IfcSessionError> {
    read(bytes).map(|(_, model)| model)
}

/// Reads an ifcXML document into an evidence session, as
/// [`crate::import_ifc_session`] reads a STEP file.
///
/// The source is `ifc-xml:<document>`; object identities are the document's
/// entity ids (`i42` is `#42`), so a model written to ifcXML from STEP keeps
/// every identity, and answers every rule as its STEP form does.
///
/// # Errors
///
/// As [`read_ifc_xml`], and as [`crate::import_ifc_session`] once the model
/// is read.
pub fn import_ifc_xml_session(
    document: impl Into<String>,
    bytes: &[u8],
) -> Result<EvidenceSession, IfcSessionError> {
    let source = SourceId::new("ifc-xml", document.into())
        .map_err(|error| IfcSessionError::Identity(error.to_string()))?;
    let (release, model) = read(bytes)?;
    session(&source, release, model, bytes)
}
