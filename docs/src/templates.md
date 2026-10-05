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
([#278](https://github.com/axioval/engine/issues/278)); `triangle-count`
and `plan-area` follow ([#282](https://github.com/axioval/engine/issues/282)),
and `property-predicate` is the first of the generic judges
([#287](https://github.com/axioval/engine/issues/287)).

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
| `grades` | Whether findings are graded (the descriptor's `grades_deviation`): each states how far its value misses the bound it fails, measured from the declared bound, never the widened one, for the runtime's severity bands. |
| `name` | How rule-scoped messages name the capability (`body-extent: …`). |
| `defaults` | Values optional parameters take when unstated (`tolerance` 0 m). |
| `declaration` | `Check`s over the rule's parameters, in order: `choice` (a string among options), `length` (a non-negative length), `count` (an integer of at least zero, stated where the descriptor requires it), `kind` (a parameter of its descriptor's kind, placed where the capability read it so refusals keep their order), `nonNegative` (numbers of at least zero), `traversal` (a valid `relationship` or `path`, declared only with one of the named parameters), `exclusive`, `anyOf`, `requires`, `ordered` (numbers, integers or quantities, as the descriptor types them). The first failing check leaves the rule not evaluated as an invalid declaration, worded as the capability worded it. |
| `services` | The host services the values need (`Service`: `object-frame`, `vertical-extent`, `triangle-count`), and the message leaving the whole rule open without them, before anything is selected. |
| `texts` | Named message parts, optionally conditional: `Condition::Positive` (a parameter above zero), `Condition::Inexact` (a value read from evidence that is not exact, such as a count of a tessellation), `Condition::Equals` (a string parameter or its default is a value). Several texts may share a name, each under its own condition: the first that holds is rendered (`plan area` or `facade area` by `measure`). |
| `forms` | The compositions. The first form whose `when` parameters are all stated applies. |

A `Form` holds:

- `values`: `TemplateValue`s, each a named expression, read in order for
  each selected object. A value may `expect` a kind (`length`), and words
  a stated absence (`absent`) and a value of the wrong kind (`mismatch`);
- `decision`: how the values decide;
- `fail` and `undecided`: the finding's message, and the message of an
  object the values cannot decide;
- `members`: where the form judges anchors through members (`Members`):
  the selector parameter picking them and what undecided members leave;
- `table`: the report table it fills (`Table`: a name and columns, each a
  value under an id that may name a text), one row per selected object
  whose values were read, passing or not.

**Members.** A value reads an anchor's members as an aggregate over
`Members::source(selector)`, the objects the selector parameter picks: in
the catalogue it reads as written. Run, each anchor's members are its
reach (`counts::tally`, the one reader `related-count` and `keyed-limit`
share): the objects the selector picks that the rule's `relationship` or
`path` reaches from the anchor or, without one, every such object of the
anchor's own source but the anchor. The aggregate is narrowed to the
members surely picked, findings relate them, and their traversal evidence
is cited. Members the selector cannot decide leave the anchor as
`UndecidedMembers` says: `onlyExcess` (they can only add, as areas do)
keeps only a value surely above its maximum and leaves the anchor
otherwise not evaluated with its message (`{undecided}` the count,
`{relation}` how they are reached), and the anchor has no row: its value
is known only from below. A member's value that cannot be read leaves the
anchor open worded as the member's measured value refused it.

`Decision::Within` is the generic range judge: a value between a minimum
and a maximum, each a left-to-right sum of values and parameters
(`Term`), widened by `ROUNDING_ULPS` (four units in the last place) of the
largest `rounding` magnitude, an end of a value's or a parameter's
interval (`Magnitude`); without a magnitude (a count) the bounds are not
widened, and the expression form compares with them as they are. A
verdict needs the whole interval on one side of a bound, a straddling one
is undecided naming the bound. Each bound is decided by the one comparison
every rule uses (`axioval_engine::comparison::numbers`, through
`plan_area::judge`); only the bounds' arithmetic, a sum widened by the
allowance, stays plain binary, as the capabilities computed it, rather
than the evaluator's sound interval arithmetic, which rounds a widened
bound outward by an ulp and would leave open a value the capability
decided (see [Expand and fork](#expand-and-fork)).

`Decision::Compare` is the generic comparison judge: the stated value of
`value` against the target a rule names by an operator word
(`Comparison`): the operator parameter, the words testing presence, and
the target parameters in the order a statement is checked, each with its
kind (`TargetKind`) and the operator words it admits (`Operation`: an
order, `contains`, `matches`, `oneOf`, `noneOf`), and the parameters
folding case, stating a date precision and declaring a tolerance. Binding
refuses a statement in the order the capabilities refused it: exactly one
target (none for a presence word), an operator the target's kind admits,
`precision` for a date target only, a `matches` pattern that compiles, a
tolerance for a numeric target only. Per object it judges what the source
states, through the one comparison: an absence, `null` and a value of
another kind fail every operator but a presence test (which reads blank
text as undefined), a quantity against a unit-less target (or the
reverse) is not evaluated.

### `property-predicate`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| one | always | `actual` = the stated property `{property_set}`.`{property}` | `actual` compared by `Compare`: `operator` against one of `value` (integer), `number`, `quantity`, `date`, `date_time`, `boolean` (equality), `texts` (`one_of`, `none_of`) or `text` (`equal`, `not_equal`, `contains`, `matches`); `is_defined`/`is_undefined` take none; `case_sensitive`, `precision` and the tolerance parameters as declared |

Its finding reads `property {property_set}.{property} does not satisfy
{operator}{target}{tolerance:suffix}; actual value is {actual:stated}`. A
target computed per object (`per_object`) is evaluated to a literal per
object first (`object_parameters::per_object`), as before.

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

### `triangle-count`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| maximum | always | `count` = `triangle_count` | `count` within no minimum and `maximum`, no rounding |

The declaration is one `count` check (`maximum` stated, at least zero);
the service is `triangle-count`. The finding's message appends the text
`tessellated` (the count depends on the host's tessellation) where the
count's evidence is not exact (`Condition::Inexact`), as the capability
said so on a tessellation of curved faces. The finding cites the measured
`triangle_count`, whose evidence carries the host's count locator and its
exactness.

### `plan-area`

| Form | When | Values | Decision |
| --- | --- | --- | --- |
| members | `member_selector` | `area` = the sum over the members of `plan_area;measure={measure}`, in m² | `area` within `minimum` and `maximum`, no rounding |
| own | always | `area` = `plan_area;measure={measure}`, in m² | the same |

`measure` defaults to `footprint`; the texts `noun` (`plan area` or
`facade area`) and `column` (`plan_area` or `facade_area`) follow it. The
declaration checks the bounds' kinds, that one is stated
(`minimum or maximum is required`), that none is negative, their order,
the member selector's kind, the traversal (only with `member_selector`)
and `measure`, in the capability's order. The template grades, fills the
table `areas` with the area in the column `{column}`, and judges anchors
through their members with `onlyExcess`. An area is read as a plain number
of square metres (the measured area divided by 1 m²), as the bounds are
stated, so the forked rule compares like with like. The sum of members is
the evaluator's exact interval sum: where the binary sum rounds, its
interval holds the capability's rounded sum and is a unit in the last
place wider (D19).

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
`{area:area}`: `26` or `between 24 and 26`, rounded to 1e-4 as the area
capabilities showed areas; `{target:stated}`: the value as the source
states it), `{bound}` (the bound a `Within` failed or straddled, a length:
`at least 0.26 m`), `{bound:plain}` (the bound as declared: `at least 6`),
`{why}`, and an anchor's `{undecided}` and `{relation}`; a `Compare`
form also reads `{target}` (the stated target as the rule declares it,
after a space: ` 250 mm`, `` `F30` ``, ` [F30, F90]`) and
`{tolerance:suffix}` (` (within tolerance 0.01)`, nothing when exact).
Lengths are shown rounded to the micrometre, as the capabilities showed
them.

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

A form judging members forks into aggregates along the rule's traversal:
the `path`, or the `relationship` in its `direction`, filtered by the
member selector. Members everywhere in an anchor's source, a followed
chain and skipped absent ends have no aggregate path, and such a rule is
not forked (`ForkError::Inexpressible`, naming why). An aggregate counts
an undecided member as possibly there, where the template leaves the
anchor open unless it already exceeds its maximum (D7): the fork's
verdict is sound but may decide an anchor the template leaves open.

A `Compare` form expands into an `if` over the operator words, each
branch the test its word names with the target read as a `parameter`. A
rule forked from it states only its own test (`compare`, `oneOf`,
`noneOf`, `isDefined`, `isUndefined`) with its target a literal. The fork
reaches the template's verdicts wherever the value is of the target's kind
or absent (an absence is a missing-information finding, a finding as the
template's); a value of another kind, which the template fails, leaves the
fork open, and blank text is defined to it. A declared tolerance or date
precision has no expression form, so such a rule is not forked
(`ForkError::Inexpressible`).

## Export

A rule bound to a template is still a rule of that capability with its
parameters, so every export profile writes it, or refuses it, exactly as
before the rebuild: IDS refuses `body-extent` and `property-predicate` as
capabilities it has no facet for. A forked rule is an `expression` rule: a profile exports it only
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

`triangle-count` is held the same way: every fixture of
`crates/engine/rules/tests/triangle_count.rs` and generated meshes of
random count and exactness against
`axioval_rules::reference::TriangleCountLimit` under `Parity::contract()`,
each object's count compared exactly; the `element-triangles` rules of
the cases `elements` and `unmeshed` recorded before the switch (the
rule-scoped outcome without the service, D4); and the fork.

`plan-area` is held so too: every fixture of `plan_area.rs` and
`storey_metrics.rs` runs the template and
`axioval_rules::reference::PlanAreaRange` under `Parity::contract()`
(`Model::holding_contract`, the tests' shared helper), the `areas` table's
values included; generated spaces and storeys of random footprints,
slack, membership and bounds likewise; the `plan-area` rules of the case
`storeys`, recorded before the switch, with their tables; and the fork,
on every fixture whose members are decided.

`property-predicate` is held to its replaced implementation
(`property_predicate/reference.rs`,
`axioval_rules::reference::PropertyPredicate`) the same way: every fixture
of the rules crate that runs it (its own tests and those of dates,
tolerances, quantities, complex and unreadable values) goes through
`common::predicate`, which runs both under `Parity::contract()`;
generated predicates (every operator word, target kind, text option,
tolerance and precision against values of every kind, absent, `null`,
lists, measured intervals and complex properties included) do too; the
`elements` case records `wall-width` and `slab-depth` on every public
model; and its fork reaches its verdicts on values of the target's kind.

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
  expression form for expand and fork. Decide every comparison through
  `axioval_engine::comparison`, the one implementation the evaluator, the
  selectors, the judges and `Within` share (#287); `Compare` is the
  comparison judge the property judges build on.
- **Grading** (built, `Template::grades`). A decision reports the
  deviation from the declared bound, never the widened one; severity bands
  stay the runtime's.
- **Scopes and related objects** (members built, `Form::members`). An
  anchor judged through the members a selector parameter picks, read as an
  aggregate over them, findings relating the decided members; further
  `UndecidedMembers` policies, and members read one by one (a storey's
  levels, a space's doors), are still to come.
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
- **Report tables** (built, `Form::table`). A template whose capability
  reports a table states its columns as values to tabulate, so the table
  stays a report contract; a column id may name a text.

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
