# `axioval-ids`

The IDS package importer: translates a buildingSMART IDS 1.0 document into a
definition package and a ruleset, and exports the rules IDS states exactly
back as a document. See `docs/src/ids.md` for the mapping of every facet.

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
- `Options::filter` is a prefilter joined to every specification's
  applicability through `allOf` (`bind_filter`). It is written in IFC names
  and bound to concepts per specification exactly as the specification's
  own names are; derived-set names, paths, patterns and source facts pass
  through, and `ruleOutcome` is refused up front. `None` must leave the
  translation byte-identical.
- `export` (`src/export.rs`) writes rules back as IDS, exact or not at all:
  a rule, or a translated folder, is exported only when translating the
  specification it reads as gives it again, compared as `canonical` states
  it (capability, parameters with defaults, applicability, gates, grading;
  concepts by the names they bind to, rule ids by position; presentation
  left out). Never add a reading without that check, and never let a
  refused rule pass silently: each gets a `Refusal`. A folder `translate`
  writes keeps its specification in the `ids:specification` annotation
  and the root the `<info>` under `ids:info.*`; changing what `translate`
  writes for a facet must keep the corpus round trip passing.
- `src/write.rs` is the only IDS writer, a stand-in until `openbim-ids`
  writes IDS (openbimrs/ids#10): keep it behind `document`,
  `specification` and `read_specification` so switching changes only
  them. `ids.xsd` is CC BY-ND 4.0 and not vendored.
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
  unflagged `fail-` case is explained by a reported gap, and that every
  complete translation round-trips through `export` (same specifications,
  packages and findings; its rules one by one too), validating each export
  against the checkout's `Schema/ids.xsd` with `python3` and `lxml`.
  `./scripts/check.sh test` runs it when `IDS_TEST_CASES` is set. Keep it
  ignored by default.
- `cargo run -p axioval-ids --example coverage -- *.ids` ranks the gaps of
  real documents.
