# `axioval-ifc`

IFC source contracts, conformance doubles, and the production IFC2X3/IFC4 STEP evidence-session seam.

- `src/release.rs` decides the one release a file declares (`IfcRelease::from_header`) and holds each release's schema table and type-system identity. Every service receives that release; none may call `ifc4()`/`ifc2x3()` directly. IFC2X3 and IFC4 share entity names with different attributes and ancestry, so reading one with the other's table gives confident wrong answers, not errors.
- `src/ifc.rs` owns strict STEP parsing, IFC object adaptation, snapshot fingerprinting, the exact direct-property service (upstream `ifc-properties` ≥ 0.2.1, which binds per release), and session assembly.
- `src/relationships.rs` is the exact relationship-selection service. Relationship identities are entity names in the source's own release; end slots come from that release's schema (`ends_of`), never a hand-written table. Every answer carries a `relationship-scan` completeness locator.
- `src/integrity.rs` reports source irregularities through the same `read_instance` reader as selection. Keep them sharing one reader so a warning and a refusal always describe the same instances.
- Absent required ends (`$`) are recorded, not fatal: strict requests refuse, `AbsentEndPolicy::Skip` answers and cites each one, and integrity reports them as warnings. "Required" is per release: `IfcRelSpaceBoundary.RelatedBuildingElement` is optional in IFC2X3 and required in IFC4. Dangling or wrongly shaped ends stay hard refusals and integrity errors. Do not widen skip to cover those.
- `src/unread.rs` indexes the property-set definitions `exact_property` skips (quantity sets, predefined property sets). The property service refuses an absence that one of them could hold. Keep it an index of names only; resolving quantities belongs upstream.
- `src/identity.rs` reads every `IfcRoot` GlobalId once per session. The same scan decides which objects carry the `ifc-globalid` alias and which integrity warnings are raised, so a missing alias always has exactly one warning. Never attach a GlobalId that is invalid or shared, and keep the `to_uuid`/`from_uuid` round trip until openbimrs/ifc#62 lands.
- `src/classifications.rs` maps `ifc-classification` answers onto `ClassificationService`. Never read classification slots here: release-specific names, notations and hierarchy rules are the upstream crate's. An assignment with no stated system stays `system: None`; never infer one from names or locations.
- Integrity cardinality warnings (`CONTAINED_TWICE`, `ZONE_MEMBER_NOT_SPATIAL`) come from `ifc-systems` anomalies; do not re-derive them from raw relationships.
- Preserve source-qualified identities and bind every evidence locator to the source fingerprint.
- IFC2X3 TC1 and IFC4 ADD2 TC1 only. IFC4X3 is refused until upstream exact property resolution covers it; do not add a local resolver for any release.
- Parse/model diagnostics, unsupported schemas, malformed traversal, conflicts, and unsupported values fail closed.
- Keep IFC dependencies behind the facade's optional `ifc` feature; never add Axiolid, geometry-kernel, rule-policy, path, or Git dependencies here.
- This crate adapts **IFC**, not the whole OpenBIM ecosystem. A future CityGML, GAEB, or IDS adapter is a new sibling under `sources/semantic/`, never a module added here.
- `UnavailableOpenBimImporter` remains the explicit conformance placeholder for host-defined importer implementations.
- Run `cargo test -p axioval-ifc` and strict clippy after changes. `tests/ifc_session.rs` is the production E2E contract; `tests/ifc2x3.rs` pins release binding; `tests/relationships.rs` and `tests/relationship_integrity.rs` pin the relationship and integrity contracts; `tests/identity.rs` pins GlobalId aliases; the facade's `tests/ifc_classification.rs` pins classification selection end to end.
