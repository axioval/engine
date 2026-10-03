//! Strict IFC STEP reading, with one stated tolerance.
//!
//! A STEP file is read under `ParseOptions::strict()`: a malformed record
//! refuses the whole file, as it always has. The one exception is a REAL
//! written without the decimal point ISO 10303-21 requires (`1E-05`), which
//! common exporters write and whose meaning is not in doubt:
//! `accept_real_without_point` reads it as the real it spells. Nothing is
//! dropped, so the model is complete, but the file is not conforming, and
//! every such token is kept here and reported as an integrity warning
//! ([`crate::REAL_WITHOUT_DECIMAL_POINT`]). It is never accepted silently.
//! A token that is not a number (`1E`, `1EE2`) is still a parse error.

use std::ops::Range;

use ifc_model::{Codec, Model};
use ifc_step::{ParseOptions, StepCodec};

use crate::ifc::IfcSessionError;

/// How `ifc-step` words the diagnostic for a REAL read without its decimal
/// point. Under the options [`read`] uses it is the only diagnostic the
/// reader can raise; anything else still refuses the model.
const REAL_WITHOUT_POINT: &str = "read REAL `";

/// A REAL the file wrote without its decimal point, read as the real it
/// spells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RealWithoutPoint {
    /// The bytes of the token as written.
    pub(crate) bytes: Range<usize>,
    /// The reader's description, quoting the token.
    pub(crate) detail: String,
}

/// Reads STEP `bytes` strictly, accepting only reals without a decimal
/// point, each of which is returned beside the model.
pub(crate) fn read(bytes: &[u8]) -> Result<(Model, Vec<RealWithoutPoint>), IfcSessionError> {
    let model = StepCodec::with_options(ParseOptions::strict().accept_real_without_point(true))
        .read_bytes(bytes)
        .map_err(|error| IfcSessionError::Parse(error.to_string()))?;
    if model.diagnostics().is_empty() {
        return Ok((model, Vec::new()));
    }
    let reals = model
        .diagnostics()
        .iter()
        .map(|diagnostic| match diagnostic.byte_range() {
            Some(range) if diagnostic.detail().starts_with(REAL_WITHOUT_POINT) => {
                Some(RealWithoutPoint {
                    bytes: range.clone(),
                    detail: diagnostic.detail().to_owned(),
                })
            }
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(IfcSessionError::IncompleteModel {
            diagnostics: model.diagnostics().len(),
        })?;
    // Every record was read, so the model is complete; only its diagnostics
    // say otherwise, and downstream readers refuse a model carrying any. The
    // same entities under the same ids, without them, are the model the file
    // states, and the warnings travel beside it.
    let mut complete = Model::new();
    *complete.header_mut() = model.header().clone();
    for (id, entity) in model.iter() {
        complete.insert(id, entity.clone());
    }
    Ok((complete, reals))
}

/// Reads an IFC STEP file into the model an evidence session is built from,
/// with the same reader and refusals as [`crate::import_ifc_session`].
///
/// A REAL written without its decimal point (`1E-05`) is read as the real it
/// spells; the session reports each one as a
/// [`crate::REAL_WITHOUT_DECIMAL_POINT`] integrity warning.
///
/// # Errors
///
/// Returns [`IfcSessionError::Parse`] when the file is not strictly
/// readable STEP, a malformed record included.
pub fn read_ifc_step(bytes: &[u8]) -> Result<Model, IfcSessionError> {
    read(bytes).map(|(model, _)| model)
}
