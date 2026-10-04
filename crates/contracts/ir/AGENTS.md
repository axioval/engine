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
- `RuleSetPackage::groupings` (`GroupingDefinition`: `members` selector
  and a `GroupingKey`, `property`, `classification` or `compartment` with
  its separators, boundary and optional adjacency tolerances) and
  `Selector::DerivedGroup` are omitted when empty / absent so rulesets
  serialize as before; `GROUP_SET` is a derived set (`is_derived_set`).
  The MCS ruleset and selector schemas must mirror them.
- `RuleSetPackage::relations` (`RelationDefinition`: `from` and `to`
  selectors and a `RelationKey`, `property` with a `RelationProperty` per
  end, `pairs`, a table value of text `from`/`to` cells with an optional
  external id `scheme`, or `supplied`, pairs the host supplies at check
  time with optional `TableFileColumn`s and `scheme`, never listed in the
  package) is omitted when empty, so rulesets serialize as before. The
  MCS ruleset schema must mirror it.
- `ParameterValue::TableFile` names a table parameter's rows in a package
  data file (`path`, optional xlsx `sheet`, `sha256`, declared
  `TableFileColumn`s with optional `header` and a quantity's `unit`). Its
  loaded `rows` are `#[serde(skip)]`: the wire form is the reference only,
  so loading never changes how a package serializes. The MCS parameter
  value schema must mirror the reference.
- `ClassificationDefinition::classes` makes a classification hierarchical
  (ids, optional codes, localized names, optional parents); empty is
  omitted, so flat classifications serialize byte-identically. Keep tree
  validation, levels and ancestry in `contract::ClassTree` and the
  `<id>;level=<n>` name in `contract::ClassificationProperty`, the one
  place each is decided. The MCS classification and the `derivedClass`
  selector must mirror these fields.
- A decision's `comments` is a thread. Its wire form is a compatibility
  contract: exactly one comment by the decision's author at its date and
  without `id` is written as the legacy `comment` string, anything else as
  `comments`; `comment` and `comments` together are refused. Keep
  `thread_to_wire`/`thread_from_wire` the only place that decides this.
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
- `contract::Expression` is the expression contract: tagged by `kind`,
  `deny_unknown_fields`, literals only of `ScalarValue` kinds. Add a kind
  to `Expression::KINDS`, `kind()`, `children()` and a golden fixture in
  `tests/fixtures/expression` together; never add a loop, recursion, a
  user-defined function or anything that runs package code. The MCS
  expression schema must mirror it.
- `measured.rs` is the registry of `axioval:measured` values: one
  `MeasuredDescriptor` per name, sorted, labelled in English and German.
  Register a new measured value there (and nowhere else), with its
  parameters, dimension, services, exactness and not-evaluated causes.
