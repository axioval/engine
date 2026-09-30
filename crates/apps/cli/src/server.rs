//! `axioval bcf push` and `axioval bcf pull`: a saved result's findings to
//! a BCF API 3.0 server, and the server's review state back as decisions.

use std::error::Error;
use std::path::PathBuf;

use axioval::{
    bcf, bcf_api,
    ir::{Decisions, Object, ObjectId, Project, Report},
};
use clap::{Args, Subcommand};

use crate::digest::CheckOutput;

/// Environment variable holding a bearer token for the server.
pub const TOKEN_VARIABLE: &str = "AXIOVAL_BCF_TOKEN";
/// Environment variable holding the `OAuth2` client secret.
pub const SECRET_VARIABLE: &str = "AXIOVAL_BCF_CLIENT_SECRET";

#[derive(Subcommand)]
pub enum BcfCommand {
    /// Create a saved result's topics on a BCF API 3.0 server, or update
    /// the ones it has.
    ///
    /// Topics are mapped as `check --bcf` maps them, under the findings'
    /// identities, so a second push updates rather than duplicates. Review
    /// state the result did not decide is kept as the server has it. Exit
    /// status: 0 pushed, 1 failed.
    Push(PushArgs),
    /// Read the review state of a server's topics into a decisions file.
    ///
    /// Topics are read as `check --decisions-from` reads an archive; each
    /// decision is recorded with its finding's basis from the result,
    /// replacing an earlier one about the same finding. `check --decisions`
    /// then carries them over. Exit status: 0 pulled, 1 failed.
    Pull(PullArgs),
}

/// Where the server is and how to sign in.
///
/// Credentials are never arguments: a bearer token comes from
/// `AXIOVAL_BCF_TOKEN`, a client secret from `AXIOVAL_BCF_CLIENT_SECRET`,
/// and neither is written anywhere.
#[derive(Args)]
pub struct ServerArgs {
    /// The server's root URL, before `/bcf`. `https` needs a build with the
    /// `tls` feature.
    #[arg(long, value_name = "URL")]
    server: String,
    /// The server's project id.
    #[arg(long, value_name = "ID")]
    project: String,
    /// `OAuth2` client id: with `AXIOVAL_BCF_CLIENT_SECRET` set, signs in by
    /// client credentials; with `--device-authorization-url`, by the device
    /// flow, printing where to enter the code on stderr.
    #[arg(long, value_name = "ID")]
    client_id: Option<String>,
    /// The `OAuth2` device authorization endpoint, for the device flow.
    #[arg(long, value_name = "URL", requires = "client_id")]
    device_authorization_url: Option<String>,
    /// The `OAuth2` token endpoint, when the server's `/bcf/3.0/auth` names
    /// none.
    #[arg(long, value_name = "URL", requires = "client_id")]
    token_url: Option<String>,
}

#[derive(Args)]
pub struct PushArgs {
    /// The JSON result `check --report` wrote.
    result: PathBuf,
    #[command(flatten)]
    server: ServerArgs,
    /// Author of the decision comments' topics, as `--bcf-author`.
    #[arg(long, default_value = "axioval")]
    author: String,
}

#[derive(Args)]
pub struct PullArgs {
    /// The JSON result `check --report` wrote, whose findings the topics
    /// are matched to.
    result: PathBuf,
    #[command(flatten)]
    server: ServerArgs,
    /// The decisions file to update; created when it does not exist.
    #[arg(long, value_name = "FILE")]
    decisions: PathBuf,
}

pub fn run(command: BcfCommand) -> Result<(), Box<dyn Error>> {
    match command {
        BcfCommand::Push(args) => push(&args),
        BcfCommand::Pull(args) => pull(&args),
    }
}

fn connect(args: &ServerArgs) -> Result<bcf_api::Client, Box<dyn Error>> {
    let secret = std::env::var(SECRET_VARIABLE).ok();
    let auth = if let Ok(token) = std::env::var(TOKEN_VARIABLE) {
        bcf_api::Auth::Bearer(token)
    } else {
        match (&args.client_id, secret, &args.device_authorization_url) {
            (Some(client_id), _, Some(device)) => bcf_api::Auth::Device {
                client_id: client_id.clone(),
                device_authorization_url: device.clone(),
                token_url: args.token_url.clone(),
                show: Box::new(|code| {
                    let place = code
                        .verification_uri_complete
                        .as_deref()
                        .unwrap_or(&code.verification_uri);
                    eprintln!("sign in at {place} with the code {}", code.user_code);
                }),
            },
            (Some(client_id), Some(secret), None) => bcf_api::Auth::ClientCredentials {
                client_id: client_id.clone(),
                client_secret: secret,
                token_url: args.token_url.clone(),
            },
            (Some(_), None, None) => {
                return Err(format!(
                    "--client-id needs {SECRET_VARIABLE} or --device-authorization-url"
                )
                .into());
            }
            (None, _, _) => bcf_api::Auth::None,
        }
    };
    Ok(bcf_api::Client::connect(&args.server, &auth)?)
}

/// The project a saved result was computed over, as far as the result
/// names it: every object its report names, with its kind and `GlobalId`.
/// A resource object stays with the report, which carries it.
fn project_of(output: &CheckOutput) -> Result<Project, Box<dyn Error>> {
    let report = &output.report;
    let mut named: Vec<&ObjectId> = Vec::new();
    for finding in report.findings() {
        named.extend(finding.object_id());
        named.extend(&finding.related);
    }
    named.extend(report.not_evaluated().iter().filter_map(|n| n.object_id()));
    named.sort();
    named.dedup();
    let mut objects = Vec::new();
    for id in named {
        if report.resource(id).is_some() {
            continue;
        }
        let info = output
            .objects
            .get(&id.to_string())
            .ok_or_else(|| format!("the result does not describe {id}"))?;
        let mut object = Object::new(id.clone(), info.kind.clone());
        if let Some(global_id) = &info.global_id {
            object = object.with_external_id(axioval::ir::ExternalId::new(
                bcf::IFC_GLOBAL_ID_SCHEME,
                global_id,
            )?);
        }
        objects.push(object);
    }
    Ok(Project::new(objects)?)
}

fn push(args: &PushArgs) -> Result<(), Box<dyn Error>> {
    let output: CheckOutput = crate::load(&args.result)?;
    let project = project_of(&output)?;
    let client = connect(&args.server)?;
    let options = bcf::Options::new(args.author.clone(), crate::timestamp()?);
    let pushed = client.push(&args.server.project, &output.report, &project, &options)?;
    println!(
        "pushed to {}: {} topic(s) created, {} updated, {} comment(s), {} viewpoint(s) added",
        args.server.project, pushed.created, pushed.updated, pushed.comments, pushed.viewpoints
    );
    Ok(())
}

fn pull(args: &PullArgs) -> Result<(), Box<dyn Error>> {
    let output: CheckOutput = crate::load(&args.result)?;
    let report: &Report = &output.report;
    let project = project_of(&output)?;
    let mut decisions: Decisions = if args.decisions.exists() {
        crate::load(&args.decisions)?
    } else {
        Decisions::default()
    };
    let client = connect(&args.server)?;
    let markups = client.pull(&args.server.project)?;
    let imported = bcf::import_topics(
        &markups,
        report,
        &project,
        &std::collections::BTreeMap::new(),
    )?;
    let pulled = imported.decisions.decisions().len();
    for decision in imported.decisions.decisions() {
        let mut decision = decision.clone();
        if let Some(finding) = report
            .findings()
            .iter()
            .find(|finding| finding.id == Some(decision.finding))
        {
            decision = decision.with_basis(finding);
        }
        decisions.record(decision)?;
    }
    let json = serde_json::to_string_pretty(&decisions)? + "\n";
    crate::write(&args.decisions, json.as_bytes())?;
    for topic in &imported.unmatched {
        eprintln!(
            "warning: topic {} ({}) decided no finding of the result: {}",
            topic.guid.as_deref().unwrap_or("without GUID"),
            topic.title.as_deref().unwrap_or("untitled"),
            match &topic.reason {
                bcf::Unmatched::Unreadable(why) => format!("unreadable, {why}"),
                reason => reason.as_str().to_owned(),
            }
        );
    }
    println!(
        "pulled {pulled} decision(s) from {} into {}; {} topic(s) decided no finding",
        args.server.project,
        args.decisions.display(),
        imported.unmatched.len()
    );
    Ok(())
}
