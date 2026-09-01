# `axioval-openbim`

OpenBIM source contracts, conformance doubles, and the production IFC4 STEP evidence-session seam.

- `src/ifc.rs` owns strict STEP parsing, IFC object adaptation, snapshot fingerprinting, and the exact direct-property service.
- Preserve source-qualified identities and bind every evidence locator to the source fingerprint.
- Property completeness is direct-object only. Do not register relationship traversal until a separately exact relationship service exists.
- Parse/model diagnostics, unsupported schemas, malformed traversal, conflicts, and unsupported values fail closed.
- Keep OpenBIM dependencies behind the facade's optional `openbim` feature; never add Axiolid, geometry-kernel, rule-policy, path, or Git dependencies here.
- `UnavailableOpenBimImporter` remains the explicit conformance placeholder for host-defined importer implementations.
- Run `cargo test -p axioval-openbim` and strict clippy after changes; `tests/ifc_session.rs` is the production E2E contract.
