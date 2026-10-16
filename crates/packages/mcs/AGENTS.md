# `axioval-mcs`

The reader of MCS package archives (`.mcs`): the canonical ZIP container,
its `META-INF/mcs.json` metadata and inventory, the archived manifest, and
the normalized packages as the IR's `RuleSetPackage` and
`DefinitionPackage`, with their table files loaded from the archive. See
`docs/src/mcs-archives.md`.

- The contract is axioval/mcs's, never this crate's: `scripts/mcs_archive.py`
  (`_read_verified`, `verify`), `scripts/validate.py` (`validate_manifest`,
  `bind_definitions_package`) and `schema/registry-manifest.schema.json`.
  Mirror a change there here, check for check, and refuse what it refuses;
  never accept an archive it refuses. Where the engine adds a check (one the
  MCS verifier makes by evaluating Pkl, checked statically against the
  normalized JSON), say so in a comment.
- Fail closed, one `McsError` variant per refusal a host acts on. The
  container is parsed byte by byte (`src/container.rs`) before anything is
  inflated; never hand it to a lenient ZIP reader first.
- The kind comes from the archived manifest only: a `definitions` archive
  carrying a rule set and a `ruleset` archive without one are refused, and a
  definitions manifest's identity must be the package identity of exactly
  one definition document.
- Never evaluate the Pkl sources, and never run anything from an archive.
- Core to the architecture gate: it reads packages, never a model, and
  depends on `axioval-ir`, `axioval-engine` (`PackageFiles`, the table-file
  loaders) and format-neutral libraries (`flate2`, `sha2`,
  `unicode-normalization`). The facade is a path-only dev-dependency.
- Fixtures are real MCS packer output (`fixtures/mcs-v0.1.0`, commit in
  `fixtures/README.md`); tests change one thing in an unpacked copy
  (`tests/support`) and assert the exact refusal. Never edit a fixture
  archive by hand.
- Run `cargo test -p axioval-mcs`.
