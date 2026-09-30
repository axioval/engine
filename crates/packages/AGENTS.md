# `packages/`

Importers that write Axioval rule packages (definition packages and
rulesets) from other rule formats, one subdirectory per format, and
`export/`, the framework every export target shares.

- An importer never reads a model and adapts no source: it depends on
  `axioval-ir`, the engine's capability descriptors and `axioval-rules`
  helpers, never on a source adapter or geometry kernel. Tests may run the
  output through the facade as a path-only dev-dependency.
- It is core to the architecture gate. The format reader and any schema
  table it needs are exempted by name, for that crate alone, in
  `PERMITTED_COUPLINGS` of `scripts/architecture.py`.
- Imported packages get no trust the format did not state: every rule selects
  a trusted built-in capability, and a construct no capability decides
  exactly is a reported gap, never a silently dropped or widened rule.
- A new importer is a workspace member through `crates/packages/*`: add it to
  `EXPECTED` in `scripts/check_package_contents.py`, bump `EXPECTED_MEMBERS`
  in `scripts/staging_isolation.py`, and give it its own `AGENTS.md`.

- An exporter implements `axioval_export::ExportProfile` and judges
  exactness with its comparator (`compare`, `precheck`), never with one of
  its own. Anything that would change a verdict is a refused `Loss`, never
  a degraded one; name no host product, only the format.

## Direct children

- `export/` (`axioval-export`) — export profiles, losses and the shared rule comparator.
- `ids/` (`axioval-ids`) — rule packages from buildingSMART IDS documents, and exported back to them.
