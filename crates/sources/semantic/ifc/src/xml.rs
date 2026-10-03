//! ifcXML: the XML serialization of IFC, read into the STEP reader's model.
//!
//! The document is parsed by `ifc-xml`'s `XmlCodec` into the same
//! `ifc_model::Model` the STEP codec produces, then bound by the same
//! session path as a STEP file ([`crate::ifc::session`]), so every service
//! answers an ifcXML model exactly as its STEP form.
//!
//! Two layouts are read, told apart by the root element alone:
//!
//! - the buildingSMART configuration the release XSD declares, for IFC4
//!   ADD2 TC1 and IFC4X3 ADD2: a root in one of that release's ifcXML
//!   namespaces, without a `schema` attribute (which the XSD does not
//!   declare). The codec's XSD reader (`XmlCodec::xsd`) is always
//!   schema-strict; it numbers entities in document order.
//! - the codec's own layout: a root stating its release in a `schema`
//!   attribute, attributes named by that release. It is read strictly
//!   (`SchemaReading::Strict`, the codec's default with a schema): every
//!   value is typed from its declaration, and an entity or attribute the
//!   release does not declare, a value it does not admit and a dangling or
//!   wrongly typed reference are refused by the codec, never placed by
//!   guess.
//!
//! A root that is neither, a release no reader here binds, and every codec
//! refusal are refused ([`IfcSessionError::Xml`] or
//! [`IfcSessionError::UnsupportedSchema`]), never read in part. Before the
//! codec parses, no start tag may hold more than [`MAX_ATTRIBUTES`]
//! attributes or [`MAX_NAMESPACES`] namespace declarations, so a hostile
//! document's cost stays bounded. Entity names are stored upper case, as
//! STEP writes them.

use std::sync::{Arc, OnceLock};

use axioval_engine::EvidenceSession;
use axioval_ir::SourceId;
use ifc_model::{Entity, Model};
use ifc_schema::{Schema, SchemaVersion};
use ifc_xml::{XmlCodec, XmlProfile};

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

/// How a document lays out its entities, and the release it is read with.
#[derive(Clone, Copy, Debug)]
enum Layout {
    /// The codec's own layout, under the release the root's `schema` names.
    Native(Release),
    /// The buildingSMART XSD configuration of one release.
    Xsd(Release, XmlProfile),
}

/// The XSD profile whose release namespaces include `namespace`.
fn profile_of(namespace: &str) -> Option<XmlProfile> {
    [XmlProfile::Ifc4Add2Tc1, XmlProfile::Ifc4x3Add2]
        .into_iter()
        .find(|profile| profile.namespaces().contains(&namespace))
}

/// The layout and release the root element declares.
fn layout(bytes: &[u8]) -> Result<Layout, IfcSessionError> {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;

    let mut reader = quick_xml::NsReader::from_reader(bytes);
    let mut buffer = Vec::new();
    let (namespace, root) = loop {
        match reader
            .read_resolved_event_into(&mut buffer)
            .map_err(|error| refused(error.to_string()))?
        {
            (namespace, Event::Start(element) | Event::Empty(element)) => {
                let namespace = match namespace {
                    ResolveResult::Bound(namespace) => Some(namespace.as_ref().to_owned()),
                    _ => None,
                };
                break (namespace, element.into_owned());
            }
            (_, Event::Eof) => return Err(refused("the document has no root element")),
            _ => {}
        }
        buffer.clear();
    };
    if root.local_name().as_ref() != "ifcXML" {
        return Err(refused(format!(
            "the root element is `{}`, not `ifcXML`",
            root.name().as_ref()
        )));
    }
    let mut schema = None;
    for attribute in root.attributes() {
        let attribute = attribute.map_err(|error| refused(error.to_string()))?;
        if attribute.key.as_ref() == "schema" {
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| refused(error.to_string()))?;
            schema = Some(value.into_owned());
        }
    }
    if let Some(schema) = schema {
        let schemas = vec![schema];
        return Release::from_header(&schemas)
            .map(Layout::Native)
            .ok_or(IfcSessionError::UnsupportedSchema(schemas));
    }
    let Some(profile) = namespace.as_deref().and_then(profile_of) else {
        let namespace = namespace.as_deref().map_or_else(
            || "no namespace".to_owned(),
            |namespace| format!("`{namespace}`"),
        );
        return Err(refused(format!(
            "the root names no `schema` and is in {namespace}, which is neither an IFC4 ADD2 TC1 \
             nor the IFC4X3 ADD2 ifcXML namespace"
        )));
    };
    let token = match profile.version() {
        SchemaVersion::Ifc4 => "IFC4",
        SchemaVersion::Ifc4x3 => "IFC4X3",
        _ => profile.schema_token(),
    };
    let schemas = vec![token.to_owned()];
    let release =
        Release::from_header(&schemas).ok_or(IfcSessionError::UnsupportedSchema(schemas))?;
    Ok(Layout::Xsd(release, profile))
}

/// Reads an ifcXML document into the model its STEP form parses to, and
/// the release it declares.
fn read(bytes: &[u8]) -> Result<(Release, Model), IfcSessionError> {
    bounded(bytes)?;
    let (release, codec) = match layout(bytes)? {
        Layout::Native(release) => (release, XmlCodec::with_schema(shared(release))),
        Layout::Xsd(release, profile) => (release, XmlCodec::xsd(shared(release), profile)),
    };
    let read = ifc_xml::reader::read(&codec, bytes).map_err(|error| refused(error.to_string()))?;
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
        // The strict readers lay every entity out by its declaration; one
        // that is not is refused, never padded or cut.
        if schema.entity(&name).is_none()
            || entity.attributes.len() != schema.attribute_names(&name).len()
        {
            return Err(refused(format!(
                "{id} ({name}) is not laid out as {} declares it",
                release.label
            )));
        }
        model.insert(id, Entity::new(name, entity.attributes.clone()));
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
/// Returns [`IfcSessionError::Xml`] when the document is in neither layout,
/// cannot be read, or states a name or value its release does not declare
/// or admit, and [`IfcSessionError::UnsupportedSchema`] for a release the
/// adapter does not read.
pub fn read_ifc_xml(bytes: &[u8]) -> Result<Model, IfcSessionError> {
    read(bytes).map(|(_, model)| model)
}

/// Reads an ifcXML document into an evidence session, as
/// [`crate::import_ifc_session`] reads a STEP file.
///
/// The source is `ifc-xml:<document>`. In the codec's own layout object
/// identities are the document's entity ids (`i42` is `#42`), so a model
/// written to ifcXML from STEP keeps every identity; in the XSD layout they
/// are the entities' places in document order (`#1`, `#2`, ...), as the
/// codec numbers them. Either answers every rule as its STEP form does.
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
    session(&source, release, model, Vec::new(), bytes)
}
