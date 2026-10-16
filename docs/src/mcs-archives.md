# MCS package archives

An [Axioval MCS](https://github.com/axioval/mcs) package travels as a `.mcs`
archive: a deterministic ZIP container of the reviewed Pkl sources
(`source/`), the normalized JSON packages the engine binds (`normalized/`)
and `META-INF/mcs.json`, which states the package identity, maps each
source module to its normalized member and inventories every member by
role, size and SHA-256. The contract is MCS's own:
`schema/registry-manifest.schema.json`, `scripts/mcs_archive.py` and
`scripts/validate.py` in that repository. `axioval-mcs` reads it as that
reader does and returns the normalized packages as the IR's
`RuleSetPackage` and `DefinitionPackage`; the CLI takes an archive wherever
it takes `--ruleset` or `--definitions` ([command line](./cli.md#mcs-package-archives)).

The Pkl sources are never evaluated. The engine binds the normalized JSON
exactly as it binds a package written as JSON, so an archive grants no
trust a JSON package lacks, and package code never runs.

## Two kinds

The archived manifest (`axioval.json`) states the kind, and the archive
must agree with it:

| Kind | Carries | Manifest |
| --- | --- | --- |
| `ruleset` | `normalized/ruleset.json` and the definition documents it binds | `entrypoint` and `definitionEntrypoints` |
| `definitions` | `normalized/definitions/*.json` only: a library of definitions, such as rule templates | `definitionEntrypoints`, no `entrypoint` |

A definitions manifest's `id` and `version` must be the package identity of
exactly one of its definition documents: that document is the library the
archive publishes.

A rule set names the definition packages it binds by ID
(`definitionPackages`). Each resolves from the documents the rule-set
archive carries or from the definition packages the host supplies (a
`definitions` archive or JSON), through `resolve_definitions`: every
supplied package, then each carried one whose ID none supplied has. A
package carried and supplied is kept once when the two are equal and
refused (`ConflictingDefinitions`) when they differ. An ID nothing gives is
refused by the compiler (`MissingDefinitionPackage`), as for JSON input. A
rule-set archive may not carry a definition document its rule set does not
declare.

## What is refused

`axioval_mcs::read` fails closed. Each refusal is its own `McsError`:

- **The container** (`Container`): anything but the canonical layout the
  MCS packer writes: `mimetype` first and stored with
  `application/vnd.axioval.mcs+zip`, the metadata second and at most
  256 KiB, every other member
  deflated, in central-directory order from offset 0 with no gap, preamble
  or trailing data, no extra fields, comments, data descriptors or ZIP64,
  the fixed 1980-01-01 timestamp and `0644` regular-file attributes, a
  UTF-8 flag exactly on non-ASCII names, and a size and CRC-32 matching
  every member; 3 to 512 members.
- **Member names** (`UnsafeName`, `DuplicateMember`): empty, longer than
  240 characters, absolute, with a backslash, a control character, an
  empty, `.` or `..` segment, or not in NFC; two members of one name, or of
  names equal once case folded.
- **Limits** (`Limit`): an archive over 64 MiB, a member over 10 MB or
  compressed more than 100 to 1, members totalling over 64 MiB.
- **The metadata and inventory** (`Metadata`): unknown or missing keys,
  other constants, a package root outside `source/`, a manifest other than
  `<package root>/axioval.json`; an inventory entry whose role does not fit
  its path, whose size or SHA-256 differs from the member's, unsorted
  entries, a member it does not declare; a normalized mapping that is
  unsorted, maps one member twice, leaves a normalized member unmapped or
  names other modules than the manifest's entrypoints; a `.pkl-version`
  other than the metadata's.
- **The manifest** (`Manifest`), validated as `validate.py` validates it,
  and its `$schema`, which must name a source member.
- **Identity**: metadata whose package identity differs from the archived
  manifest's (`IdentityMismatch`); a normalized rule set or definition
  document of another identity or schema version than the metadata's
  (`Package`); a definitions manifest naming no document, or several
  (`DefinitionsIdentity`).
- **Kind**: a `definitions` archive carrying a rule set
  (`DefinitionsCarryRuleSet`), a `ruleset` archive without one
  (`RuleSetMissing`).
- **Binding** (`Binding`): definition documents sharing a package ID, a
  rule-definition ID or a concept ID, an ID naming components of two kinds,
  or a carried document the rule set does not declare.

Every JSON member is read strictly: a duplicate object key is refused.

## As a library

```rust,ignore
let archive = axioval_mcs::read_file(path)?;      // or read(&bytes)
match archive.kind() {
    PackageKind::RuleSet => {
        let (ruleset, carried) = archive.into_ruleset()?;
        let definitions = axioval_mcs::resolve_definitions(installed, carried)?;
        let plan = compile(&registry, &definitions, &ruleset)?;
    }
    PackageKind::Definitions => installed.extend(archive.into_definitions()?),
}
```

`is_archive` tells an archive by its first member. Table files a package's
parameters name are loaded from the archive's package root
(`Archive::files`, a `PackageFiles`), so the returned packages bind as they
are.

## Not checked

The engine reads an archive as data and does not run Pkl, so the checks of
the MCS verifier that evaluate the sources are left to it: that the
normalized JSON is exactly what the sources evaluate to, that the sources
are exactly the Pkl import closure, and that the dependency lock resolves.
A host that installs archives from an untrusted registry runs
`scripts/mcs.py verify` before it trusts their provenance; the engine's
guarantees (no package code, fail-closed binding) hold either way.
