# `axioval-ifc`

IFC source contracts, conformance doubles, and the production IFC4 STEP evidence-session seam.

- `src/ifc.rs` owns strict STEP parsing, IFC object adaptation, snapshot fingerprinting, the exact direct-property service, and session assembly.
- `src/relationships.rs` is the exact relationship-selection service. Relationship identities are IFC4 entity names; end slots come from the bundled schema (`ends_of`), never a hand-written table. Every answer carries a `relationship-scan` completeness locator.
- `src/integrity.rs` reports source irregularities through the same `read_instance` reader as selection. Keep them sharing one reader so a warning and a refusal always describe the same instances.
- Absent required ends (`$`) are recorded, not fatal: strict requests refuse, `AbsentEndPolicy::Skip` answers and cites each one, and integrity reports them as warnings. Dangling or wrongly shaped ends stay hard refusals and integrity errors. Do not widen skip to cover those.
- Preserve source-qualified identities and bind every evidence locator to the source fingerprint.
- IFC4 only. IFC2X3 is refused until upstream exact property resolution supports it (openbimrs/ifc#48); do not add a local IFC2X3 property resolver.
- Parse/model diagnostics, unsupported schemas, malformed traversal, conflicts, and unsupported values fail closed.
- Keep IFC dependencies behind the facade's optional `ifc` feature; never add Axiolid, geometry-kernel, rule-policy, path, or Git dependencies here.
- This crate adapts **IFC**, not the whole OpenBIM ecosystem. A future CityGML, GAEB, or IDS adapter is a new sibling under `sources/semantic/`, never a module added here.
- `UnavailableOpenBimImporter` remains the explicit conformance placeholder for host-defined importer implementations.
- Run `cargo test -p axioval-ifc` and strict clippy after changes. `tests/ifc_session.rs` is the production E2E contract; `tests/relationships.rs` and `tests/relationship_integrity.rs` pin the relationship and integrity contracts.
