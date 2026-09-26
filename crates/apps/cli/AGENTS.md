# `axioval-cli`

Strict command-line entry point for normalized Axioval packages and validation runs.

CLI output and exit codes are public automation contracts. Parse packages fail closed, write diagnostics to stderr, and use non-zero status for invalid input or execution failure.

- `check` exit status: 0 complete and clean, 3 findings, 4 not evaluated without findings, 1 failure, 2 usage (clap). Status 4 exists so an incomplete check can never exit 0; never fold it into 0.
- Build every output before writing any, so a failing run leaves no partial report or archive.
- The CLI is the host: it may enable several facade features and compose an adapter with a sink. Libraries never do.
- Never read the clock for BCF output when `--bcf-date` or `SOURCE_DATE_EPOCH` is given.
- `tests/check.rs` runs the real binary; keep a case for every exit status. Update `docs/src/cli.md` with any change to arguments, output or status.
