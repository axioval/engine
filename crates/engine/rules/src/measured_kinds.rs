//! What the built-in measured-value providers share: reading the source
//! kinds a measured name selects objects by.

use std::collections::BTreeSet;

use axioval_engine::{PropertyResolutionError, RuleContext, TypeHierarchyServiceHandle};
use axioval_ir::ObjectId;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};

/// The project's objects of the `,`-separated source kinds `key` names,
/// `object` left out: subtypes included where the source declares a type
/// hierarchy, kinds matched exactly where it declares none.
pub(crate) fn objects_of_kinds(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
    object: &ObjectId,
) -> Result<BTreeSet<ObjectId>, PropertyResolutionError> {
    let Some(MeasuredArgument::SourceKind(kinds)) = call.argument(key) else {
        return Err(PropertyResolutionError::InvalidRequest);
    };
    let kinds: Vec<&str> = kinds.split(',').map(str::trim).collect();
    let hierarchy = context.services.get::<TypeHierarchyServiceHandle>();
    let mut found = BTreeSet::new();
    for candidate in context.project.objects() {
        if candidate.id == *object {
            continue;
        }
        let held = candidate.kind();
        for kind in &kinds {
            let matched =
                held.eq_ignore_ascii_case(kind)
                    || match hierarchy {
                        Some(hierarchy) => hierarchy
                            .is_a(&candidate.id.source, held, kind)
                            .map_err(|error| {
                                PropertyResolutionError::Unavailable(format!(
                                    "whether {} is a {kind} is unknown: {error}",
                                    candidate.id
                                ))
                            })?,
                        None => false,
                    };
            if matched {
                found.insert(candidate.id.clone());
                break;
            }
        }
    }
    Ok(found)
}
