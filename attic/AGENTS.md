# `attic/`

Retired artifacts kept for maintenance, deliberately **outside** the Cargo
workspace.

## `axioval-spec-notice/`

The source of `axioval-spec` `0.4.0`, a retirement notice pointing at
`axioval-ir` (rule packages) and `axioval-export` (export profiles),
containing no functionality. `0.3.0` is the last release with code and is
not yanked. Like the notice below it roots its own workspace, and
`cargo test` in its directory is the check. Republish only to correct the
text, bumping to `0.4.x`; never fold it back into the workspace.

## `axioval-openbim-notice/`

The source of `axioval-openbim` `0.2.0` on crates.io — a deprecation notice
pointing at `axioval-ifc`, containing no functionality.

It is not a workspace member on purpose. The publish gate asserts the workspace
holds exactly the crates in `EXPECTED_MEMBERS`, and this is not one of them: it shares neither the
workspace version nor its dependencies, and must keep building unchanged long
after the engine moves on.

Being outside the workspace is not automatic, though: it carries its own
empty `[workspace]` table. Without it Cargo walks up, finds the engine
workspace and refuses every command run here, which silently breaks the
republish procedure below. `cargo test` in this directory is the check.

To republish (rarely needed — only to correct the notice text):

```bash
cd attic/axioval-openbim-notice
cargo publish            # bump the version first; 0.2.x
```

Do **not** fold this into the workspace, and do not give it a `0.1.x` version:
`0.1.x` would be selected automatically by anyone depending on `"0.1"`, turning
a routine update into an empty crate. `0.1.11` remains the last functional
release under the old name and is intentionally not yanked.
