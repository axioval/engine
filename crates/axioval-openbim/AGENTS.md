# `axioval-openbim`

OpenBIM semantic source contracts and in-memory conformance double.

- Preserve source-qualified identities and producer ordering.
- Do not add Axiolid, geometry-kernel, or rule-policy dependencies here.
- `UnavailableOpenBimImporter` is intentional: wire a real IFC/STEP parser only behind `OpenBimImporter` and make unavailability explicit.
- Run `cargo test -p axioval-openbim` after changes; `tests/conformance.rs` is the public contract.
