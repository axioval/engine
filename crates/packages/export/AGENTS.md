# `axioval-export`

The framework every export target shares: the `ExportProfile` trait, the
`ExportOutcome` with its `Loss`es, and the rule comparator profiles judge
exactness with. See `docs/src/export.md`.

- Depends on `axioval-ir` and `serde_json` only. It is core: it names no
  target format, no host application and no source, and enumerates no
  profiles. A profile id is an open string; never turn it into an enum.
- The safety rule: anything that would change a check's result is
  `LossKind::Refused`, never `Degraded`. Degraded is for structure and
  presentation only (flattened folders, cut depth, dropped languages, an
  unsupported column, annotations). `tests/safety.rs` pins it; keep it
  passing and extend it with every new comparison field.
- `compare` is the one comparator: `canonical` decides what is compared,
  `substitute` binds concepts to names and rule ids to positions,
  `first_difference` names the path. A new rule field that decides a
  verdict must enter `canonical`, or every profile would export it as
  equal. Format-specific leniency (case, presentation parameters) goes
  through `Comparison`, never a special case here.
- `precheck` is the order profiles refuse non-plain rules in (disabled,
  auxiliary, gated, graded, severity, groups). `PreCheck` is exhaustive on
  purpose, so a new reason reaches every profile's mapping.
- `axioval-ids` depends on this crate: changing `canonical`,
  `first_difference` or `pre_check` must keep the IDS corpus round trip
  complete (`IDS_TEST_CASES=… ./scripts/check.sh test`).
- Run `cargo test -p axioval-export`.
