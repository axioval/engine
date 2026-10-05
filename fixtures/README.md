# Contract fixtures

`schema-v0.1.0/` is copied byte-for-byte from the checked normalized output of
the Axioval MCS `examples/minimal` package (`examples/minimal/expected/*.json`)
at commit `307ce0805b7ae782d0c39a98b6442a4e6b9325c8`.

These files are executable compatibility fixtures: the Rust binder must
deserialize and bind them exactly, reject missing or mismatched definitions and
parameters, and execute `axioval:capability.property-exists` without evaluating
package-provided code. Because they are real MCS output rather than hand-written
JSON, a contract drift between MCS and the engine fails here first.

Refresh them by copying the same two files again and updating the commit above;
never edit them by hand.

## Parity models and cases

`parity/models.json` pins the public IFC models the parity harness runs on,
each by a URL fixed to an upstream commit, its SHA-256, its licence and its
attribution. The models are not vendored: `scripts/parity_models.py fetch`
downloads them into a cache and verifies every digest. They are taken from
buildingSMART International's
[Sample-Test-Files](https://github.com/buildingSMART/Sample-Test-Files) at
commit `80d976a9b193a26a8e928c3e79bff67af1de68a8`, licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) (© buildingSMART
International Ltd.); only the test's verdicts about them are published.

`parity/cases/` holds the rule packages run over them, written for this
repository: each case's `definitions.json` and `ruleset.json`, and its
`parity.json` naming the models, the rule pairs compared and every
recorded divergence. `crates/apps/cli/tests/parity.rs` documents the format.
To pin another model, add it to the manifest with its digest
(`sha256sum`), from a source whose licence `scripts/parity_models.py`
accepts.

## IFC4X3 earthworks

`ifc4x3-earthworks/infra-road-fill.ifc` is a trimmed excerpt of
`IFC 4.3.2.0 (IFC 4.3 ADD2)/Simple-Scene/Infra-Road.ifc` from
[buildingSMART/Sample-Test-Files](https://github.com/buildingSMART/Sample-Test-Files/blob/80d976a9b193a26a8e928c3e79bff67af1de68a8/IFC%204.3.2.0%20(IFC%204.3%20ADD2)/Simple-Scene/Infra-Road.ifc)
at commit `80d976a9b193a26a8e928c3e79bff67af1de68a8` (whole file SHA-256
`a3cb29433d7176bd31ff7535a596b5d7cd4eb7f2fb0eb6eee451612063c1f167`).
© buildingSMART International Ltd., licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

Changed from the original: only the header and the entities reachable from
the project (`#13`) and the `IfcEarthworksFill` `#473` ("road subgrade -
bridge road") are kept, each line unchanged and in its original order; every
other entity is left out. The fill is one `IfcTriangulatedFaceSet` whose top
is a level part, a ramp rising one in ten and a short step about 84° steep,
so the hull over its top's slope spans 0° to 84° while each piece's slope is
decided (`crates/apps/cli/tests/check.rs`). Regenerate it by keeping the same
entities of the same file at the same commit.
