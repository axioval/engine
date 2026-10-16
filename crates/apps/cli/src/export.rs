//! `export --profile`: a ruleset written as another format through its
//! export profile, every loss listed. `ids export` is this command with the
//! `ids` profile.

use std::{error::Error, path::PathBuf};

use axioval::ir::RuleSetPackage;
use axioval_export::{ExportProfile, LossKind};
use axioval_ids::IdsProfile;
use clap::Args;

/// The profiles this frontend offers, by id.
fn profiles() -> Vec<Box<dyn ExportProfile>> {
    vec![Box::new(IdsProfile)]
}

#[derive(Args)]
pub(crate) struct ProfileExportArgs {
    /// The export profile, by id: `ids` (a buildingSMART IDS 1.0 document).
    #[arg(long, value_name = "ID")]
    profile: String,
    #[command(flatten)]
    export: ExportArgs,
}

impl ProfileExportArgs {
    /// Runs the export; see [`export_command`].
    pub(crate) fn run(&self) -> Result<bool, Box<dyn Error>> {
        export_command(&self.profile, &self.export)
    }
}

#[derive(Args)]
pub(crate) struct ExportArgs {
    /// A definition package the ruleset uses (JSON, or an MCS `definitions`
    /// archive); repeat for several. A ruleset archive carries its own.
    #[arg(long = "definitions", value_name = "FILE")]
    definitions: Vec<PathBuf>,
    /// The ruleset to export: JSON, or an MCS `ruleset` archive.
    #[arg(long, value_name = "FILE")]
    ruleset: PathBuf,
    /// Where to write the exported document.
    #[arg(long, value_name = "FILE")]
    out: PathBuf,
}

/// Writes the artifact of the profile `id`, and lists every loss on
/// stderr. `Ok(false)` when anything was lost; nothing is written when no
/// rule is exported.
pub(crate) fn export_command(id: &str, args: &ExportArgs) -> Result<bool, Box<dyn Error>> {
    let profiles = profiles();
    let Some(profile) = profiles.iter().find(|profile| profile.id() == id) else {
        let known: Vec<&str> = profiles.iter().map(|profile| profile.id()).collect();
        return Err(format!(
            "unknown export profile {id:?}; known profiles: {}",
            known.join(", ")
        )
        .into());
    };
    let (definitions, rulesets) =
        crate::packages(&args.definitions, std::slice::from_ref(&args.ruleset))?;
    let [ruleset]: [RuleSetPackage; 1] = rulesets
        .try_into()
        .map_err(|_| "expected exactly one ruleset")?;
    let outcome = profile.export(&definitions, &ruleset);
    let shown = args.ruleset.display();
    for loss in &outcome.losses {
        match loss.kind {
            LossKind::Refused => eprintln!(
                "{id}: {shown}: rule {} is not exported: {}",
                loss.path, loss.reason
            ),
            LossKind::Degraded => {
                eprintln!("{id}: {shown}: {} is degraded: {}", loss.path, loss.reason);
            }
        }
    }
    let refused = outcome.refused().count();
    let Some(artifact) = &outcome.artifact else {
        return Err(format!(
            "{shown}: no rule can be exported as {}; {refused} rule(s) not exported",
            profile.format()
        )
        .into());
    };
    crate::write(&args.out, artifact)?;
    let contents = outcome
        .contents
        .as_ref()
        .map(|contents| format!(" as {contents}"))
        .unwrap_or_default();
    println!(
        "exported {} rule(s){contents}; {refused} rule(s) not exported",
        outcome.exported.len()
    );
    Ok(outcome.is_complete())
}
