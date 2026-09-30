//! IDS 1.0 XML, written by the `openbim-ids` writer.
//!
//! Documents are written by [`openbim_ids::to_string`], which checks the
//! whole model against `ids.xsd` 1.0 before writing and refuses what it
//! cannot state with a [`WriteError`] naming the part of the model.
//!
//! What remains here is what `openbim-ids` does not offer: a single
//! `<specification>` outside a document. A folder
//! [`translate()`](crate::translate) writes keeps its specification in the
//! `ids:specification` annotation as such a fragment, and the export reads
//! it back. [`specification`] cuts the fragment out of a one-specification
//! document the upstream writer wrote; [`read_specification`] wraps a
//! fragment into a document for the upstream reader, so it reads fragments
//! written by this writer and by the earlier stand-in writer alike.

use openbim_ids::{Ids, Info, Specification, WriteError};

/// The IDS namespace, and the XML Schema one restrictions are written in.
const NAMESPACES: &str =
    r#"xmlns="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema""#;

/// The title of the document a fragment is written in or read from.
const ANNOTATION_TITLE: &str = "annotation";

/// A whole IDS 1.0 document.
pub(crate) fn document(
    info: &Info,
    specifications: &[&Specification],
) -> Result<String, WriteError> {
    let mut ids = Ids::new(info.clone());
    ids.specifications.extend(
        specifications
            .iter()
            .map(|&specification| specification.clone()),
    );
    openbim_ids::to_string(&ids)
}

/// Why one specification cannot be written, located within it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Unwritable {
    /// The path to the refused part, relative to the specification (empty
    /// for the specification itself).
    pub(crate) location: String,
    /// What the writer refused.
    pub(crate) why: String,
}

impl Unwritable {
    fn new(error: &WriteError) -> Self {
        let location = error.location();
        let location = location
            .strip_prefix("specifications[0]")
            .map_or(location, |rest| rest.trim_start_matches('/'));
        Self {
            location: location.to_owned(),
            why: error.kind().to_string(),
        }
    }
}

/// One `<specification>` element in the IDS namespace without declaring
/// it: the form an `ids:specification` annotation keeps, read back inside a
/// document's `<specifications>` by [`read_specification`].
pub(crate) fn specification(specification: &Specification) -> Result<String, Unwritable> {
    const CLOSE: &str = "</specification>";
    let mut ids = Ids::new(Info::new(ANNOTATION_TITLE));
    ids.specifications.push(specification.clone());
    let document = openbim_ids::to_string(&ids).map_err(|error| Unwritable::new(&error))?;
    // Markup in text and attributes is escaped, so the only literal tags
    // are the element's own.
    match (document.find("<specification "), document.rfind(CLOSE)) {
        (Some(start), Some(end)) if start < end => {
            Ok(document[start..end + CLOSE.len()].to_owned())
        }
        _ => Err(Unwritable {
            location: String::new(),
            why: "the written document holds no <specification> element".to_owned(),
        }),
    }
}

/// Reads a `<specification>` element [`specification`] wrote, or the
/// earlier stand-in writer did.
pub(crate) fn read_specification(fragment: &str) -> Result<Specification, String> {
    let wrapped = format!(
        "<ids {NAMESPACES}><info><title>{ANNOTATION_TITLE}</title></info><specifications>{fragment}</specifications></ids>"
    );
    let mut ids = openbim_ids::from_str(&wrapped).map_err(|error| error.to_string())?;
    match ids.specifications.len() {
        1 => Ok(ids.specifications.remove(0)),
        count => Err(format!("holds {count} specifications, not one")),
    }
}
