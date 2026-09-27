# `axioval-cli`

Strict command-line entry point for normalized Axioval packages and validation runs.

CLI output and exit codes are public automation contracts. Parse packages fail closed, write diagnostics to stderr, and use non-zero status for invalid input or execution failure.

- `check` exit status: 0 complete and clean, 3 findings, 4 not evaluated without findings, 1 failure, 2 usage (clap). Status 4 exists so an incomplete check can never exit 0; never fold it into 0.
- Build every output before writing any, so a failing run leaves no partial report or archive.
- The CLI is the host: it may enable several facade features and compose an adapter with a sink. Libraries never do.
- Never read the clock for BCF output when `--bcf-date` or `SOURCE_DATE_EPOCH` is given.
- `src/digest.rs` owns the saved result format (`CheckOutput`) and its two views. The summary must stay bounded by distinct groups, never by entry count; the listing must page. Every "next" hint must run unchanged in a POSIX shell: quote through `shell_quote`, which quotes `#` because an unquoted `#` starts a comment.
- The result's `objects` field is additive (`serde(default)`), so results saved before it existed still load.
- `src/geometry.rs` is the IFC→Axiolid bridge. It belongs here and nowhere else: the adapters must not depend on each other. Every object must end in exactly one state: exact mesh, tessellated mesh, no body, or unmeasured. Never declare a physical product bodiless because meshing failed; that makes it vanish from contact and free-space checks. The planarity check may only grow by structures proven planar; anything unrecognised stays tessellated.
- Register a geometry service only when every role it needs can be read from IFC; otherwise leave it unregistered so its rules report `missing-service`.
- `--model PATH[:DISCIPLINE]` repeats: each file is imported as its own session and `EvidenceSession::federate` combines them before geometry is attached. The bridge meshes every source into one set, looks each object's model up by its source, builds spatial trees per source, and binds every service to all snapshots. Never key anything on a bare local id: two files share `#10`.
- `src/compare.rs` is `axioval compare`: two single-model sessions matched by `GlobalId` through `axioval_rules::compare_sessions`. Its result is a `CheckOutput` with the additive `comparison` field (owned by `digest.rs`), so `report`, `--summary` and `--bcf` stay shared through `emit`; exit status as for `check`. Two files with one name become `name@base` and `name@revised`; never let the two revisions share a source id. `tests/compare.rs` runs it end to end.
- Walkability takes every role from its request; metric routing takes surfaces (`IfcSpace`), portals (`IfcDoor`, opening voids) and connectors (stairs, ramps, transport elements) from IFC classes. Never declare a door clear width from `OverallWidth`: it includes the lining, and an overstated clear width turns an undecided route into a false pass.
- `tests/check.rs` runs the real binary; keep a case for every exit status. Update `docs/src/cli.md` with any change to arguments, output or status.
