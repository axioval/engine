# `attic/`

Retired artifacts kept for maintenance, deliberately **outside** the Cargo
workspace.

## `axioval-openbim-notice/`

The source of `axioval-openbim` `0.2.0` on crates.io — a deprecation notice
pointing at `axioval-ifc`, containing no functionality.

It is not a workspace member on purpose. The publish gate asserts the workspace
is exactly nine crates, and this is not one of them: it shares neither the
workspace version nor its dependencies, and must keep building unchanged long
after the engine moves on.

To republish (rarely needed — only to correct the notice text):

```bash
cd attic/axioval-openbim-notice
cargo publish            # bump the version first; 0.2.x
```

Do **not** fold this into the workspace, and do not give it a `0.1.x` version:
`0.1.x` would be selected automatically by anyone depending on `"0.1"`, turning
a routine update into an empty crate. `0.1.11` remains the last functional
release under the old name and is intentionally not yanked.
