# Capability templates

A built-in capability is a named, parameterized rule type: packages, the
[authoring catalogue](./catalogue.md) and the [export profiles](./export.md)
address it as one rule, and external rule formats know many of them as one
rule type each. Inside, a capability is being rebuilt from the shared parts
of [composed rules](./composing-rules.md): measured values, expressions,
aggregates, generic judges and, where needed, a search. A **template** is
that rebuild: the capability's outside contract, kept unchanged, and a
composition the engine evaluates with the same evaluator as any
`expression` rule, so an improvement to a part improves every template
that uses it.

Templates are engine-owned and built in. A package never supplies one,
and nothing in a template runs package code: it is data the engine reads,
like an expression.

`body-extent` is the first capability that runs as a template
([#278](https://github.com/axioval/engine/issues/278)).

## The outside contract

A template keeps, byte for byte:

- the capability id and parameter descriptor (names, kinds, requirement,
  grading, authored parameters), pinned for every built-in by
  `crates/engine/rules/tests/descriptors.rs`;
- how a rule's parameters are checked, and the rule-scoped outcome of a
  declaration it refuses or of a host missing its services;
- every finding and not-evaluated outcome: its scope, reason, count,
  wording word for word, evidence exactness and related objects.

The [parity harness](./parity.md) holds a template to all of it under
`Parity::contract()`.

## The template form

`axioval_engine::template::Template` is plain data, built in Rust by the
rules crate (`body_extent/template.rs`):

| Field | What it states |
| --- | --- |
| `id`, `parameters` | The capability's id and descriptor, unchanged. |
| `name` | How rule-scoped messages name the capability (`body-extent: …`). |
| `defaults` | Values optional parameters take when unstated (`tolerance` 0 m). |
| `declaration` | `Check`s over the rule's parameters, in order: `choice` (a string among options), `length` (a non-negative length), `exclusive`, `anyOf`, `requires`, `ordered`. The first failing check leaves the rule not evaluated as an invalid declaration, worded as the capability worded it. |
| `services` | The host services the values need (`Service`), and the message leaving the whole rule open without them, before anything is selected. |
| `texts` | Named message parts, optionally conditional (`Condition::Positive`). |
| `forms` | The compositions. The first form whose `when` parameters are all stated applies. |

A `Form` holds:

- `values`: `TemplateValue`s, each a named expression, read in order for
  each selected object. A value may `expect` a kind (`length`), and words
  a stated absence (`absent`) and a value of the wrong kind (`mismatch`);
- `decision`: how the values decide;
- `fail` and `undecided`: the finding's message, and the message of an
  object the values cannot decide.

`Decision::Within` is the generic range judge: a value between a minimum
and a maximum, each a left-to-right sum of values and parameters
(`Term`), widened by `ROUNDING_ULPS` (four units in the last place) of the
largest `rounding` magnitude, an end of a value's or a parameter's
interval (`Magnitude`). It decides in plain binary arithmetic, as the
capabilities judged: a verdict needs the whole interval on one side of a
bound, a straddling one is undecided naming the bound.

### `body-extent`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| stated length | `target_property` | `extent` = `body_extent;axis={axis}`, `low`/`high` = `body_position;axis={axis};end=low\|high`, `target` = the stated property, a length | `extent` within `target − tolerance` and `target + tolerance`, rounding over `low`, `high`, `target` and `tolerance` |
| range | always | `extent`, `low`, `high` | `extent` within `minimum` and `maximum`, rounding over `low`, `high` and `extent` |

`body_position` is a measured value of its own: where the body begins and
ends along its axis, the magnitudes the binary rounding of the extent
scales with. The capability allowed that rounding all along; the template
states it, so a wall exactly at a bound far from the origin is judged as
before.

## Binding and running a rule

The rules crate's `templates` module runs a template
(`templates::run`, behind the capability's `RuleCapability::evaluate`):

1. **Bind.** The declaration checks run over the rule's parameters. Each
   stated parameter becomes a constant, a quantity converted to coherent
   SI units exactly as the capability converted it, then the defaults
   apply and the form is chosen. Every slot in a string field of the
   form's expressions (`{axis}`, `{target_property.set}`,
   `{target_property.name}`) is filled and every `parameter` read of a
   constant folded into a literal. The result is the rule's plan: the same
   expressions an equivalent hand-written `expression` rule would hold.
2. **Check the services**, and leave the rule open without them.
3. **Select** the objects as every capability does (`select_objects`).
4. **Read the values** of each object with the shared evaluator, through
   the same leaves an `expression` rule reads (`ObjectLeaves`): stated
   properties through property resolution and concept binding, measured
   values through the registered providers. The first value that cannot be
   read leaves the object not evaluated, for the reason its leaf gives and
   worded as the measured value refused it (`{why}`); a `null` value is a
   missing-information finding (`absent`); a value not of the expected
   kind is invalid evidence (`mismatch`).
5. **Decide** and word the outcome with the form's messages.

### Messages

A message is a template of placeholders: a text's name, a parameter's
name (as stated; a property reference as `set.name`), a value with a
format (`{extent:length}`: `0.3 m`, or `between 0.48 m and 0.52 m`;
`{target:stated}`: the value as the source states it), `{bound}` (the bound
a `Within` failed or straddled, `at least 0.26 m`) and `{why}`. Lengths are
shown rounded to the micrometre, as the capabilities showed them.

## Expand and fork

**Expand.** The catalogue's entry of a templated capability carries
`template`: the template itself (declaration, services, texts, forms with
their values, decisions and messages) and `requirements`, each form as one
expression (`Form::requirement`: the decision's expression form with every
value inlined) with its [block tree](./block-editors.md). Its expressions
still hold the slots and `parameter` reads; a block editor shows them as
written.

**Fork.** `axioval_rules::templates::fork(capability, rule)` copies a rule
bound to a template into the `expression` rule it composes
(`Fork::CAPABILITY`, `Fork::parameters`): the form's requirement with the
rule's parameters bound in, every measured read canonical (a `measured.`
block with its parameters as fields). It is the starting point for a
stricter or extended rule. The fork reads the same values through the same
evaluator and reaches the template's verdicts on every fixture
(`the_forked_rule_reaches_the_templates_verdicts`), with two differences
that are its own: its findings are worded as an `expression` rule's, and
its bounds are decided by the evaluator's sound interval arithmetic, which
may leave open a value within a unit in the last place of a bound's
rounding allowance that the judge decides.

## Export

A rule bound to a template is still a rule of that capability with its
parameters, so every export profile writes it, or refuses it, exactly as
before the rebuild: IDS refuses `body-extent` as a capability it has no
facet for. A forked rule is an `expression` rule: a profile exports it only
where its `expression_kinds` state every node, and otherwise refuses it
naming the first node it cannot state (IDS: the subtraction of the
rounding allowance). `crates/packages/ids/tests/export.rs` pins both.

## Proving the rebuild

`body-extent` is held to the implementation it replaced in three places:

- **Fixtures and generated inputs.** Every fixture test of
  `crates/engine/rules/tests/body_extent.rs` runs the template and the
  replaced implementation and compares them under `Parity::contract()`,
  each wall's measured extent included; so do the generated walls of
  random size, heading, slack and stated thickness. The replaced
  implementation stays in `body_extent/reference.rs`, compiled only with
  the rules crate's `parity-reference` feature (its own tests turn it on
  through a dev-dependency on itself) and exported as
  `axioval_rules::reference::BodyExtent`. It is no capability of any
  registry: the id resolves to the template.
- **Public models.** The cases `elements` and `unmeshed` list the
  `body-extent` rules as `recorded`: their outcomes on every pinned model,
  as the replaced implementation reported them, are stored beside the case
  and compared with the template's (see [Parity harness](./parity.md)).
  The template reproduces the rule-scoped outcome without geometry (D4,
  D18).
- **The fork.** The forked rule reaches the template's verdicts.

The reference is kept, rather than deleted, because generated inputs need
a live implementation to compare with; recorded outcomes outlive it on the
public models. A rebuild retires its reference once its family's
templates are proven (the rebuild issue decides), deleting the module and
its call sites' entries in `scripts/measurement_ledger.json`, while the
recorded outcomes and the fixtures' literal messages remain.

## Extension points for the rebuilds

The rebuilds of #280–#287 extend the form where a family needs it, each
addition data the runner interprets, never code per capability:

- **Decisions.** `Decision` is an enum: add a variant for a judge a
  family shares (a truth `Requirement` expression over the values, worded
  by which labelled operand failed; counts; consistency), with its
  expression form for expand and fork. Keep one implementation of each
  comparison (#287).
- **Grading.** A template that grades states it in `Template` (the
  descriptor's `grades_deviation`) and a decision reports the deviation
  from the declared bound, never the widened one; severity bands stay the
  runtime's.
- **Scopes and related objects.** Findings are per selected object today.
  A family judging an anchor through members (a storey's levels, a space's
  doors) adds a form scope and names related objects from a value (an
  aggregate's members, a measured member list).
- **Several findings per object.** A capability reporting one finding per
  failed check needs one decision per check in a form, each with its
  messages, so counts match (divergence D1).
- **Tables and defaults.** Table parameters reach values through `lookup`;
  fallbacks (a stated value, then a table, then a declared default, each on
  an exact absence only) are a value list read in order, the first that is
  not absent winning, and every default used is worded and cited.
- **Searches.** A search stays Rust: a measured value (or member list)
  registered by a provider beside the capability, which a value reads by
  name. Its results become values like any measurement.
- **Messages.** Add a placeholder format where a family words values
  another way (areas, angles, counts), in `templates.rs`.
- **Services.** Add a `Service` variant for each host service a template
  needs before selection.
- **Report tables.** A template whose capability reports a table states
  its columns as values to tabulate, so the table stays a report contract.

## Performance

A template must stay within the budget of the code it replaces (#279).
Nothing here precludes the plan #279 compiles: binding already folds the
parameters into constants and fills every slot, so a plan is built once per
rule and evaluated per object. Still to come there: caching the bound plan
per compiled rule instead of per evaluation, and memoizing measured values
per object and call for the run. `body-extent` reads three measured values
per object (`body_extent` and both `body_position` ends) where the
capability measured once; memoizing the directional extent per object and
axis in the provider brings it back to one service call.

## Rebuilding a capability as a template

1. Pin the contract: the capability's entry in
   `tests/golden/descriptors.json` must not change, and its fixture tests
   assert their messages literally.
2. Make sure every measurement the capability takes is a registered
   measured value (`scripts/measurement_ledger.json` maps each call site),
   including any magnitude its rounding allowance scales with.
3. Write the template as data in `<capability>/template.rs`: descriptor,
   defaults, declaration checks in the capability's order and wording,
   services, texts, forms with values, decision and messages.
4. Move the implementation to `<capability>/reference.rs` behind
   `parity-reference`, export it from `axioval_rules::reference`, and let
   the capability's type run the template (`templates::run`, returning the
   template from `RuleCapability::template`). Rename its ledger entries to
   the reference module.
5. Run every fixture under `Parity::contract()` against the reference,
   measured values included, and the generated inputs likewise; fork each
   fixture's rule and compare outcomes.
6. List the capability's rules in a public parity case as `recorded`,
   record them with `AXIOVAL_PARITY_RECORD=1` **before** switching, then
   switch and compare. Record any divergence with its reason and decision.
7. Bless the catalogue (`AXIOVAL_BLESS=1 cargo test -p axioval-rules --test
   catalogue`); never the descriptors.
8. Document the template here and in the capability's section.
