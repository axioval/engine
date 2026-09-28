# `axioval-ir`

Source-neutral identities, values, selectors, provenance, evidence, findings, and normalized package contracts.

Keep this crate serializable and deterministic. It must not depend on a source format, federation format, geometry backend, or executable rule implementation.

- `identity.rs` derives `FindingId`s: UUIDv5 over rule, the objects' aliases
  in a host-named stable scheme (source-qualified id without one), scope kind
  and message, qualified by sources only when a key repeats. The BCF sink's
  topic GUIDs are these identities, and users' decisions are keyed by them:
  the key layout and `NAMESPACE` are a compatibility contract. Never add
  location, evidence or severity to the key.
- `decision.rs` owns decisions. They only mark findings (`Finding::decision`)
  and list stale ones (`Report::stale_decisions`); they never remove a
  finding, change a severity or touch a not-evaluated outcome. Change is
  judged from the recorded basis only (rule, message, severity, evidence
  counts); never compare evidence locators, which renumber on re-export.
  A missing basis is `unknown`, never `unchanged`.
- `Finding::id`, `Finding::decision` and `Report::stale_decisions` are absent
  on the wire when unset, so reports serialize as before.
- Resource objects (instances outside the object population) are never in
  a `Project`. A report carries the ones its outcomes name in
  `Report::resources`, sorted, omitted when empty; resolve an outcome's
  object with `Report::object`, never `Project::object` alone, wherever a
  report may name one (identities, sinks, hosts).
- `PropertyValue::Reference` names another instance of the same source. It
  is never compared with a literal and never read as text or a number.
- `PropertyValue::Complex` is a property grouping members, present and no
  value of any type: not scalar, never inside another value, never typed
  (the engine's property handle refuses either), never compared as one of
  its members.
