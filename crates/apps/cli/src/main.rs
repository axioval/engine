//! Command-line validation of normalized Axioval packages.

use std::{error::Error, fs, path::PathBuf};

use axioval::{
    default_registry,
    engine::compile,
    ir::{DefinitionPackage, RuleSetPackage},
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "axioval", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Strictly bind a ruleset to definitions and trusted capabilities.
    Validate {
        #[arg(long, required = true)]
        definitions: Vec<PathBuf>,
        #[arg(long)]
        ruleset: PathBuf,
    },
}
fn load<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T, Box<dyn Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}
fn run() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Validate {
            definitions,
            ruleset,
        } => {
            let definitions = definitions
                .iter()
                .map(load)
                .collect::<Result<Vec<DefinitionPackage>, _>>()?;
            let ruleset: RuleSetPackage = load(&ruleset)?;
            let plan = compile(&default_registry()?, &definitions, &ruleset)?;
            println!("validated {} executable rule(s)", plan.rules().len());
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("axioval: {error}");
        std::process::exit(1);
    }
}
