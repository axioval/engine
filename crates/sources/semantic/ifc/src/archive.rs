//! ifcZIP: a zip archive holding one IFC model.
//!
//! The archive is a container, never a source of its own: exactly one model
//! member is read with the same session as a plain STEP file, and the source
//! is named by the archive and that member together.

use std::io::{Cursor, Read};
use std::path::{Component, Path};

use axioval_engine::EvidenceSession;
use thiserror::Error;

use crate::ifc::{IfcSessionError, import_ifc_session};

/// The file extension of an ifcZIP archive, without the dot.
pub const IFC_ZIP_EXTENSION: &str = "ifczip";

/// Local file header signature every zip archive starts with.
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// Why an ifcZIP archive was refused.
#[derive(Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum IfcZipError {
    /// The bytes are not a readable zip archive.
    #[error("not a readable ifcZIP archive: {0}")]
    Archive(String),
    /// A member path that is absolute, climbs out of the archive (`..`),
    /// holds a backslash or a NUL, or is a symbolic link.
    #[error("ifcZIP member {0:?} has an unsafe path")]
    UnsafePath(String),
    /// No `.ifc` member.
    #[error("the ifcZIP archive holds no .ifc model")]
    NoModel,
    /// More than one model member; which one is meant is not stated.
    #[error("the ifcZIP archive holds {} models ({}); exactly one is read", .0.len(), .0.join(", "))]
    SeveralModels(Vec<String>),
    /// The one model member is IFC-XML, which is not read yet.
    #[error("ifcZIP member {0:?} is IFC-XML, which is not supported; only STEP (.ifc) is read")]
    UnsupportedModel(String),
    /// The model member could not be decompressed.
    #[error("ifcZIP member {member:?} cannot be read: {reason}")]
    Member {
        /// The member's path in the archive.
        member: String,
        /// Why.
        reason: String,
    },
    /// The member was read, and the IFC session refused it.
    #[error("{member}: {error}")]
    Session {
        /// The member's path in the archive.
        member: String,
        /// The session's refusal.
        error: IfcSessionError,
    },
}

/// The one model member of an ifcZIP archive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IfcZipMember {
    name: String,
    bytes: Vec<u8>,
}

impl IfcZipMember {
    /// The member's path in the archive, as stored (`/`-separated).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The member's decompressed STEP bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the member into its bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// The source document of this member read from the archive named
    /// `archive`: `archive/member` (`model.ifczip/model.ifc`), so two
    /// archives holding a member of one name stay two sources.
    #[must_use]
    pub fn document(&self, archive: &str) -> String {
        format!("{archive}/{}", self.name)
    }
}

/// Whether `bytes` start as a zip archive does. A STEP file never does, so
/// this tells an ifcZIP from a plain IFC file whatever its name.
#[must_use]
pub fn is_ifc_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(ZIP_MAGIC)
}

/// Reads the one model member of an ifcZIP archive.
///
/// Every member path is checked first: an absolute path, a `..` component,
/// a backslash, a NUL or a symbolic link refuses the whole archive. A model
/// member is one ending in `.ifc` or `.ifcxml`, ignoring case; any other
/// member (a readme, a thumbnail) is ignored. Exactly one model member is
/// read.
///
/// # Errors
///
/// Returns [`IfcZipError`] when the archive is unreadable, a path is unsafe,
/// it holds no model or several, its model is IFC-XML, or the model cannot
/// be decompressed.
pub fn read_ifc_zip(bytes: &[u8]) -> Result<IfcZipMember, IfcZipError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| IfcZipError::Archive(error.to_string()))?;
    let mut models = Vec::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index_raw(index)
            .map_err(|error| IfcZipError::Archive(error.to_string()))?;
        let name = file.name().to_owned();
        if !is_safe(&name) || file.enclosed_name().is_none() || file.is_symlink() {
            return Err(IfcZipError::UnsafePath(name));
        }
        if file.is_dir() {
            continue;
        }
        if let Some(kind) = model_kind(&name) {
            models.push((index, name, kind));
        }
    }
    let (index, name, kind) = match models.len() {
        0 => return Err(IfcZipError::NoModel),
        1 => models.remove(0),
        _ => {
            return Err(IfcZipError::SeveralModels(
                models.into_iter().map(|(_, name, _)| name).collect(),
            ));
        }
    };
    if kind == ModelKind::Xml {
        return Err(IfcZipError::UnsupportedModel(name));
    }
    let member = |reason: String| IfcZipError::Member {
        member: name.clone(),
        reason,
    };
    let mut file = archive
        .by_index(index)
        .map_err(|error| member(error.to_string()))?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)
        .map_err(|error| member(error.to_string()))?;
    drop(file);
    Ok(IfcZipMember {
        name,
        bytes: content,
    })
}

/// Reads the one model of an ifcZIP archive named `archive` into a session,
/// as [`import_ifc_session`] reads a plain file. The source document is
/// [`IfcZipMember::document`]; the fingerprint is the member's, so a zipped
/// copy of a model answers every rule as the plain file does.
///
/// # Errors
///
/// Returns [`IfcZipError`] as [`read_ifc_zip`] does, and
/// [`IfcZipError::Session`] when the session refuses the member.
pub fn import_ifc_zip_session(archive: &str, bytes: &[u8]) -> Result<EvidenceSession, IfcZipError> {
    let member = read_ifc_zip(bytes)?;
    import_ifc_session(member.document(archive), member.bytes()).map_err(|error| {
        IfcZipError::Session {
            member: member.name().to_owned(),
            error,
        }
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelKind {
    Step,
    Xml,
}

fn model_kind(name: &str) -> Option<ModelKind> {
    let extension = Path::new(name).extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("ifc") {
        Some(ModelKind::Step)
    } else if extension.eq_ignore_ascii_case("ifcxml") {
        Some(ModelKind::Xml)
    } else {
        None
    }
}

/// A relative, `/`-separated path that never leaves the archive.
fn is_safe(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\\', '\0'])
        && Path::new(name)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}
