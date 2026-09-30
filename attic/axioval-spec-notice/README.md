# axioval-spec

> **Retired.** Rule packages are [`axioval-ir`](https://crates.io/crates/axioval-ir);
> writing them as other formats is [`axioval-export`](https://crates.io/crates/axioval-export).
> This crate is a notice only and contains no functionality.

No Axioval crate depended on `axioval-spec`. Its types duplicated what the
package contracts in `axioval-ir` state, and its export targets were a closed
list shaped after particular host applications, which a source-neutral engine
cannot keep.

## Migrating

- Rule definitions, rule instances, rulesets, selectors and severities:
  `axioval_ir::contract`.
- Writing a ruleset as another format, and stating what that format cannot
  hold: implement `axioval_export::ExportProfile`. Each profile has an open
  string id, and every loss is reported as refused (the item's checking
  meaning cannot be expressed) or degraded (structure or presentation only).

## About the versions

The last release with code is **0.3.0**. It is not yanked, so existing
lockfiles keep building; it receives no further updates. The notice ships as
**0.4.0**, so no build depending on `"0.3"` picks it up by accident.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE).
