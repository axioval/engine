# Parity harness

A built-in capability moves onto shared parts (measured values, expressions,
generic judges) only once its re-expression judges every model as the
capability does. The differential parity harness, `axioval_rules::parity`,
proves that. It runs the capability and its re-expression over the same
model and lines their outcomes up scope by scope: every object, every
source and the project. Two kinds of re-expression are compared:

- an **expression rewrite**, one or more `expression` rules that reach the
  capability's verdicts but word their own findings;
- a **template**, a capability rebuilt as a preconfigured composition
  ([#278](https://github.com/axioval/engine/issues/278)), which must keep
  the capability's whole outside contract: the same findings, wording,
  related objects and grades.

## Reading a side

`Observations` is everything one side reported:

- `Observations::of_report(report, rule)` reads one rule of a report: its
  findings and not-evaluated outcomes at every scope, its report tables,
  and its rule summary when the run made one
  (`Runtime::with_rule_summaries`).
- `Observations::of_recorded(report, outcomes, rule)` reads the same and
  the rule's selection, from a run that recorded every rule's selection
  (`Runtime::run_session_recorded`, or `run_recorded` over a bare
  project). An object the rule selected and reported nothing about
  *passed*; one it neither selected nor reported about was *not selected*;
  one whose selection was undecided is *undecided*.
- `Observations::of_evaluation(evaluation)` reads a capability evaluation,
  as fixture tests run capabilities directly, with each graded finding's
  deviation. `.selecting(objects)` states its selection where the test
  knows it.
- `.merge(other)` joins the sides of a capability rewritten as several
  rules (one per check, or one per severity band).
- `.with_value(scope, name, measure)` adds a value the caller measured,
  such as the measured value a template reads, beside a measurement the
  capability makes; `.retain_values(keep)` keeps only the values both
  sides measure by contract.

A report table's cells become values named `<table>.<column>` (a grouped
row's `<table>[<group>].<column>`). A `Measure` is a number interval in a
unit with its exactness where known, a text, `null` (stated absent) or not
evaluated; `null` and not evaluated never agree.

## What must agree

Every scope's verdict always must: finding (with the most severe
severity, and whether every finding's evidence is exact), not evaluated
(with its reason), passed, not selected or undecided. Passed and not
selected are told apart wherever both sides know their selection, and read
as one ("reported nothing") where either does not. A source or the
project is never selected: about those a rule reports or it does not.

A `Parity` adds what else must agree where two scopes' outcomes do:

| | `Parity::outcomes()` | `Parity::contract()` |
|---|---|---|
| for | an expression rewrite | a template |
| finding count per scope | yes | yes |
| categories | yes | yes |
| messages, word for word | no | yes |
| related objects | no | yes |
| graded deviation | no | yes, to a few units in the last place |

`.uncounted()` drops the count, for a rewrite that reports one finding
where the capability reports one per failed check (divergence D1);
`.value(name, step)` compares the measured value `name` of every scope
within its declared rounding: both interval bounds may differ by at most
`step`, in the value's unit, and a value one side has and the other lacks
differs. Values are compared whatever the outcomes. Two rules whose
summaries differ (how many objects each checked, or its status) differ
about the project.

`parity.compare((capability, &left), (expression, &right))` returns
`ParityEvidence`: the number of scopes compared, how many the capability
found and left open, how many values were compared, and every
`Difference`, which names the scope, both outcomes and what else differs
(`Detail`: count, categories, messages, related objects, deviation, the
rule as a whole, a value). `holds()` is true when there is none; `diff()`
prints one line per scope. The free functions `compare`,
`compare_recorded` and `compare_evaluations` compare under
`Parity::outcomes()`.

## Registering a comparison

A rebuild issue registers its comparison in three places.

**Fixtures.** Beside the capability's fixture tests, run the capability
and its template on the same model and compare under the contract,
measured values included. Once the template replaces the capability, the
replaced implementation stays reachable for this only behind the rules
crate's `parity-reference` feature (`axioval_rules::reference`); see
`body_extent.rs`'s `run`, which holds every fixture so:

```rust,ignore
use axioval_rules::parity::{Observations, Parity};

let capability = model().evaluate_with(&StairGeometry, &rule, register);
let template = model().evaluate_measured(&template_capability, &rule, register);
// Each flight's rise as the template reads it, and as the capability
// measured it (from its own measurement, here a helper of the test).
let rises = model().measure("flight_rise", &flights, register);
let template = rises.into_iter().fold(
    Observations::of_evaluation(&template),
    |side, (flight, rise)| side.with_value(flight, "rise", rise),
);
let capability = capability_rises().into_iter().fold(
    Observations::of_evaluation(&capability),
    |side, (flight, rise)| side.with_value(flight, "rise", rise),
);
let parity = Parity::contract()
    .value("rise", 1e-6)
    .compare(("stair-geometry", &capability), ("template", &template));
assert!(parity.holds(), "{}", parity.diff());
```

`Model::measure` (in the rules crate's test support) reads a measured
value of each object as a run reads it. The capability's side gets its
values from its report tables, or from `with_value` where the test reads
the capability's own measurement.

**Generated inputs.** `proptest` (a test-only dependency) generates inputs
for the simple geometric capabilities: `body_extent.rs` generates walls of
random size, heading, measuring slack and stated thickness, and
`level_spacing.rs` buildings of random storey elevations, and each holds
the rewrite to parity, level heights compared with the `levels` table.
Add a `generated` module beside a rebuilt capability's rewrite tests the
same way.

**Public models.** A case directory under `fixtures/parity/cases` holds
`definitions.json`, `ruleset.json` (the capability's rule and its
re-expression's, side by side) and `parity.json`:

```json
{
  "models": "*",
  "pairs": [
    {"capability": "stair", "expression": "stair-template",
     "comparison": "contract", "values": {"levels.height": 1e-6}}
  ],
  "divergences": []
}
```

`models` is `"*"` or a list of pinned model names, `geometry: false` checks
without meshing, and a pair may set `comparison` (`outcomes` or
`contract`), `uncounted` and `values`. Every difference a pair shows on a
model must be recorded in `divergences` with the line the harness prints, a
reason and a decision; a recorded one no longer shown fails as well.

## Where it runs

- **Fixtures, in every gate.** The rules crate's tests, the parity
  module's own tests and the generated inputs.
- **Public models, in CI.** `fixtures/parity/models.json` pins openly
  licensed IFC models (the buildingSMART sample files, CC BY 4.0) by a URL
  fixed to a commit and their SHA-256; they are not vendored.
  `scripts/parity_models.py fetch` downloads them into
  `~/.cache/axioval/parity-models` (or `AXIOVAL_PARITY_MODELS`) and keeps a
  file only at its pinned digest. With `AXIOVAL_PARITY_MODELS` set,
  `./scripts/check.sh test` runs the CLI's `parity` test, which checks each
  case through `axioval check --geometry --rule-status` over each model.
  CI runs it in its own `parity` job, the models cached by the manifest's
  hash, and the `check` job requires it. Without the models a test still
  compiles every case against the registry.
- **Private models, locally.** Set `AXIOVAL_PARITY_CASES` to a directory of
  cases, each holding its own `.ifc` models beside `definitions.json`,
  `ruleset.json` and `parity.json`; `./scripts/check.sh test` then runs the
  facade's ignored `ifc_parity` test, in process, every selection
  recorded. Once the variable is set, a missing, empty or unreadable case
  fails rather than skips, and any difference fails.

**Recorded outcomes.** A template replaces its capability under the
capability's id, so a case cannot run both side by side. A case's
`recorded` entries (`{"rule", "reason"}`, with a pair's `comparison`,
`uncounted` and `values`) name such rules: the outcomes the replaced
implementation reported on each model are stored in the case's
`recorded/<model>.json` (the report restricted to those rules: findings,
not-evaluated outcomes, tables and rule summaries), and the rule as it runs
now is compared with them. `AXIOVAL_PARITY_RECORD=1` writes those files
from the current run instead of comparing: run it once, before the
implementation is replaced, and review the recordings like any other
change. A divergence of a recorded rule names it as `"recorded"` in place
of `"capability"`. The compile test checks that every recorded rule exists
and that a recording exists for every model the case names. The
`body-extent` and `triangle-count` rules of `elements` and `unmeshed` are recorded so, and
the `property-predicate` rules `wall-width` and `slab-depth` of `elements` (see
[Capability templates](./templates.md)). The case `storeys` holds only
recorded rules: `plan-area` (own footprints and facades, and storeys
summing their members), `area-ratio`, `plan-coverage`, `level-spacing`,
`slab-stack-spacing` and `coordinate-consistency` over every public
model, recorded before any of them is rebuilt as a template (#282). The
case `counts` holds `object-count` (per source and across sources),
`related-count` (along a relationship, and every object of the anchor's
source) and `unique-value` (per source, per storey and across sources)
rules over every public model, unmeshed, recorded before they are
rebuilt as templates (#287, #290). The case `judges` holds the remaining
generic judges, `consistent-value` (exact, within a number or quantity
tolerance, across sources and per storey), `selector-conformance`,
`relative-count` (ratios with small counts, tables, the whole source and
groups by value), `property-value` (ranges, literals, patterns over names
and digits), `property-requirements` (exact and pattern rows, grouped by
value under a category) and `property-comparison` (each, at least one,
count and sum, against constants, properties and text lists), over every
public model, unmeshed, recorded before they are rebuilt as templates
(#287). The
case `shelving` records two `shelf-capacity` rules (doors through stated
space boundaries, and through the derived adjacency among selected
spaces) before it became a template (#289). The case `stairs` records
`stair-geometry` and `ramp-geometry` rules over slabs, beams and storeys
standing in for flights, ramps and whole stairs (#280): the public models
hold none, so it records the walking-surface service's refusals and the
whole-stair mode's own outcomes. The case `coverage` records
`slab-contact` (against selected walls, columns and beams, and against
every object leaving out the bottom storey), `counterpart-coverage` (in
plan and height, and in the elevation with a frame's infill),
`effective-coverage` (grown and by travel) and `external-wall-validation`
over every public model, before they are rebuilt as templates (#282).
The cases `spans`, `proximity` and `numbering` record the plan-span,
distance and layout capabilities before they are rebuilt as templates
(#283): `spans` holds `centre-line-distance` (both sides, the nearest and
against a wall), `recess-width` (spaces, and slabs, walls and columns),
`light-well` (zones, storeys of spaces and storeys of their contents),
`exit-separation` (closest points, centres under an unknown flag, and
farthest points with a flag default), `wall-spacing` (walls with
footprints, beams) and `parking-bay` (sizes, orientation to aisles with
obstructions, and the filter mode); `proximity` holds `distance` (nearest,
none closer, at least a count, and overlapping in height within a
container), `containment` (counts with orphans, and cover bands over
combined outer elements) and `component-visibility` (at least two targets,
and none); `numbering`, unmeshed, holds `name-sequence` (storeys of a
building, spaces of a storey ordered by placement where they state no
elevation, elements along a relationship) and `numbering-consistency`
(per source, across sources and per storey).

The case `openings` records the door,
window and opening capabilities over every public model before they are
rebuilt as templates (#281): `opening-area` and `empty-host` on walls (both
side areas stated, one, one of another kind, a minimum opening area and
each face), `opening-zone` on the openings (margins, edges, zones,
dimensions, supports) and on beams through the derived intersections,
`keyed-limit` with each of its quantities (plan and member areas, stated
and measured values, sill heights, clear widths and heights, glazing
ratios, threshold steps, keys along the derived adjacency and pairs of
them, no row and a refused declaration), `door-swing`, `opening-spaces`,
`space-connection` and `corridor-end-openings`; the public models hold
one wall with an opening and a window and no door, so most door rules
record the object-frame and adjacency services' refusals.

The case `clashes` records `clash` and `clash-matrix` over every public
model before they are rebuilt as templates (#285): `clash` with axis and
volume tolerances, severities by class and graded by the smallest extent,
duplicates compared by stated values; clearances grouped per type pair and
storey; walls grouped as similar clashes, excluded along a connection and
by a shared presentation layer; subjects grouped, graded by the shared
volume and excused by tolerance cases along the elements' own axes; and
`clash-matrix` with cells picked by selectors, a cell's severity, unmatched
pairs reported, and cells keyed by a property and a discipline the public
models do not declare.
The case `floors` records the capabilities whose decisions become
templates over the free-space searches (#286) before they are split:
`free-floor-circle` (every other object an obstacle, furniture within a
band, a circle too large to fit, merged spaces, a path from the entrances,
door swings and an invalid band), `free-floor-rectangle` (furniture, a
rectangle too large and one without an orientation), `component-clearance`
on the furniture (a front box over the floor, both sides within the
space, any of three sides, a cylinder, a sliding volume, a supported area,
fixed sizes, a front against a wall, a size from the component's and an
invalid side) and `local-circulation` on the spaces (reaching, required
entrances, path ends, linked components, passing spaces and an invalid
declaration). The free-space search cannot map the public models' spaces
(their footprints repeat a vertex), so most rules record its refusals;
one area off the slabs is found unsupported and the spaces without an
entrance are found.

The case `routes` records the route capabilities before their decisions
become templates over their searches (#286): `space-distance` (straight,
between bodies, walking, on one storey, with direct access, to other
kinds of destination and an invalid row), `accessible-route` (from the
slabs and the furniture, through doors with obstacles and forbidden
stairs, with passing spaces, and an invalid declaration) and
`escape-route` (travel, a travel too short, exits, exit and passage
widths, clear heights, multiplied sections and an invalid use). The
public models' two spaces lie side by side without a door, so the bodies
are found too close, no destination has direct access, the exits are too
few, and the walks, the walkability snapshot and the footprints the
services cannot read leave the rest open.

The case `programmes` records the allocation capabilities before their
decisions become templates over their allocations (#286):
`group-composition` on the zones grouping the spaces (entries by name, a
shortfall, competing entries, a surplus, group keys with absent groups and
ungrouped spaces, and an invalid group cell) and `table-allocation` on the
spaces (rows by name, an extra, summed and individual areas, rows per
storey, the most specific row across sources, and an invalid anchor cell).

Each pair's evidence prints as one JSON line. That line is what the
migration ledger records as a proof item, tagged `"kind": "parity"`; the
ledger check (`scripts/migration.py`) rejects a parity proof that does not
name both rules, covers no scope, or records any difference.

## Recorded divergences

Every difference a test accepts is asserted exactly where it shows, and
classified here. A decision names what the rebuild of the capability must
do: the capability's outcome is the outside contract, so a template must
reproduce it unless that rebuild issue changes the contract on purpose.

| | Capability | Where | Difference | Reason | Decision |
|---|---|---|---|---|---|
| D1 | `counterpart-coverage`, `light-well`, `opening-zone`, `shelf-capacity`, `space-validation`, `stair-geometry`, `ramp-geometry` | their rewrites' parity helpers and `shelf-capacity`'s fork, compared `uncounted` | finding counts per object | The capability reports one finding per failed check (bound, margin, door, aspect, side, strip), or one graded finding where one rule per band finds every band a share exceeds; a rewrite reports one per rule. | Accepted for rewrites. A template is held to the contract, which counts: it reports the capability's findings one for one. |
| D2 | `level-spacing` | `an_unordered_building_leaves_its_storeys_open` | the building open against each storey open | A storey without an elevation leaves the capability unable to order the building's storeys; the rewrite judges storeys and leaves each open. | Accepted for the rewrite (both fail closed). The template (#282) reports on the building, the anchor the capability judges. |
| D3 | `recess-width` | `the_rows_as_an_expression_over_recesses_reach_the_verdicts` | open against passed | A depth on both sides of a row boundary leaves the capability's row open; the rewrite sees the width suffice under either row. | The rewrite's verdict is sound, every possible row agreeing. The template (#283) keeps the capability's outcome; deciding it is a contract change that issue must make explicitly. |
| D4 | `horizontal-guard`; `body-extent`, `triangle-count` on public models without meshing | `the_guard_decision_as_an_expression_over_edges_reaches_the_verdicts`; case `unmeshed` | the project open against each object or source open | The capability asks for its service once and leaves the whole rule open; the rewrite reads a measured value per object, which the runtime collapses per source. | Accepted for rewrites: the same fail-closed reason at another scope. Templates reproduce the rule-scoped outcome: `body-extent`'s does (#278) and `triangle-count`'s (#282), each held to the recorded outcomes of case `unmeshed`; `horizontal-guard`'s does too (#284), its guard surfaces measured once per rule. |
| D5 | `counterpart-coverage` | `a_share_straddling_a_higher_band_is_graded_milder` | warning against info | A share straddling a higher band: the capability grades by the most severe band it may reach, the band rules find only the band surely exceeded. | The template (#282) grades one finding with a `deviation` and the runtime's severity bands, which take the most severe reachable band, instead of one rule per band. |
| D6 | `area-ratio` | `a_window_stating_no_area_is_found_where_the_capability_leaves_it_open` | open against finding | A member stating no area leaves the capability unable to sum; an aggregate over it is `null`, so the rewrite requires every member to state one and finds the anchor. | The template (#282) leaves the anchor open, from a measured sum that is not evaluated on a member without an area. |
| D7 | `plan-area` | `undecided_members_are_measured_by_the_rewrite` | open against passed | The capability leaves an anchor with undecided members open unless its sum already exceeds the maximum; the aggregate measures them too, and a sum within the bounds either way passes. | Sound, as D3. The template (#282) keeps the capability's outcome (`UndecidedMembers::OnlyExcess`); a rule forked from it is the aggregate, and may decide such an anchor. |
| D8 | `slab-contact` | `an_undecided_counterpart_is_a_candidate_of_the_rewrite` | open against finding | Counterparts named by more than a kind cannot be a measured value's candidates: an undecided one leaves the capability's shortfall open, the rewrite finds it. | Accepted for the rewrite. The template (#282) binds the rule's selector (`with=@counterparts`): undecided candidates widen the share up to the whole face, and the shortfall stays open. |
| D9 | `distance` | `distance_expressions_hold_to_the_parity_harness_in_every_mode` | the counterpart open against nothing | The capability reports an unmeasurable counterpart itself as not evaluated; the rewrite judges its subjects only. | The template (#283) reports unmeasured counterparts as the capability does. |
| D10 | `exit-separation` | `intervals_counts_and_unknown_flags_reach_the_verdicts` | open against finding | An unknown sprinkler flag widens the capability's requirement to both fractions; the rewrite reads an unstated flag as unsprinklered. | The rewrite's default is not allowed in a template. The template (#283) reads the separation required as the measured `exit_separation`'s `required`, an interval over both fractions where the flag is unknown, and keeps the capability's outcome. |
| D11 | `centre-line-distance` | `the_centre_line_distance_as_a_value_reaches_the_verdicts` | inexact against exact evidence | With no wall within reach the capability cites its inexact side measurements, the rewrite the exact absence of a distance within reach. | The template (#283) cites the side measurements, so its evidence is as exact as the capability's. |
| D12 | `opening-area` | `the_measured_opening_area_judges_every_wall_alike` | open against finding | A wall stating only one side area is left open by the capability; the rewrite reads the missing side as `null`, a missing-information finding. | The template (#281) reads the area difference as a value that is not evaluated when one side is unstated. |
| D13 | `opening-area` | `the_summed_section_areas_judge_walls_of_separate_openings_alike` (fixture 6) | finding against passed | Summed one by one, section areas never check overlapping openings; the capability does. | The template (#281) uses `opening_area`, which holds parity but for D12, never the summed sections. |
| D14 | `opening-area` | the same test (fixture 13) | passed against finding | A section area knows no minimum, so a small hole the capability leaves out is counted. | As D13. |
| D15 | `keyed-limit` (`sill-height`) | `sill_heights_reach_the_verdicts_but_where_keys_disagree_or_a_floor_is_unknown` | finding against open | A floor that cannot be measured beside a failing one: any failing floor is the capability's finding, while the rewrite leaves the door open. | Kept: the template (#281) combines the floors (`Combined`), so a sure failure stands beside an unknown floor, held to the capability by `an_unmeasured_floor_is_not_evaluated_unless_another_floor_fails`. |
| D16 | `keyed-limit` (path keys) | the same test | open against finding | A key reached along a path that cannot be read as one value (the objects disagree, or there are none) leaves the capability open; the rewrite finds the door without a row. | Kept: the template (#281) reads such a key as unknown (`limit_row` refused), never as no row. |
| D17 | `level-spacing` | the generated buildings in `level_spacing.rs` | a table height against the measured rise | The `levels` table reports a level the check leaves out (the lowest with `ignore_lowest`, the highest) as unmeasured, where `level_rise` leaving it out states it absent. | Accepted: the table is informative only. Heights are compared on the levels the check judges; the template (#282) keeps the table as it is. |
| D18 | `body-extent` on a public model without walls | case `unmeshed` | the rule open against nothing selected | With nothing selected the capability still leaves the rule open for its missing services; the rewrite selects nothing and passes vacuously. | Accepted for the rewrite. The template (#278) reproduces the rule-scoped outcome, which never reads as a vacuous pass, held to the recorded outcomes of case `unmeshed`. |
| D19 | `plan-area` with `member_selector` | the generated storeys in `plan_area.rs` | the `areas` value and the graded deviation within a unit in the last place of the sum | The template sums the members with the evaluator's exact interval arithmetic: where a binary sum rounds, its interval holds the capability's rounded sum and is one unit in the last place wider. | Accepted: the interval is sound and holds the capability's value; fixtures compare the table within 1e-9 m² and deviations within 1e-12. A sum within a unit in the last place of a bound would be left open where the capability decided on its rounded sum; none of the generated inputs reaches one. |
| D20 | `property-comparison` | `a_signed_zero_equals_zero` | passed against finding | The capability ordered exact numbers by their bit pattern (`total_cmp`), so a stated `-0` was less than `0`; every other rule, the selectors and expressions compared them by value. | Changed on purpose by #287: the one comparison (`axioval_engine::comparison`) decides `-0` equal to `0` for every rule. Only signed zeros differ; no public model states one. |
| D21 | `area-ratio` | `a_sum_on_a_rounding_midpoint_is_shown_as_its_interval` | a finding's rounded areas and ratio | The template sums the members' areas as the evaluator sums any aggregate, an exact interval (D19). Where the binary sum lies within a unit in the last place of a rounding midpoint of the message (an area shown to 1e-2 m², a ratio to 1e-4), the interval holds both neighbours: the template shows its lower end, and the ratio as `between`, where the capability showed its rounded point. | Accepted, as D19: outcome, grade and table agree within a unit in the last place; only a message at a midpoint differs. The generated inputs keep to areas binary arithmetic sums exactly, and the fixture asserts both messages. |
| D22 | `area-ratio` | `an_empty_population_needs_no_service` | open with a missing service against open with no denominator area | An anchor reaching no member of a population whose areas are measured: the capability asked for the area service before summing nothing and left the anchor open without it; the template measures nothing and needs no service, so an anchor without any member is open for a denominator without area. | Accepted: both leave the anchor open; nothing is measured, so no service is needed. With the service registered, as on every public model, both agree. |
| D23 | `plan-coverage` | review of the `@candidate_selector` binding | open against a verdict | A candidate selector whose selection leaves a whole source undecided (its resource objects cannot be listed): the capability searched the candidates it could list; the template's search takes the selection bound as a measured value's argument, which refuses a selection it cannot list whole, and leaves every subject open. | Accepted: fail closed where candidates may be missing. No public model or fixture lists resource objects for the case's selector. |
| D24 | `ramp-geometry`, `stair-geometry` | not reached by a fixture | the selection's words and count | A selector parameter (`headroom_obstacles`, `landing_objects`, `handrail_objects`, …) whose source's objects cannot all be listed: the capability left each end, run or check open naming the selection (`headroom obstacle selection is undecided: …`), the template leaves each check open once, worded as binding the selection refuses it (``the objects `@headroom_obstacles` selects cannot all be listed: …``). | Accepted: both fail closed, for the same reason, on the same object. |
| D25 | `slab-contact` | review of the template's reads and `the_forked_rule_reaches_the_templates_verdicts` | open against a verdict, and the reverse | The template reads a face's storey before its contact, as the capability did, so a face whose storey cannot be placed is open even where its contact suffices; its fork, an `or`, passes it. The share is the sound interval of the contact area over the face, where the capability compared its rounded quotient: a quotient within a unit in the last place of the minimum, or of a band's threshold, is left open (or graded by the interval's end) where the capability decided on the rounded point. A counterpart selection that cannot be listed whole leaves every face open as the binding words it (the objects `@counterparts` selects cannot all be listed), after any storey refusal, where the capability said first that the counterpart selection is undecided. | Accepted: the fork's `or` is sound, and the template keeps the capability's outcome; the rounding needs a share within an ulp of a bound (no fixture, generated input or public model reaches one); an unlistable counterpart selection fails closed either way, only worded and ordered otherwise (no public model lists resource objects for the case's selectors). |
| D26 | `slab-stack-spacing` | `a_prevailing_distance_between_tessellated_slabs_is_cited_inexactly` | exact against inexact evidence | A slab whose own pair is exact, compared with the distance prevailing between tessellated slabs of its stack: the capability cited only its own pair; the template cites the prevailing distance it read as well, which is inexact. | Accepted: the verdict and its words agree, and the template's evidence states what the comparison rests on. The generated stacks keep every slab of a case exact or tessellated. |
| D27 | `counterpart-coverage`, `effective-coverage` | `a_subject_counts_every_near_counterpart_of_its_own_kind` | a finding against a pass | An element that is a counterpart (or a source) of another selected element (walls checked against walls): the capability's one plan broad phase over every subject reported each pair of two such elements once, from the one of lesser identity, so the other never counted it as a counterpart and found itself uncovered; the template's measured cover counts every counterpart (or source) whose plan box lies near each element. A counterpart selection that cannot be listed whole leaves each element open as the binding words it (``the objects `@counterparts` selects cannot all be listed: …``) where the capability judged the counterparts it could list. | Accepted: the cover no longer depends on which other elements are checked; an unlistable selection fails closed. No public model's case selects elements of the counterparts' kinds, and the generated walls keep the two apart. |
| D28 | `centre-line-distance` | not reached by a fixture | the selection's words | A `wall_selector` whose source's objects cannot all be listed: the capability left each object open saying `centre line: wall selection is undecided: …`, the template leaves it open as binding the selection refuses it (``centre line: the objects `@wall_selector` selects cannot all be listed: …``). | Accepted: both fail closed, for the same reason, on the same object; no public model lists resource objects for the case's selector. |
| D29 | `component-visibility` | not reached by a fixture | open against a verdict | A `targets` or `blockers` selector whose source's objects cannot all be listed: the capability judged the objects it could list and left the rest out; the template binds the selection into the measured view, which refuses a selection it cannot list whole, so each component is open. | Accepted: fail closed where targets or blockers may be missing; no public model lists resource objects for the case's selectors. |
| D30 | `external-wall-validation` | review of the bounding selections' binding | open against a verdict | A bounding or group selector whose selection leaves a whole source undecided (its resource objects cannot be listed): the capability derived the envelope around the objects it could list, the template's measured values take the selection bound as an argument, which refuses a selection it cannot list whole, and leave the derivation not evaluated (``the objects `@bounding_selector` selects cannot all be listed: …``). | Accepted: fail closed where a bounding object may be missing, as D23. No public model or fixture lists resource objects for the case's selectors. |
| D31 | `exit-separation` | not reached by a fixture | open against a verdict | An `exit_selector` whose source's objects cannot all be listed: the capability judged the exits it could list; the template binds the selection into the measured `exit_separation`, which refuses a selection it cannot list whole, so each space is open (``the objects `@exit_selector` selects cannot all be listed: …``). | Accepted: fail closed where an exit may be missing, as D29; no public model lists resource objects for the case's selectors. |
| D32 | `name-sequence` | not reached by a fixture | the selection's words | A `member_selector` whose source's objects cannot all be listed: the capability left each anchor open saying `member selection is undecided: its resource objects cannot be listed: …`, the template leaves it open as binding the selection refuses it (``the objects `@member_selector` selects cannot all be listed: …``). | Accepted: both fail closed, for the same reason, on the same object, as D28; no public model lists resource objects for the case's selectors. |
| D33 | `numbering-consistency` | not reached by a fixture | open against a verdict | A rule selection whose source's objects cannot all be listed: the capability judged the numbers of the objects it could list; the template binds the selection into the measured `numbering` (`@selection`), which refuses a selection it cannot list whole, so each selected object is open. | Accepted: fail closed where an object that may fill a gap or shift a prefix is missing, as D29; no public model lists resource objects for the case's selectors. |
| D34 | `wall-spacing` | not reached by a fixture | open against a verdict, and the selection's words | A `members` or `footprints` selector whose source's objects cannot all be listed: the capability paired the members it could list and left the coverage open saying `the storey's footprint objects are undecided: …`; the template binds both selections into the measured `wall_spacing`, which refuses a selection it cannot list whole, so each storey is open (``the objects `@members` selects cannot all be listed: …``). | Accepted: fail closed where a member or footprint may be missing, as D29; no public model lists resource objects for the case's selectors. |
| D35 | `parking-bay` | not reached by a fixture | open against a verdict | An `aisles` or `obstacles` selector, or the rule's own selection of bays, whose source's objects cannot all be listed: the capability searched the objects it could list; the template binds the selections into the measured `parking_bay`, which refuses a selection it cannot list whole, so each bay is open. | Accepted: fail closed where an aisle, obstacle or neighbour may be missing, as D29; no public model lists resource objects for the case's selectors. |
| D36 | `distance` | not reached by a fixture | open against a verdict | A subject, counterpart or container selection whose source's objects cannot all be listed: the capability judged the objects it could list; the template binds the selections into the measured `distance_items`, which refuses a selection it cannot list whole, so each subject is open. | Accepted: fail closed where a counterpart or container may be missing, as D29; no public model lists resource objects for the case's selectors. |
| D37 | `containment` | not reached by a fixture | open against a verdict | An inner or outer selection whose source's objects cannot all be listed: the capability judged the objects it could list; the template binds the selections into the measured `containment_items`, which refuses a selection it cannot list whole, so each inner element is open. Where one object is both an inner element and an outer one, or both an inner element and an undecided outer one, and nothing of it is a finding, its first open outcome follows the template's order (its own items, then the project's) rather than the capability's sorted one. | Accepted: fail closed where an element may be missing, as D29; the reason of an object's first open outcome may differ only where it is open for several reasons; no public model lists resource objects for the case's selectors. |
| D38 | `clash`, `clash-matrix` | not reached by a fixture | open against a verdict | A `counterparts` selection whose source's objects cannot all be listed: the capability paired the objects it could list; the template binds the selection into the measured pairs, which refuses a selection it cannot list whole, so every selected object is open (``the objects `@counterparts` selects cannot all be listed: …``). A matrix rule stating an empty `system_path` while `exclude_same_system` is off: the capability never read the path, the measured pairs refuse an empty text argument, and every selected object is open as an invalid declaration. A finding cites what its pair was measured from (the proximity measurement, a duplicate's compared values and a tolerance case's frames and extents, where read), where the capability cited a duplicate's values and a case's measurement only on the finding that needed them: only the exactness of such evidence could differ. | Accepted: an unlistable selection and an empty path both fail closed where the capability judged; no public model or fixture lists resource objects for the case's selectors or states an empty path, and every source evidence the fixtures and public models cite is as exact as the proximity measurement beside it. |
| D39 | `component-clearance` | not reached by a fixture | the scope of an open outcome | A rule with `front_axis` `against-wall` on a host without the plan-span service: the capability left the rule open as a whole; the template asks for its three services for the rule and the list leaves each component open, in the same words and for the same reason (`component-clearance with `front_axis` `against-wall` needs the plan-span service`). | Accepted: both fail closed for the missing service; a template states one set of services for the rule, and the plan-span service is needed only by that mode. Every host the CLI builds with geometry registers it. |
| D40 | `group-composition` | `derived_groups.rs` (`a_room_whose_flat_number_cannot_be_read_leaves_every_flat_undecided`) | the scope of an open outcome | A rule selecting derived groups of a source whose membership cannot be read: both leave the source open for the same reason; the capability went on to judge the groups and members it could list (each undecided), while the template, whose list of the project needs the rule's whole selection, leaves the rule open for the project (`the objects `@selection` selects cannot all be listed: …`). | Accepted: both fail closed, no group is found, and the source's outcome names the unreadable membership; a list judging the selection as a whole never reads a part of it. |

The comparison grew stricter with this chapter: it now counts findings
(D1), compares source- and project-scoped outcomes (D4 now shows the
project as well) and rule summaries (D18), and compares measured values
(D17). Every divergence recorded before kept its verdict-level form, the
wording of a silent side changing from "passed" to "reported nothing"
where the selection is unknown.
