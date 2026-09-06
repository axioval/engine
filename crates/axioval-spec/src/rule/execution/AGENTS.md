# Execution IR modules

This directory decomposes `../execution.rs` without changing its public API.

- `validation.rs`: `CheckRuleSpec` semantic validation.
- `properties.rs`: property predicates, aggregate rows, and requirement state.
- `scope.rs`: neutral IFC element scope, field, domain, and relation types.
- `diagnostics.rs`: typed lowering/compiler diagnostics.

`execution.rs` is the compatibility facade and re-exports the public child-module types. Keep authored DTOs and source aliases in `codec`; keep concrete runtime rules in `rules`. File moves must remain behavior-neutral and preserve serde names/defaults exactly.
