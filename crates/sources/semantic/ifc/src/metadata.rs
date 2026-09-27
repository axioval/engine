//! What an IFC file states about itself as a whole: the applications that
//! wrote it and the project it describes.
//!
//! The authoring application is `IfcOwnerHistory.OwningApplication`, an
//! `IfcApplication` whose `ApplicationFullName` names it; the project is
//! `IfcProject.Name`. Every owner history and project of the file is read,
//! so the values are complete: a file without owner histories states no
//! application. A reference that does not lead to an `IfcApplication`, or a
//! name that is not text, leaves the field unread rather than dropping the
//! value, so a `source` selector over it is not evaluated, never a non-match.
//! The schema comes from the snapshot and the file name from the host.

use ifc_model::{Model, Value};

use axioval_engine::SourceMetadata;
use axioval_ir::contract::SourceField;

use crate::release::Release;

/// The metadata the file states: every field it reads exactly.
pub(crate) fn read(release: Release, model: &Model) -> SourceMetadata {
    let mut metadata = SourceMetadata::new();
    if let Some(applications) = applications(release, model) {
        metadata = metadata.with(SourceField::Application, applications);
    }
    if let Some(projects) = projects(release, model) {
        metadata = metadata.with(SourceField::Project, projects);
    }
    metadata
}

fn slot(release: Release, entity: &str, attribute: &str) -> Option<usize> {
    release
        .schema
        .attribute_names(entity)
        .iter()
        .position(|name| *name == attribute)
}

/// Every owner history's owning application, by full name; `None` when one
/// cannot be read exactly.
fn applications(release: Release, model: &Model) -> Option<Vec<String>> {
    let owning = slot(release, "IfcOwnerHistory", "OwningApplication")?;
    let full_name = slot(release, "IfcApplication", "ApplicationFullName")?;
    let mut names = Vec::new();
    for (_, history) in model.iter() {
        if !release.schema.is_a(&history.type_name, "IFCOWNERHISTORY") {
            continue;
        }
        let Some(Value::Ref(application)) = history.attribute(owning) else {
            return None;
        };
        let application = model.get(*application)?;
        if !release
            .schema
            .is_a(&application.type_name, "IFCAPPLICATION")
        {
            return None;
        }
        names.push(text(application.attribute(full_name)?)?);
    }
    Some(names)
}

/// Every project's name; an unset name states none. `None` when a name is
/// not text.
fn projects(release: Release, model: &Model) -> Option<Vec<String>> {
    let name = slot(release, "IfcProject", "Name")?;
    let mut names = Vec::new();
    for (_, project) in model.iter() {
        if !release.schema.is_a(&project.type_name, "IFCPROJECT") {
            continue;
        }
        match project.attribute(name) {
            None | Some(Value::Null) => {}
            Some(value) => names.push(text(value)?),
        }
    }
    Some(names)
}

/// A label's text, bare or wrapped in its defined type.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::Text(text) => Some(text.to_string()),
        Value::Typed { value, .. } => text(value),
        _ => None,
    }
}
