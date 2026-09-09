# axioval-openbim

> **Deprecated — renamed to [`axioval-ifc`](https://crates.io/crates/axioval-ifc).**
> This crate is a notice only and contains no functionality.

`axioval-openbim` was renamed to `axioval-ifc` in Axioval 0.1.12. The name now
says what the adapter actually maps — IFC semantics — rather than the broader
OpenBIM umbrella. Nothing was removed in the rename.

## Migrating

```toml
# before
axioval-openbim = "0.1"

# after
axioval-ifc = "0.1"
```

- Rust paths change from `axioval_openbim::` to `axioval_ifc::`.
- Using the `axioval` facade, the feature flag `openbim` is now `ifc`.
- Types, methods and behaviour are otherwise unchanged.

## About the versions

The final functional release under this name is **0.1.11**. It is intentionally
**not yanked**: yanking would break existing lockfiles while explaining nothing.
Pinning `=0.1.11` keeps working — it simply receives no further updates.

This notice ships as **0.2.0**, not 0.1.12. Under Cargo's semver rules a 0.1.12
would be picked up automatically by anyone depending on `"0.1"`, silently
turning a routine update into an empty crate. A minor bump within `0.x` is a
breaking change, so existing builds stay put until their author opts in.

crates.io has no rename primitive, which is why the old name has to be retired
this way rather than redirected.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE).
