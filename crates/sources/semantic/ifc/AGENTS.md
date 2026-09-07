# `axioval-ifc`

IFC source contracts, conformance doubles, and the production IFC4 STEP evidence-session seam.

- `src/ifc.rs` owns strict STEP parsing, IFC object adaptation, snapshot fingerprinting, and the exact direct-property service.
- Preserve source-qualified identities and bind every evidence locator to the source fingerprint.
- Property completeness is direct-object only. Do not register relationship traversal until a separately exact relationship service exists.
- Parse/model diagnostics, unsupported schemas, malformed traversal, conflicts, and unsupported values fail closed.
- Keep IFC dependencies behind the facade's optional `ifc` feature; never add Axiolid, geometry-kernel, rule-policy, path, or Git dependencies here.
- This crate adapts **IFC**, not the whole OpenBIM ecosystem. A future CityGML, GAEB, or IDS adapter is a new sibling under `sources/semantic/`, never a module added here.
- `UnavailableOpenBimImporter` remains the explicit conformance placeholder for host-defined importer implementations.
- Run `cargo test -p axioval-ifc` and strict clippy after changes; `tests/ifc_session.rs` is the production E2E contract.
