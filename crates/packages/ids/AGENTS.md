# `axioval-ids`

The IDS package importer: translates a buildingSMART IDS 1.0 document into a
definition package and a ruleset. See `docs/src/ids.md` for the mapping of
every facet.

- Exact or not at all. Every facet it cannot translate exactly is reported as
  a `Gap`, never dropped. An untranslatable applicability facet leaves its
  whole specification without rules, because dropping it would widen the
  checked population; an untranslatable requirement leaves only itself out.
- Facets lower onto existing capabilities and selectors only. A facet no
  selector states exactly is checked by an auxiliary rule the
  specification's rules select through `ruleOutcome`; a prohibited facet no
  capability negates by an auxiliary rule and a rule failing what it passed;
  a facet no capability decides exactly is a gap.
- An entity facet naming a class whose instances are no session objects
  (`IFCMATERIAL`, a relationship) translates to its `entityType` selector,
  which selects the model's resource objects of that class; only a part-of
  whole no traversal reaches stays a `NotAnObject` gap.
- Depends on `axioval-ir` (the package contracts), `axioval-engine` (the
  capability descriptors, so definitions follow every new parameter or
  column), `axioval-rules` (XML Schema pattern translation only),
  `openbim-ids` (the reader) and `ifc-schema` (which IDS classes are
  occurrences, per release). Never a source adapter: the architecture gate
  exempts exactly `openbim-ids` and `ifc-schema` for this crate
  (`PERMITTED_COUPLINGS` in `scripts/architecture.py`). The facade is a
  path-only dev-dependency, left out of the published manifest.
- Run `cargo test -p axioval-ids`. The conformance harness needs the
  buildingSMART corpus, which is CC BY-ND 4.0 and not vendored:
  `IDS_TEST_CASES=<IDS>/Documentation/ImplementersDocumentation/TestCases cargo test -p axioval-ids -- --ignored corpus`.
  It asserts that no translated rule fails a `pass-` case and that every
  unflagged `fail-` case is explained by a reported gap. `./scripts/check.sh
  test` runs it when `IDS_TEST_CASES` is set. Keep it ignored by default.
- `cargo run -p axioval-ids --example coverage -- *.ids` ranks the gaps of
  real documents.
