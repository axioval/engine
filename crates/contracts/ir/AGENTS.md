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
  on the wire when unset, so reports serialize as before. So are a
  decision's `assigned_to`, `due_date`, `priority` and `labels`; blank ones
  are refused (`DecisionError::Blank`), and `FindingDecision` carries them
  unchanged.
- `RuleFolder::annotations` is provenance only (namespaced keys, text
  values, omitted when empty): nothing in the engine may read it or let it
  change a selection, evidence or outcome. `axioval-ids` keeps an IDS
  specification's origin there so it can be exported again. The MCS rule
  folder must mirror this field.
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
