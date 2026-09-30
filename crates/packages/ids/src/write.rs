//! IDS 1.0 XML, written from the `openbim-ids` model.
//!
//! `openbim-ids` reads IDS but does not write it yet; openbimrs/ids#10 asks
//! for a writer. Until it is released, this small writer is the one place
//! IDS XML is produced: [`document`] and [`specification`] are its only
//! entry points, so switching to `openbim_ids::write` changes only them.
//!
//! What it writes is schema-valid IDS 1.0 for every model the reader
//! produces: elements in the order `ids.xsd` requires, applicability facets
//! sorted into the schema's sequence (entity, partOf, classification,
//! attribute, property, material), every attribute a facet may carry in its
//! position, and every text escaped so it reads back unchanged, surrounding
//! whitespace and line breaks included.

use openbim_ids::{
    Applicability, Facet, Info, Requirement, Requirements, Restriction, Specification, Value,
};

/// The IDS namespace, and the XML Schema one restrictions are written in.
const NAMESPACES: &str =
    r#"xmlns="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema""#;

/// The schema location every written document declares.
const SCHEMA_LOCATION: &str = r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://standards.buildingsmart.org/IDS http://standards.buildingsmart.org/IDS/1.0/ids.xsd""#;

/// A whole IDS 1.0 document, indented.
pub(crate) fn document(info: &Info, specifications: &[&Specification]) -> String {
    let mut xml = Xml::new(true);
    xml.out
        .push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.open_raw("ids", &format!("{NAMESPACES} {SCHEMA_LOCATION}"));
    xml.info(info);
    xml.open("specifications", &[]);
    for specification in specifications {
        xml.specification(specification);
    }
    xml.close("specifications");
    xml.close("ids");
    xml.out
}

/// One `<specification>` element on a single line, in the IDS namespace
/// without declaring it: the form an `ids:specification` annotation keeps,
/// read back inside a document's `<specifications>` by [`read_specification`].
pub(crate) fn specification(specification: &Specification) -> String {
    let mut xml = Xml::new(false);
    xml.specification(specification);
    xml.out
}

/// Reads a `<specification>` element [`specification`] wrote.
pub(crate) fn read_specification(fragment: &str) -> Result<Specification, String> {
    let wrapped = format!(
        "<ids {NAMESPACES}><info><title>annotation</title></info><specifications>{fragment}</specifications></ids>"
    );
    let mut ids = openbim_ids::from_str(&wrapped).map_err(|error| error.to_string())?;
    match ids.specifications.len() {
        1 => Ok(ids.specifications.remove(0)),
        count => Err(format!("holds {count} specifications, not one")),
    }
}

/// An XML text under construction.
struct Xml {
    out: String,
    pretty: bool,
    depth: usize,
}

impl Xml {
    fn new(pretty: bool) -> Self {
        Self {
            out: String::new(),
            pretty,
            depth: 0,
        }
    }

    fn indent(&mut self) {
        if self.pretty {
            for _ in 0..self.depth {
                self.out.push('\t');
            }
        }
    }

    fn newline(&mut self) {
        if self.pretty {
            self.out.push('\n');
        }
    }

    fn start(&mut self, name: &str, attributes: &[(&str, &str)]) {
        self.indent();
        self.out.push('<');
        self.out.push_str(name);
        for (attribute, value) in attributes {
            self.out.push(' ');
            self.out.push_str(attribute);
            self.out.push_str("=\"");
            escape(&mut self.out, value, true);
            self.out.push('"');
        }
    }

    fn open(&mut self, name: &str, attributes: &[(&str, &str)]) {
        self.start(name, attributes);
        self.out.push('>');
        self.newline();
        self.depth += 1;
    }

    fn open_raw(&mut self, name: &str, raw: &str) {
        self.indent();
        self.out.push('<');
        self.out.push_str(name);
        self.out.push(' ');
        self.out.push_str(raw);
        self.out.push('>');
        self.newline();
        self.depth += 1;
    }

    fn close(&mut self, name: &str) {
        self.depth -= 1;
        self.indent();
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push('>');
        self.newline();
    }

    fn empty(&mut self, name: &str, attributes: &[(&str, &str)]) {
        self.start(name, attributes);
        self.out.push_str("/>");
        self.newline();
    }

    fn text(&mut self, name: &str, text: &str) {
        self.start(name, &[]);
        self.out.push('>');
        escape(&mut self.out, text, false);
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push('>');
        self.newline();
    }

    fn info(&mut self, info: &Info) {
        self.open("info", &[]);
        self.text("title", &info.title);
        for (name, value) in [
            ("copyright", &info.copyright),
            ("version", &info.version),
            ("description", &info.description),
            ("author", &info.author),
            ("date", &info.date),
            ("purpose", &info.purpose),
            ("milestone", &info.milestone),
        ] {
            if let Some(value) = value {
                self.text(name, value);
            }
        }
        self.close("info");
    }

    fn specification(&mut self, specification: &Specification) {
        let releases = specification
            .ifc_versions
            .iter()
            .map(|release| release.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let mut attributes = vec![("name", specification.name.as_str())];
        attributes.push(("ifcVersion", releases.as_str()));
        for (name, value) in [
            ("identifier", &specification.identifier),
            ("description", &specification.description),
            ("instructions", &specification.instructions),
        ] {
            if let Some(value) = value {
                attributes.push((name, value));
            }
        }
        self.open("specification", &attributes);
        self.applicability(&specification.applicability);
        if let Some(requirements) = &specification.requirements {
            self.requirements(requirements);
        }
        self.close("specification");
    }

    fn applicability(&mut self, applicability: &Applicability) {
        let minimum = applicability.min_occurs.to_string();
        let maximum = applicability
            .max_occurs
            .map_or_else(|| "unbounded".to_owned(), |maximum| maximum.to_string());
        let attributes = [
            ("minOccurs", minimum.as_str()),
            ("maxOccurs", maximum.as_str()),
        ];
        if applicability.facets.is_empty() {
            self.empty("applicability", &attributes);
            return;
        }
        self.open("applicability", &attributes);
        // The schema's sequence; a document the reader accepted is in it
        // already, so the sort keeps its order.
        let mut facets: Vec<&Facet> = applicability.facets.iter().collect();
        facets.sort_by_key(|facet| sequence(facet));
        for facet in facets {
            self.facet(facet, &[]);
        }
        self.close("applicability");
    }

    fn requirements(&mut self, requirements: &Requirements) {
        let mut attributes = Vec::new();
        if let Some(description) = &requirements.description {
            attributes.push(("description", description.as_str()));
        }
        if requirements.facets.is_empty() {
            self.empty("requirements", &attributes);
            return;
        }
        self.open("requirements", &attributes);
        for requirement in &requirements.facets {
            let attributes = requirement_attributes(requirement);
            self.facet(&requirement.facet, &attributes);
        }
        self.close("requirements");
    }

    fn facet(&mut self, facet: &Facet, attributes: &[(&str, &str)]) {
        match facet {
            Facet::Entity(entity) => {
                self.open("entity", attributes);
                self.entity_content(entity);
                self.close("entity");
            }
            Facet::PartOf(part_of) => {
                let mut attributes = attributes.to_vec();
                if let Some(relation) = part_of.relation {
                    attributes.insert(0, ("relation", relation.as_str()));
                }
                self.open("partOf", &attributes);
                self.open("entity", &[]);
                self.entity_content(&part_of.entity);
                self.close("entity");
                self.close("partOf");
            }
            Facet::Classification(classification) => {
                self.open("classification", attributes);
                if let Some(value) = &classification.value {
                    self.value("value", value);
                }
                self.value("system", &classification.system);
                self.close("classification");
            }
            Facet::Attribute(attribute) => {
                self.open("attribute", attributes);
                self.value("name", &attribute.name);
                if let Some(value) = &attribute.value {
                    self.value("value", value);
                }
                self.close("attribute");
            }
            Facet::Property(property) => {
                let mut attributes = attributes.to_vec();
                if let Some(data_type) = &property.data_type {
                    attributes.insert(0, ("dataType", data_type.as_str()));
                }
                self.open("property", &attributes);
                self.value("propertySet", &property.property_set);
                self.value("baseName", &property.base_name);
                if let Some(value) = &property.value {
                    self.value("value", value);
                }
                self.close("property");
            }
            Facet::Material(material) => match &material.value {
                Some(value) => {
                    self.open("material", attributes);
                    self.value("value", value);
                    self.close("material");
                }
                None => self.empty("material", attributes),
            },
        }
    }

    fn entity_content(&mut self, entity: &openbim_ids::Entity) {
        self.value("name", &entity.name);
        if let Some(predefined) = &entity.predefined_type {
            self.value("predefinedType", predefined);
        }
    }

    fn value(&mut self, name: &str, value: &Value) {
        self.open(name, &[]);
        match value {
            Value::Simple(literal) => self.text("simpleValue", literal),
            Value::Restriction(restriction) => self.restriction(restriction),
        }
        self.close(name);
    }

    fn restriction(&mut self, restriction: &Restriction) {
        let base = format!("xs:{}", restriction.base);
        let mut facets: Vec<(&str, String)> = Vec::new();
        for value in &restriction.enumeration {
            facets.push(("xs:enumeration", value.clone()));
        }
        for pattern in &restriction.patterns {
            facets.push(("xs:pattern", pattern.clone()));
        }
        for (name, bound) in [
            ("xs:minInclusive", &restriction.min_inclusive),
            ("xs:maxInclusive", &restriction.max_inclusive),
            ("xs:minExclusive", &restriction.min_exclusive),
            ("xs:maxExclusive", &restriction.max_exclusive),
        ] {
            if let Some(bound) = bound {
                facets.push((name, bound.clone()));
            }
        }
        for (name, count) in [
            ("xs:length", restriction.length),
            ("xs:minLength", restriction.min_length),
            ("xs:maxLength", restriction.max_length),
            ("xs:totalDigits", restriction.total_digits),
            ("xs:fractionDigits", restriction.fraction_digits),
        ] {
            if let Some(count) = count {
                facets.push((name, count.to_string()));
            }
        }
        if facets.is_empty() {
            self.empty("xs:restriction", &[("base", &base)]);
            return;
        }
        self.open("xs:restriction", &[("base", &base)]);
        for (name, value) in &facets {
            self.empty(name, &[("value", value)]);
        }
        self.close("xs:restriction");
    }
}

/// The attributes a requirement facet carries, as `ids.xsd` allows them
/// per facet: an entity takes only instructions, a part-of no `uri`, an
/// attribute no `uri` either.
fn requirement_attributes(requirement: &Requirement) -> Vec<(&'static str, &str)> {
    let mut attributes = Vec::new();
    let (cardinality, uri) = match requirement.facet {
        Facet::Entity(_) => (false, false),
        Facet::PartOf(_) | Facet::Attribute(_) => (true, false),
        Facet::Classification(_) | Facet::Property(_) | Facet::Material(_) => (true, true),
    };
    if uri && let Some(uri) = &requirement.uri {
        attributes.push(("uri", uri.as_str()));
    }
    if cardinality {
        attributes.push(("cardinality", requirement.occurrence.as_cardinality()));
    }
    if let Some(instructions) = &requirement.instructions {
        attributes.push(("instructions", instructions.as_str()));
    }
    attributes
}

/// A facet's place in the applicability sequence `ids.xsd` declares.
fn sequence(facet: &Facet) -> u8 {
    match facet {
        Facet::Entity(_) => 0,
        Facet::PartOf(_) => 1,
        Facet::Classification(_) => 2,
        Facet::Attribute(_) => 3,
        Facet::Property(_) => 4,
        Facet::Material(_) => 5,
    }
}

/// Appends `text` escaped for XML content, or for a double-quoted attribute
/// value, so it reads back exactly: line breaks and tabs in attributes and
/// carriage returns anywhere are character references, which XML's
/// normalization leaves alone.
fn escape(out: &mut String, text: &str, attribute: bool) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\n' if attribute => out.push_str("&#10;"),
            '\t' if attribute => out.push_str("&#9;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}
