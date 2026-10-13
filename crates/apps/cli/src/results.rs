//! `check --model-revision`: the revisions of the checked models, for the
//! results of external calculations a results catalogue holds (#323).
//!
//! A model's revision is the SHA-256 of its file as given (an ifcZIP
//! archive's own bytes), and the revision `--model-revision` states for it,
//! if any. Both are supplied to the plan once the models are read, and only
//! when a ruleset declares a results catalogue; every supplied result
//! computed on another revision, or stating no provenance, is an integrity
//! warning, and reading it is not evaluated.

use std::error::Error;
use std::fs;
use std::path::Path;

use axioval::engine::{
    CATALOGUE_LOCATOR_PREFIX, EvidenceSession, ExecutionPlan, ModelRevision, StaleResult,
};

use crate::CheckArgs;
use crate::digest::IntegrityRecord;

/// Supplies the revision of every checked model to `plan`, where a
/// ruleset declares a results catalogue, and returns an integrity warning
/// per supplied result the run will not read.
///
/// # Errors
///
/// A model file that cannot be read again, and a `--model-revision` that
/// names no checked model, names one twice, or is bare beside several
/// models.
pub(crate) fn supply_revisions(
    plan: &mut ExecutionPlan,
    args: &CheckArgs,
    session: &EvidenceSession,
) -> Result<Vec<IntegrityRecord>, Box<dyn Error>> {
    let names: Vec<&str> = args
        .models
        .iter()
        .map(|model| file_name(&model.path))
        .collect::<Result<_, _>>()?;
    let stated = stated_revisions(&args.model_revisions, &names)?;
    if !plan.reads_calculation_results() {
        return Ok(Vec::new());
    }
    for (model, (name, stated)) in args.models.iter().zip(names.iter().zip(stated)) {
        let shown = model.path.display();
        let bytes = fs::read(&model.path).map_err(|error| format!("{shown}: {error}"))?;
        let revision =
            ModelRevision::of_bytes(&bytes, stated).map_err(|why| format!("{shown}: {why}"))?;
        // The source a model was read as is named by its file name, an
        // archive's by the archive and its member.
        let member = format!("{name}/");
        for snapshot in session.snapshots() {
            let source = snapshot.source();
            if source.document == *name || source.document.starts_with(&member) {
                plan.supply_model_revision(source.clone(), revision.clone())?;
            }
        }
    }
    Ok(plan.stale_results().iter().map(record).collect())
}

/// The file name a model is checked under.
fn file_name(path: &Path) -> Result<&str, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: model path has no UTF-8 file name", path.display()))
}

/// The revision each `--model-revision` states, per model in `documents`'
/// order: `DOCUMENT=REVISION` for the model whose file name is `DOCUMENT`,
/// and a bare `REVISION` for the one model checked.
fn stated_revisions(
    arguments: &[String],
    documents: &[&str],
) -> Result<Vec<Option<String>>, String> {
    let mut stated = vec![None; documents.len()];
    for argument in arguments {
        let named = argument.split_once('=').and_then(|(document, revision)| {
            documents
                .iter()
                .position(|name| *name == document)
                .map(|index| (index, revision))
        });
        let (index, revision) = match named {
            Some(named) => named,
            None if documents.len() == 1 => (0, argument.as_str()),
            None => {
                return Err(format!(
                    "--model-revision {argument}: name the model as DOCUMENT=REVISION, DOCUMENT \
                     one of {}",
                    documents.join(", ")
                ));
            }
        };
        if revision.trim().is_empty() {
            return Err(format!(
                "--model-revision {argument}: the revision is blank"
            ));
        }
        if stated[index].replace(revision.trim().to_owned()).is_some() {
            return Err(format!(
                "--model-revision {argument}: the revision of {} is stated twice",
                documents[index]
            ));
        }
    }
    Ok(stated)
}

/// A result the run will not read, as an integrity warning.
fn record(result: &StaleResult) -> IntegrityRecord {
    let (code, consequence) = if result.stale {
        (
            "calculation-result-stale",
            "the objects reading it are not evaluated until it is computed again on the checked \
             model",
        )
    } else {
        (
            "calculation-result-untraceable",
            "the objects reading it are not evaluated",
        )
    };
    IntegrityRecord {
        code: code.into(),
        severity: "warning".into(),
        message: format!("{}; {consequence}", result.why),
        locator: format!(
            "{CATALOGUE_LOCATOR_PREFIX};id={}#entry={}",
            result.catalogue, result.key
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::stated_revisions;

    #[test]
    fn a_revision_names_its_model_or_the_one_model_checked() {
        let args = |values: &[&str]| {
            values
                .iter()
                .map(|&value| value.to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            stated_revisions(&args(&["rev-3"]), &["m.ifc"]),
            Ok(vec![Some("rev-3".into())])
        );
        // A revision holding `=` is whole unless it names a model.
        assert_eq!(
            stated_revisions(&args(&["a=b"]), &["m.ifc"]),
            Ok(vec![Some("a=b".into())])
        );
        assert_eq!(
            stated_revisions(&args(&["s.ifc=rev-2"]), &["a.ifc", "s.ifc"]),
            Ok(vec![None, Some("rev-2".into())])
        );
        assert!(stated_revisions(&args(&["rev-2"]), &["a.ifc", "s.ifc"]).is_err());
        assert!(stated_revisions(&args(&["m.ifc= "]), &["m.ifc"]).is_err());
        assert!(stated_revisions(&args(&["r1", "r2"]), &["m.ifc"]).is_err());
    }
}
