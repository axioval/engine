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

# Parity models and cases

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
