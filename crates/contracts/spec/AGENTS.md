# axioval-spec

Source-neutral rule and classification vocabulary. Depends on `serde`,
`serde_json` and `thiserror` -- nothing else, ever.

## What is here

- `src/rule/` -- what must be true. `RuleDefinition` (parameterized check),
  `RuleInstance` (bound to values), `RuleSetPackage` (the tree a backend
  compiles into one document), plus one module per rule family's plan spec
  that no registered capability covers yet.
- `src/classification/` -- which elements a rule applies to and what they are.
  Layered: `predicate` (leaf test) -> `expr` (And/Or/Not) -> `scheme` (named,
  ordered, methoded).

## Boundaries

This crate answers *what a rule is*. It does not know how to read a model, how
to compute geometry, or how to write a vendor document.

- No source formats (IFC/STEP/ICDD), no geometry kernels, no vendor types.
  `scripts/architecture.py` fails the build if one appears.
- Backend *bindings* -- lowering a catalog entry into concrete vendor bytes --
  belong to the application that owns that format, not here.
- `Target::Cset` names a backend as an enum discriminant. Naming a target is
  not depending on it; there is no vendor code in this crate.

## Pitfalls

- **Do not add a plan a capability already expresses.** A `*PlanSpec` that
  restates a capability's parameters is removed, not lowered: the lowering
  would need `axioval-ir`, which this crate may not depend on. Record the
  replacement in the "Retired rule plans" table of `docs/src/migration.md`
  and drop its `CheckSemantics` variant.

- **`ParamValue::as_f64` refuses integers above 2^53.** `f64` cannot represent
  them, and answering a comparison with a value the package never declared is
  an exactness bug. Use `as_i64` for exact integers. Pinned by
  `rule::param::exactness_tests`.
- The crate-level `allow` list covers *shape* characteristics of vocabulary
  moved verbatim from the authoring application (wide flag structs, long
  exhaustive matches). Behavioural pedantic lints stay on -- do not extend the
  list to silence a real finding.
