//! `axioval validate`: bind rulesets, or check one rule or expression draft
//! within them with positioned diagnostics, optionally dry-running it on a
//! model, once or as a long-lived JSON-lines process for editors.

use std::error::Error;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use axioval::engine::draft::{
    DraftValidation, ExpressionTraces, validate_expression, validate_rule,
};
use axioval::engine::{CapabilityRegistry, Runtime, compile_rulesets};
use axioval::ir::{DefinitionPackage, RuleSetPackage};
use clap::Args;
use serde_json::{Value, json};

use crate::{ModelArg, Outcome, model_arg, packages};

#[derive(Args)]
pub(crate) struct ValidateArgs {
    #[arg(long, required = true)]
    definitions: Vec<PathBuf>,
    /// A ruleset; repeat to bind several together, as `check` does. With a
    /// draft, they are its context.
    #[arg(long = "ruleset", required = true)]
    rulesets: Vec<PathBuf>,
    /// A rule draft (one rule as a ruleset writes it, JSON; `-` reads
    /// standard input): it replaces the context's rule of the same id or
    /// joins the first ruleset. Prints its diagnostics as JSON. Exit
    /// status: 0 valid, 3 refused, 1 the context cannot be read.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["expression", "serve"])]
    rule: Option<PathBuf>,
    /// An expression draft (JSON; `-` reads standard input), checked as
    /// the `--parameter` of the context's rule `--into`.
    #[arg(long, value_name = "FILE", requires = "into", conflicts_with = "serve")]
    expression: Option<PathBuf>,
    /// The context's rule an expression draft is placed in.
    #[arg(long, value_name = "RULE")]
    into: Option<String>,
    /// The parameter an expression draft is placed as.
    #[arg(long, value_name = "NAME", default_value = "requirement")]
    parameter: String,
    /// Serve drafts as JSON lines: each line read is `{"rule": …}` or
    /// `{"expression": …, "into": "rule", "parameter": "requirement"}`,
    /// each answered by one line of diagnostics, until standard input ends.
    #[arg(long)]
    serve: bool,
    /// Dry-run a valid draft on this model (`PATH[:DISCIPLINE]`, as for
    /// `check`; repeat for several): print how the drafted rule judged
    /// each object, with every expression's trace.
    #[arg(long = "model", value_name = "PATH[:DISCIPLINE]", value_parser = model_arg)]
    models: Vec<ModelArg>,
    /// With `--model`, mesh the model's bodies so geometric values answer.
    #[arg(long, requires = "models")]
    geometry: bool,
}

fn read(path: &Path) -> Result<String, Box<dyn Error>> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
        return Ok(text);
    }
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()).into())
}

/// One draft's answer, without a dry run.
fn answer(validation: &DraftValidation) -> Value {
    json!({
        "valid": validation.diagnostics.is_empty(),
        "rule": validation.rule,
        "diagnostics": validation.diagnostics,
    })
}

/// Whether a report's rule id is the drafted rule's, qualified or not.
fn is_rule(reported: &str, drafted: &str) -> bool {
    reported == drafted
        || reported
            .strip_suffix(drafted)
            .is_some_and(|package| package.ends_with('/'))
}

struct Context {
    registry: CapabilityRegistry,
    definitions: Vec<DefinitionPackage>,
    rulesets: Vec<RuleSetPackage>,
}

impl Context {
    fn validate(&self, request: &Value) -> Result<DraftValidation, String> {
        if let Some(rule) = request.get("rule").filter(|rule| rule.is_object()) {
            return Ok(validate_rule(
                &self.registry,
                &self.definitions,
                &self.rulesets,
                &rule.to_string(),
            ));
        }
        let expression = request
            .get("expression")
            .ok_or("a request states `rule` (an object) or `expression`")?;
        let into = request
            .get("into")
            .and_then(Value::as_str)
            .ok_or("an expression request states `into`, the rule it goes in")?;
        let parameter = request
            .get("parameter")
            .and_then(Value::as_str)
            .unwrap_or("requirement");
        Ok(validate_expression(
            &self.registry,
            &self.definitions,
            &self.rulesets,
            into,
            parameter,
            &expression.to_string(),
        ))
    }
}

fn serve(context: &Context) -> Result<Outcome, Box<dyn Error>> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => match context.validate(&request) {
                Ok(validation) => answer(&validation),
                Err(why) => json!({"valid": false, "error": why}),
            },
            Err(error) => {
                json!({"valid": false, "error": format!("the request is not JSON: {error}")})
            }
        };
        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }
    Ok(Outcome::Passed)
}

/// How the drafted rule judged each object of the models.
fn dry_run(args: &ValidateArgs, validation: DraftValidation) -> Result<Value, Box<dyn Error>> {
    let (Some(plan), Some(rule)) = (validation.plan, validation.rule) else {
        return Err("only a valid draft runs".into());
    };
    let (session, bytes) = crate::sources(&args.models)?;
    let session = if args.geometry {
        crate::geometry::attach(session, &bytes, crate::geometry::Options::meshes(false))
            .map_err(|error| format!("geometry: {error}"))?
            .0
    } else {
        session
    };
    let recorder = ExpressionTraces::new();
    let snapshots: Vec<_> = session.snapshots().cloned().collect();
    let session = session.with_host_service(recorder.clone(), &snapshots)?;
    let report = Runtime::new(axioval::default_registry()?).run_session(&session, plan)?;
    let traced: Vec<_> = recorder
        .take()
        .into_iter()
        .filter(|trace| is_rule(&trace.rule, &rule))
        .collect();
    let findings: Vec<_> = report
        .findings()
        .iter()
        .filter(|finding| is_rule(&finding.rule_id.to_string(), &rule))
        .collect();
    let not_evaluated: Vec<_> = report
        .not_evaluated()
        .iter()
        .filter(|outcome| is_rule(&outcome.rule_id.to_string(), &rule))
        .collect();
    Ok(json!({
        "traces": traced,
        "findings": findings,
        "notEvaluated": not_evaluated,
    }))
}

pub(crate) fn run(args: &ValidateArgs) -> Result<Outcome, Box<dyn Error>> {
    let (definitions, rulesets) = packages(&args.definitions, &args.rulesets)?;
    let registry = axioval::default_registry()?;
    if args.rule.is_none() && args.expression.is_none() && !args.serve {
        if !args.models.is_empty() {
            return Err("`--model` dry-runs a draft: give `--rule` or `--expression`".into());
        }
        let plan = compile_rulesets(&registry, &definitions, &rulesets)?;
        println!("validated {} executable rule(s)", plan.rules().len());
        return Ok(Outcome::Passed);
    }
    let context = Context {
        registry,
        definitions,
        rulesets,
    };
    if args.serve {
        return serve(&context);
    }
    let validation = match (&args.rule, &args.expression) {
        (Some(path), _) => validate_rule(
            &context.registry,
            &context.definitions,
            &context.rulesets,
            &read(path)?,
        ),
        (None, Some(path)) => validate_expression(
            &context.registry,
            &context.definitions,
            &context.rulesets,
            args.into.as_deref().unwrap_or_default(),
            &args.parameter,
            &read(path)?,
        ),
        (None, None) => unreachable!("handled above"),
    };
    let mut output = answer(&validation);
    let valid = validation.diagnostics.is_empty();
    if valid && !args.models.is_empty() {
        output["dryRun"] = dry_run(args, validation)?;
    }
    let mut json = serde_json::to_string_pretty(&output)?;
    json.push('\n');
    std::io::stdout().write_all(json.as_bytes())?;
    Ok(if valid {
        Outcome::Passed
    } else {
        Outcome::Findings
    })
}

#[cfg(test)]
mod tests {
    use super::is_rule;

    #[test]
    fn a_qualified_rule_id_is_the_drafted_rule() {
        assert!(is_rule("r1", "r1"));
        assert!(is_rule("package/r1", "r1"));
        assert!(!is_rule("package/xr1", "r1"));
        assert!(!is_rule("r10", "r1"));
    }
}
