//! A check's reuse report on stderr (`check --stats`, #442).
//!
//! At the end of each run, one tab-separated line per derivation kind of
//! the run's store (and per table of its measurement memo), after a header
//! naming the fields. It is printed outside the result, so the result of a
//! run with `--stats` is the result of one without, byte for byte.

use std::sync::Arc;

use axioval::engine::{DerivationObserver, DerivationStatistics};

/// The fields of each line, in order; every line starts with `stats`.
pub const FIELDS: [&str; 9] = [
    "kind",
    "version",
    "hits",
    "builds",
    "waits",
    "build_seconds",
    "peak_bytes",
    "released",
    "evicted",
];

/// An observer printing the header and one line per kind at the end of
/// each run.
pub fn observer() -> DerivationObserver {
    Arc::new(|statistics| {
        eprintln!("stats\t{}", FIELDS.join("\t"));
        for kind in statistics {
            eprintln!("stats\t{}", line(kind));
        }
    })
}

/// A kind's fields, tab-separated: a memo table has no version (`-`).
fn line(kind: &DerivationStatistics) -> String {
    let version = kind
        .version
        .map_or_else(|| "-".to_owned(), |version| version.to_string());
    format!(
        "{}\t{version}\t{}\t{}\t{}\t{:.6}\t{}\t{}\t{}",
        kind.kind,
        kind.hits,
        kind.builds,
        kind.waits,
        kind.build_time.as_secs_f64(),
        kind.peak_bytes,
        kind.released,
        kind.evicted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval::engine::{DerivationVersion, Kernel};
    use std::time::Duration;

    #[test]
    fn a_line_states_every_field_in_order() {
        const KERNELS: &[Kernel] = &[Kernel {
            name: "kernel",
            version: "0.1.0",
        }];
        let kind = DerivationStatistics {
            kind: "placement scene",
            version: Some(DerivationVersion::new(2, KERNELS)),
            entries: 3,
            hits: 10,
            builds: 4,
            waits: 1,
            build_time: Duration::from_millis(1500),
            released: 2,
            evicted: 5,
            bytes: 100,
            peak_bytes: 4096,
        };
        assert_eq!(
            line(&kind),
            "placement scene\t2 (kernel 0.1.0)\t10\t4\t1\t1.500000\t4096\t2\t5"
        );
        assert_eq!(line(&kind).split('\t').count(), FIELDS.len());
        let memo = DerivationStatistics {
            kind: "memo",
            ..DerivationStatistics::default()
        };
        assert_eq!(line(&memo), "memo\t-\t0\t0\t0\t0.000000\t0\t0\t0");
    }
}
