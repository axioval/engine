//! Reports how much of each IDS document translates, and why the rest does not.
//!
//! ```text
//! cargo run --example coverage -- a.ids b.ids ...
//! ```
#![allow(missing_docs)]

use std::collections::BTreeMap;

use axioval_ids::{Options, translate};

fn main() {
    let options = Options {
        package_id: "ids:coverage".into(),
        version: "0.0.0".into(),
    };
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let (mut specifications, mut complete, mut skipped, mut rules) = (0, 0, 0, 0);
    for path in std::env::args().skip(1) {
        let ids = match std::fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| openbim_ids::from_slice(&bytes).map_err(|error| error.to_string()))
        {
            Ok(ids) => ids,
            Err(error) => {
                println!("{path}: unreadable: {error}");
                continue;
            }
        };
        let translation = translate(&ids, &options).expect("valid options");
        for outcome in &translation.specifications {
            specifications += 1;
            rules += outcome.rules.len();
            complete += usize::from(outcome.is_complete());
            skipped += usize::from(outcome.is_skipped());
        }
        for (_, gap) in translation.gaps() {
            // Group by reason kind, not by the names inside it.
            let reason = gap.reason.to_string();
            let key = reason
                .split('"')
                .next()
                .unwrap_or(&reason)
                .trim()
                .to_owned();
            *reasons.entry(key).or_default() += 1;
        }
    }
    println!(
        "{specifications} specifications: {complete} complete, {skipped} skipped, {rules} rules"
    );
    let mut ranked: Vec<_> = reasons.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (reason, count) in ranked {
        println!("{count:5}  {reason}");
    }
}
