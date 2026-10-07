# Template performance

A capability rebuilt as a [template](./templates.md) must not be much
slower than the code it replaces
([#279](https://github.com/axioval/engine/issues/279)). The budget, per
input, is at most **1.25 times the run time** and **1.5 times the peak
heap** of the replaced implementation, over the same rules and the same
model. A benchmark holds every rebuild to it: a report in CI, a gate
locally.

## How a template stays close

A template reads its values through the same parts an `expression` rule
reads, which do more work per value than a capability calling its services
directly. Four things keep that cost near the capability's.

**A flat plan, bound once per rule.** Binding folds the rule's parameters
into constants and fills every slot of the form's expressions
(`templates::bind`). It is a pure function of the template and the rule's
parameters, so each templated capability keeps the plans it bound
(`Plans`, at most 64, keyed by the rule's parameters): a host checking
model after model binds a rule once. A plain property read is filled in
place, without the expression's JSON form. When the rule runs, a value that
is one plain property read is read straight from the object's leaf
(`ObjectLeaves`), spending the same evaluation budget; only a composed
value goes through the evaluator, and neither records an explanation the
template never shows (`evaluate_untraced`). A measured name is parsed once
per process, not once per read.

**Measurements memoized for the run.** Every run installs an empty
`MeasuredMemo` beside its measured-value providers; a provider keeps there
what several values or rules read, keyed by everything it depends on (the
object and the call's parameters, never a rule's). Nothing measured in one
run answers another. `body-extent`'s provider keeps one entry per body: its
frame, measured once, and its extent along each axis read, measured once.
Its three values (`body_extent` and both `body_position` ends), along every
axis and in every rule, share them, so a run asks each service once per
body and axis where the replaced implementation asked once per rule.
`tests/body_extent.rs` (`a_run_measures_each_body_once_per_axis`) counts
the calls.

**Values read in batches.** A value that is one measured read is measured
for a chunk of selected objects at once (32), through the run's
`MeasuredValues`: the very measurement, value and evidence the run's
resolver answers for each object's request (`MeasuredRead`), without a
request and a resolution built per object. Each provider has a batch entry
point, `MeasuredProvider::measure_batch`; its default measures one object
at a time, and a provider overrides it to read its services and the call's
parameters once, or to ask a service about many objects together.
`body-extent`'s provider parses the axis and end once per batch. The same
test holds every value read directly to the value resolved one by one.

**Selections shared between rules.** A selector that reads no rule's
outcomes and evaluates no expression selects the same objects for every
rule of a run, so the runner selects it once per run
(`selection::select_shared`, kept in the `MeasuredMemo` by the selector as
written, as positions in its population): the rules' own selectors and the
selectors their measured values bind (`doors=@door_selector`), whichever
reads it first. An entity type is decided once per source and kind within a
selection. A provider keeping its own measurements keys them by an
`ArgumentsKey` (hashed once, compared argument by argument, never by
formatting the arguments), and the memo's tables hash with `foldhash`.

**No fixed cost per object or rule a capability did not pay**
([#292](https://github.com/axioval/engine/issues/292)). A rule's bound
plan keeps each measured name it reads for every run
(`measured_arguments::Planned`): parsed and bound once, a selection's
other arguments bound once and only the selection per run, a name naming
the anchor prepared once for each object's read to bind
(`PreparedRead::anchored`). Which checks and `unless` values apply,
constants as messages show them, and a selector parameter's shared key
are decided once per plan. An object's checks read on from one buffer
instead of a copy each; a refusal is cut in place, never formatted to be
stripped again; check values are read ahead with the form's. Entity types
combined in a shared selection (`anyOf` slab and roof) are decided once
per run for every selection of it. What only one rule reads (a provider's
rows, patterns or whole-rule lists) is kept for that rule
(`MeasuredMemo::of_rule`) and dropped once it is evaluated, never held for
the whole run.

**Aggregates over one pass of the relationship index.** Not done yet: no
template reads an aggregate, so nothing measures what it would cost. An
aggregate still walks its relationship path per object. The first rebuild
that templates a capability with an aggregate (#280–#287) adds the walk
over one pass of the index, and the benchmark below holds it to the
budget.

## The benchmark

`crates/apps/cli/benches/templates.rs` runs every pair in its `PAIRS`
list: a templated capability, the parity case whose rules of it are run,
and its replaced implementation (`axioval_rules::reference`, compiled
with the rules crate's `parity-reference` feature). For each input, a
generated fixture of 400 walls and 21 slabs (`AXIOVAL_BENCH_WALLS`) and
every pinned public model the case names, it:

1. imports and meshes the model once, as `axioval check --geometry` does
   (it includes the binary's IFC to Axiolid bridge);
2. compiles the case's rules of the capability and runs them by two
   runtimes over the same session: the default registry (the template),
   and the same registry with the reference in the template's place
   (`CapabilityRegistry::replace`);
3. holds the two reports to the parity contract (`Parity::contract()`),
   rule by rule, and records whether they agree;
4. runs both three times to warm up, then 21 times each
   (`AXIOVAL_BENCH_RUNS`), interleaved and alternating which goes first;
5. records each side's median, least and greatest run time and median peak
   heap: what the run allocated beyond what was live before it, counted by
   a global allocator (`peak_alloc`, a bench-only dependency); and every
   measured run of each side in the order it ran (`times_ns`,
   `peaks_bytes`), so run `i` of the template pairs with run `i` of the
   reference.

Run time is wall time; the paired runs of interleaved measurement absorb a
machine's drift, not its load, so a gate runs where nothing else builds.
With `--only FILE` (a JSON list of `[capability, input]`) the bench
measures only those inputs.

`scripts/bench.py` runs the bench and judges its records against
`scripts/bench_budget.json`:

- the run time over the reference's, per input whose reference takes at
  least `floor_ns` (100 µs), with a stated confidence (below); inputs
  below the floor are judged together,
  by the sum of their medians, since a run of a few microseconds is mostly
  timer noise, and may also exceed the references' summed medians by at
  most `small_slack_ns` (50 µs, a few rules' fixed cost) each;
- the median peak heap over the reference's, per input, or at most
  `slack_bytes` (64 KiB) above it: on a model of a handful of objects a
  template's fixed cost per rule, a few microseconds or kibibytes, is all
  either side measures, and no regression;
- the two sides agreeing under the parity contract;
- in a gate, a fixture and a public model measured, and every public model
  the case names fetched.

`scripts/test_bench.py` (in the `lint` gate) is its self-test: pass, fail
and undecided inputs, the interval, pooling and the re-measurement loop.

### Judging run time with a stated confidence

One median against the budget passed or failed an input near the line by
chance: the same build measured `space-validation` on one architecture
model at 1.22× and at 1.28× ([#293](https://github.com/axioval/engine/issues/293)).
A gate therefore judges an interval, not a point.

- **A round's measurement.** Each interleaved pair of runs gives one ratio,
  the template's time over the reference's; the two ran next to each
  other, so a change in the machine's speed falls on both. A *round* is one
  process of the bench, and its measurement is the median of its 21 paired
  ratios, which one slow run does not move.
- **Rounds, not runs, are independent.** A process's code and heap layout
  shifts every run in it alike. Measured on the inputs near the line,
  `door-swing` on the fixture read 1.215× to 1.291× in six processes, each
  with a 99% interval over its own runs about ±0.02 wide and several of
  them disjoint: an interval over the runs of one process is overconfident.
  The rounds' medians are the independent measurements.
- **The interval.** Over `J` rounds with logarithmic medians `m_j`, mean
  `m` and standard deviation `s`, Student's t interval
  `exp(m ± t((1 + c) / 2, J - 1) · s / √J)` covers the ratio a round
  measures with confidence `c` (`confidence` in
  `scripts/bench_budget.json`, 99%). It assumes the rounds' log medians
  are roughly normal (each is a median of 21 runs) and needs no
  resampling: the same rounds give the same interval. The table records the
  spread `s` per input, so a noisy input is visible.
- **The verdict, at planned looks.** An input passes when the interval's
  upper end is within its ceiling (the budget's, or a recorded
  exception's), and fails when its lower end exceeds it. Otherwise it is
  undecided. The interval is judged only after the round counts in `looks`
  (2, 3 and 20); between them an input stays undecided, whatever its
  interval. Judging after every round would let a template near its
  ceiling pass whenever one round's interval happened to clear it: in a
  trial gate judging every round, two inputs within 1% of the line passed
  after 12 and 16 rounds.
- **Further rounds.** The gate measures every input in a first round, then
  each input above the floor that is undecided in another round of its own
  (`--only`), until it is decided at a look or reaches the last. The second
  round covers every input above the floor; most are decided at the second
  or third look, and only the few near their ceiling are measured to 20
  rounds, each such round a fraction of a second per input.
- **The stated rule at the bound.** An input still undecided after the
  last look fails, reported as undecided with its interval: a gate passes
  only a template it shows to be within budget, as a straddling value
  never passes a check. Its run time is within the measurement's
  resolution of the ceiling; optimize it, or record an exception with its
  reason.

Each look is at 99% and there are three, so a template's verdict is wrong
(passed though over its ceiling, or failed though within it) with
probability at most 3%. The verdict repeats across gates for every input
whose ratio is not within the last look's half-width of its ceiling: with
a spread of 1 to 2% between rounds, as on the inputs near the line, about
±1% after 20 rounds. A template that close to its ceiling may be decided
either way, or not at all, from one gate to the next; one comfortably
within passes at the second or third look.

Inputs under the floor are measured in the first round only and judged by
the sum of their medians, as above. The records written back to `--out`
hold every pooled run and how many runs each round took (`rounds`), so
`bench.py judge FILE --gate` reproduces the gate's verdict. `report`
measures one round, so its inputs above the floor are listed undecided
with their ratio.

### Running it

```sh
python3 scripts/parity_models.py fetch   # once: the public models
python3 scripts/bench.py gate            # rebuild PRs: fails over budget
python3 scripts/bench.py report          # prints, never fails on a ratio
python3 scripts/bench.py judge FILE --gate   # judge saved records
```

`--out FILE` keeps the records (default `target/bench/templates.jsonl`).
On a machine shared with other builds, run the gate under the machine's
build lock, so nothing else skews it:

```sh
flock ~/.cache/axioval/gate.lock python3 scripts/bench.py gate
```

The gate is not part of `./scripts/check.sh`, whose sections must not
depend on a machine's load. CI runs the report in its own job
(`Template benchmark (report)`), which `check` does not need; it prints the
table into the job summary and keeps the records as the
`template-benchmark` artifact.

### Adding a rebuild

A rebuild adds its capability to `PAIRS`: its id, the public parity case
listing its rules (`recorded` there since the switch) and a function
registering its reference in the template's place. Its gate must pass
before the rebuild is done (step 8 of
[Rebuilding a capability as a template](./templates.md#rebuilding-a-capability-as-a-template)).

### Recorded exceptions

An input may hold a template to a ceiling of its own only as a recorded
exception in `scripts/bench_budget.json` (`exceptions`: capability, input,
`time` or `memory`, and the reason), so a regression past it still fails.
Two are recorded, both on the generated fixture, where services cost
almost nothing and a template's fixed cost per object (one read, its
evidence, the judge) shows alone:

- `triangle-count` (1.6×): the fixture's meshes are twelve-triangle boxes;
  on every public model it runs within 1.13×.
- `property-predicate` (1.45×): the fixture states none of the case's
  properties, so every wall is a finding worded through the template's
  messages; on every public model it runs within 1.03×.

Never record an exception for a public model, and never to let a rebuild
through: optimize the runner or the capability's provider instead.

## Latest measurements

Measured by the gate (`flock … python3 scripts/bench.py gate`), the
median of 21 interleaved runs; time and peak heap are the template's over
the reference's.

| input | objects | body-extent | property-predicate | triangle-count | plan-area |
| --- | ---: | --- | --- | --- | --- |
| generated fixture, 400 walls | 421 | 1.21× / 1.42× | 1.47× / 1.03× | 1.37× / 1.14× | 1.11× / 1.29× |
| building architecture (IFC4) | 38 | 1.12× / 1.12× | 1.04× / 1.04× | 1.12× / 1.03× | 0.69× / 0.98× |
| building structural (IFC4) | 33 | 0.97× / 1.06× | 1.04× / 1.07× | 1.12× / 1.04× | 1.03× / 1.04× |
| building HVAC (IFC4) | 18 | 0.88× / 1.03× | 1.00× / 1.03× | 1.07× / 1.02× | 1.14× / 1.02× |
| infra road (IFC4) | 116 | 1.21× / 1.48× | 1.04× / 1.06× | 1.11× / 1.05× | 1.02× / 1.04× |
| building architecture (IFC2x3) | 36 | 1.18× / 1.12× | 1.08× / 1.04× | 1.14× / 1.04× | 0.68× / 0.97× |
| building structural (IFC2x3) | 30 | 1.00× / 1.06× | 1.07× / 1.07× | 1.16× / 1.04× | 0.51× / 0.99× |
| building architecture (IFC4x3) | 37 | 1.14× / 1.12× | 1.05× / 1.04× | 1.10× / 1.03× | 0.69× / 0.98× |
| building structural (IFC4x3) | 33 | 0.96× / 1.06× | 1.05× / 1.07× | 1.13× / 1.04× | 0.98× / 1.04× |
| wall with opening and window | 9 | 1.11× / 1.13× | 1.13× / 1.03× | 1.12× / 1.14× | 1.08× / 0.89× |
| tessellated column | 3 | 1.03× / 1.07× | 0.98× / 1.07× | 1.21× / 1.16× | 1.12× / 1.06× |

The templates of #290, measured by the same gate:

| input | objects | level-spacing | object-count | related-count | unique-value |
| --- | ---: | --- | --- | --- | --- |
| generated fixture, 400 walls | 421 | 0.98× / 1.00× | 1.20× / 1.24× | 1.00× / 1.00× | 1.21× / 1.12× |
| building architecture (IFC4) | 38 | 1.09× / 1.12× | 1.10× / 1.04× | 1.18× / 1.21× | 1.00× / 1.01× |
| building structural (IFC4) | 33 | 1.05× / 1.13× | 1.11× / 1.04× | 1.21× / 1.14× | 1.00× / 1.01× |
| building HVAC (IFC4) | 18 | 1.07× / 1.13× | 1.15× / 1.04× | 1.24× / 1.19× | 0.86× / 1.00× |
| infra road (IFC4) | 116 | 0.96× / 1.12× | 1.04× / 1.01× | 1.06× / 1.00× | 0.92× / 1.00× |
| building architecture (IFC2x3) | 36 | 1.12× / 1.13× | 1.10× / 1.04× | 1.23× / 1.21× | 1.03× / 1.03× |
| building structural (IFC2x3) | 30 | 1.09× / 1.14× | 1.12× / 1.05× | 1.24× / 1.14× | 1.03× / 1.02× |
| building architecture (IFC4x3) | 37 | 1.13× / 1.13× | 1.09× / 1.04× | 1.19× / 1.21× | 0.99× / 1.03× |
| building structural (IFC4x3) | 33 | 1.15× / 1.12× | 1.10× / 1.04× | 1.20× / 1.14× | 1.01× / 1.01× |
| wall with opening and window | 9 | 1.08× / 1.12× | 1.23× / 1.08× | 1.24× / 1.14× | 0.93× / 1.05× |
| tessellated column | 3 | 0.83× / 1.03× | 1.35× / 1.02× | 1.07× / 1.06× | 0.97× / 1.03× |

They run the `storeys` case's `level-spacing` rules (three) and the
`counts` case's `object-count`, `related-count` and `unique-value` rules
(three, three and five). The tessellated column's `object-count` (1.34×)
runs under the floor and is judged with the other small inputs, which hold
together. `level-spacing` reads its members' populations only once an
anchor is selected and shares one per selector between its nested parts,
and its nested members' plain measured values are measured in batches like
any form's; before that it ran 1.92× on the fixture, which holds no
building, selecting storeys and spaces for nothing.

`body-extent` and `plan-area` run the `elements` and `storeys` cases' rules
of their capability (four and six), `property-predicate` and
`triangle-count` the `elements` case's (two and one). `plan-area` runs
faster than its reference where storeys sum their members' areas: its
provider keeps each object's area for the run (`AreaKey` in
`plan_area/measured.rs`), so a member measured by its own rule is not
measured again for its storey.

`shelf-capacity` runs the `shelving` case's two rules. Its values name the
rule's selectors (`doors=@door_selector`): they are bound once per rule and
read for a chunk of spaces together (`MeasuredValues::read_bound_batch`),
each memoized by space, name and bound arguments, and its provider keeps
the access index once per run for its path and selections and each space's
shelving once per run for its arguments, so `shelf_length` and
`shelf_clear_height` share one request (`a_run_asks_once_per_space`). On the generated
fixture (no spaces) and the public models whose spaces the service cannot measure it runs at 0.42
to 0.74× its reference, and at 1.18 to 1.19× on the three architecture models
(peak heap 0.79 to 1.19×). It ran 1.65 to 1.68× there before the runner
shared selections between rules ([#282](https://github.com/axioval/engine/issues/282)):
each of the two rules selected its spaces, doors and openings anew for two
spaces, which the reference does once and only for the elements reaching a
space, and the provider keyed each space's shelving by its arguments
formatted with `Debug`, selections included. Its provider now keys them by
`ArgumentsKey` and keeps its values itself (`memoizes`).

The same work took `body-extent` on the generated fixture from 1.25 to
1.21× and on the architecture models from 1.20 to 1.22× down to 1.12 to
1.18×: its provider reads the run's memo once per batch and adds an
extent in place, and a range verdict no longer copies the object's read.
Selecting an entity type once per source and kind speeds up the references
too, so some ratios rose while both sides ran faster (`level-spacing` on
the road model, 0.67 to 0.96×, and `ramp-geometry` on the fixture, 0.52
to 0.81×).

The generic judges of #287 run the `judges` case's rules of their
capability, measured by the same gate:

| input | objects | consistent-value | selector-conformance | relative-count | property-value | property-requirements | property-comparison |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| generated fixture, 400 walls | 421 | 1.13× / 1.15× | 1.03× / 0.99× | 1.13× / 1.08× | 0.99× / 1.00× | 1.00× / 1.00× | 0.97× / 1.00× |
| building architecture (IFC4) | 38 | 1.11× / 1.02× | 1.01× / 1.01× | 1.11× / 1.06× | 0.96× / 1.00× | 1.01× / 1.00× | 0.97× / 1.00× |
| building structural (IFC4) | 33 | 1.09× / 1.02× | 1.01× / 1.01× | 1.13× / 1.05× | 0.97× / 1.00× | 1.00× / 1.00× | 0.97× / 1.00× |
| building HVAC (IFC4) | 18 | 1.18× / 1.02× | 1.03× / 1.02× | 1.11× / 1.12× | 0.95× / 1.00× | 1.02× / 1.00× | 0.87× / 1.00× |
| infra road (IFC4) | 116 | 1.18× / 1.00× | 1.02× / 1.02× | 1.09× / 1.03× | 0.97× / 1.00× | 1.04× / 1.00× | 0.93× / 1.00× |
| building architecture (IFC2x3) | 36 | 1.10× / 1.02× | 1.02× / 1.01× | 1.12× / 1.06× | 0.98× / 1.00× | 0.99× / 1.00× | 0.95× / 1.00× |
| building structural (IFC2x3) | 30 | 1.10× / 1.02× | 1.00× / 1.00× | 1.12× / 1.05× | 0.98× / 1.00× | 1.01× / 1.00× | 0.97× / 1.00× |
| building architecture (IFC4x3) | 37 | 1.07× / 1.02× | 1.02× / 1.01× | 1.12× / 1.06× | 0.94× / 1.00× | 1.01× / 1.00× | 0.96× / 1.00× |
| building structural (IFC4x3) | 33 | 1.10× / 1.02× | 1.04× / 1.01× | 1.12× / 1.06× | 0.96× / 1.00× | 0.99× / 1.00× | 0.97× / 1.00× |
| wall with opening and window | 9 | 1.02× / 1.09× | 1.03× / 1.00× | 1.20× / 1.12× | 0.97× / 1.00× | 0.99× / 1.00× | 0.96× / 1.00× |
| tessellated column | 3 | 1.02× / 1.08× | 1.09× / 1.01× | 1.02× / 1.07× | 1.00× / 1.00× | 1.01× / 1.00× | 0.99× / 1.00× |

`consistent-value` runs six rules, `selector-conformance` and `relative-count` four each, `property-value` seven, `property-requirements` two, `property-comparison` six.

Before this work `body-extent`'s template ran 2.3 to 3.8 times as long as
its reference (the fixture 3.8×): every value went through a request, a
resolution and the evaluator's explanation, and the extent was measured
three times per rule. Its peak heap is mostly the memo: one body per
measured object, kept for the run.

`area-ratio` runs the `storeys` case's two rules, a share of spaces over
slabs and a window-to-wall ratio of facade areas: 1.00 to 1.15× its
reference's run time on every input (the generated fixture 1.01×, the
building architecture models 1.13 to 1.15×, the structural models 1.01 to
1.11×) and 1.00 to 1.20× its peak heap. A numeric aggregate stops listing
members at its first unreadable one
(`ExpressionContext::members_until_unreadable`), as the capability stopped
summing at its first unmeasurable member: before that, a storey whose
walls' facades cannot be measured read every wall, 1.94× on the
structural models. Members are counted only where a rule asks for a
finding on an anchor without them.

`plan-coverage` runs the `storeys` case's rule: 1.02 to 1.22× its
reference's run time and 1.04 to 1.20× its peak heap, the architecture
models the highest. Its value reads the search once per subject, its
"none" wording from the search's citation, not a second read.

`slab-contact` runs the `coverage` case's two rules, a face against
selected walls, columns and beams, and one leaving out the bottom storey:
0.99 to 1.25× its reference's run time per input (the tessellated column
and the wall model, under the floor, judged together) and 1.03 to 1.35×
its peak heap, the infra road the highest. It reached that through
generic changes: a bound measured value is prepared once per rule
(`PreparedRead`: parsed, bound and keyed once, each selection shared and
kept in memos by identity), a provider keeping its measurements for the
run is not memoized a second time (`MeasuredProvider::memoizes`), the
grading values are read ahead with the form's, and the undecided count is
read only to word an undecided shortfall. Before them it ran 2.2× on the
infra road and held 11.8× the heap on the generated fixture, every read
formatting its selection into a memo key.

`slab-stack-spacing` runs the `storeys` case's rule: 0.94 to 1.16× its
reference's run time and 1.00 to 1.27× its peak heap, the generated
fixture the highest. Its values measure the stacks once per run, keyed by
the rule's selection, which the run's own selection supplies
(`@selection`); a value read again by several checks keeps what was read
ahead, and a rule selecting nothing reads nothing ahead.

`recess-width` runs the `spans` case's two rules: 1.01 to 1.03× its
reference's run time and 1.01 to 1.15× its peak heap, the generated
fixture the highest. Its list measures each footprint's recesses once per
object, as the capability did, and reads the rule's rows once per call.

`light-well` runs the `spans` case's three rules: 0.97 to 1.19× its
reference's run time and 1.00 to 1.17× its peak heap. Its values and
lists read one memoized measurement of each well (its spaces, their
extents, their section) for the run, and the section is measured only once
the stack is: read ahead for a chunk of wells, a section measured before
the stack refused it ran 5 to 16× its reference on the architecture models,
whose storeys hold a body without a mesh.

`centre-line-distance` runs the `spans` case's three rules: 0.80 to 1.07×
its reference's run time and 1.00 to 1.10× its peak heap. Its list reads
the wall selection bound once per rule and measures each footprint's
sides once per object, as the capability did.

`component-visibility` runs the `proximity` case's two rules: 0.58 to
1.06× its reference's run time and 0.81 to 1.16× its peak heap. Its view
binds the target and blocker selections once per rule, where the
capability selected them per rule as well, and asks the line-of-sight
service exactly what the capability asked.
`coordinate-consistency` and `external-wall-validation` (#291) run the
`storeys` case's `coordinates` rule and the `coverage` case's
`external-walls` rule. On the public models neither has more than one
source or a derivable envelope, so both leave the rule open before
judging a scope: `coordinate-consistency` runs at 1.02 to 1.09× its
reference (peak heap 1.00 to 1.05×) on every input above the floor and
`external-wall-validation` at 1.00 to 1.24× (peak heap 1.00 to 1.10×), the
generated fixture's 1.24× the cost of selecting before reading the
declaration and its value once per rule; the small models, under the
floor, hold together. Each derivation of the envelope is asked of the
service once per run, however many values and rules read it.

`space-boundary-coverage` runs the `spaces` case's two rules: 0.91 to 1.08×
its reference's run time and 1.00 to 1.13× its peak heap, the architecture
models (the only ones with spaces) the highest. Its values read one
boundary-coverage request per space and plane tolerance for the run
(`BoundaryMeasures`), so the off-surface count, share, uncovered and
overlapping areas and the surface share it, and two rules of one tolerance
share it too.

`horizontal-guard` runs the `spaces` case's two rules: 1.06× its reference's
run time on the generated fixture and 1.12× on the infra road (peak heap
1.20 and 1.15×); on the architecture models, whose guard requests the
service refuses, its runs of 80 to 100 µs lie below the floor and are
judged with the other small inputs (1.23 to 1.29× each, within budget
together). Its values name the rule's own selection (`@selection`), so the
surfaces are measured in one request for the run, read once for the rule
(`once`) and then by each surface's edges.


The door, window and opening templates of
[#281](https://github.com/axioval/engine/issues/281) run the `openings`
case's rules of their capability (`empty-host` eleven, `opening-area`
seven, `door-swing` four, `corridor-end-openings` two), measured by the
same gate:

| input | objects | empty-host | opening-area | door-swing | corridor-end-openings |
| --- | ---: | --- | --- | --- | --- |
| generated fixture, 400 walls | 421 | 1.03× / 1.11× | 1.21× / 1.00× | 2.00× / 1.01× | 0.73× / 1.00× |
| building architecture (IFC4) | 38 | 1.06× / 1.03× | 0.86× / 1.09× | 1.26× / 1.07× | 1.11× / 1.09× |
| building structural (IFC4) | 33 | 1.08× / 1.03× | 0.86× / 1.10× | 1.24× / 1.04× | 0.57× / 1.00× |
| building HVAC (IFC4) | 18 | 0.40× / 0.91× | 0.54× / 0.94× | 0.93× / 1.00× | 0.60× / 1.00× |
| infra road (IFC4) | 116 | 0.96× / 1.05× | 0.47× / 0.89× | 0.97× / 1.00× | 0.47× / 1.00× |
| building architecture (IFC2x3) | 36 | 1.08× / 1.02× | 0.95× / 1.09× | 1.26× / 1.07× | 1.12× / 1.09× |
| building structural (IFC2x3) | 30 | 1.10× / 1.03× | 0.95× / 1.10× | 1.27× / 1.04× | 0.58× / 1.00× |
| building architecture (IFC4x3) | 37 | 1.06× / 1.02× | 0.87× / 1.09× | 1.24× / 1.07× | 1.12× / 1.09× |
| building structural (IFC4x3) | 33 | 1.07× / 1.03× | 0.87× / 1.10× | 1.24× / 1.04× | 0.56× / 1.00× |
| wall with opening and window | 9 | 1.24× / 0.94× | 0.85× / 1.10× | 1.58× / 1.11× | 0.69× / 0.99× |
| tessellated column | 3 | 0.69× / 1.00× | 0.81× / 1.03× | 1.02× / 1.00× | 0.82× / 1.04× |

`door-swing`'s public models run under the floor and hold together. On
the generated fixture its walls rule selects 400 walls, none a door: the
reference refuses each at its leaves, while the template binds its list,
measures it (refused at the same leaves) and words the refusal through the
item list, a fixed cost per object the gate does not yet admit.
`opening_area` and `middle_face_area` opt out of the run's memo: kept
for every host and rule of the eleven `empty-host` rules, the values held
3.4× the reference's peak heap on the fixture, while placing a host's
openings again costs less than that; an object's leaves keep a value they
measured, so a truth composing it and the value worded share one
placement.
`exit-separation` runs the `spans` case's four exit rules: 0.57 to 1.18×
its reference's run time and 0.99 to 1.34× its peak heap. Its list binds
the exit selection once per rule and reads its universe once per
selection (`MeasuredMemo`), reaching each space's exits by identity
(`Traversal::related_ids`), and measures each space's diagonal, flag and
pairs once, as the capability did.

`name-sequence` runs the `numbering` case's three name rules: 0.83 to
1.23× its reference's run time and 1.00 to 1.34× its peak heap. Its list
binds the member selection once per rule, where the capability selected
the members again for each anchor, and reads the declaration from the
bound list rather than re-reading the rule; what remains is the
template's fixed cost per anchor, on models of few anchors and members.

`numbering-consistency` runs the `numbering` case's four number rules:
0.67 to 1.21× its reference's run time and 0.93 to 1.07× its peak heap.
Its list reads the whole selection's numbers and scopes once per rule,
compiling the pattern once, and hands each object its own items.

`wall-spacing` runs the `spans` case's two rules: 0.43 to 1.35× its
reference's run time and 0.90 to 1.34× its peak heap, the small models
under the floor together within 1.22×. Its list binds the member and
footprint selections once per rule, lists each selection's objects and
the project's once per run (`support::everything`, which a traversal walks
through), and measures each storey's pairs and footprints once, as the
capability did.

`parking-bay` runs the `spans` case's three rules: 0.32 to 1.06× its
reference's run time and 0.62 to 1.44× its peak heap. Its list runs the
plan broad phase once per rule and kind of neighbour, keeping only the
extents a bay reads (`Nearby::kept`), as the capability searched once per
rule, and measures each bay's rectangle, extent and obstacles once.

`distance` runs the `proximity` case's four rules: 1.02 to 1.17× its
reference's run time on every input above the floor and 0.94 to 1.36× its
peak heap; on the one model under the floor (three objects) the template's
fixed cost per rule (binding its two lists and the selections) is 1.82×.
Its lists read one rule at a time (`measured_kinds::latest`), each rule's
pairs prepared once and dropped before the next.

`containment` runs the case's two rules: 1.00 to 1.28× its reference's run
time on the public models above the floor and 1.03 to 1.38× its peak
heap; on the generated walls it is 1.38× and 1.65×, its count items one
per wall, and on the two models under the floor 1.40×. Both are over the
budget there and are the open part of #283.

`clash` and `clash-matrix` run the `clashes` case's rules (four and two):
1.00 to 1.11× their reference's run time on every input above the floor
and 1.03 to 1.29× its peak heap, the generated walls and the infra road
the highest. The list measures each pair once, as the capability did; it
states only what a pair states (a text without words, a false truth, a
switched-on class and a `null` are left out, and `clash`'s tolerances are
the rule's parameters rather than fields of every pair), lists each
object's open outcome once, and the judge lets each pair go once judged:
before that, the infra road held 2.0× the reference's peak heap. On the
one model under the floor (three objects, no pair) the template runs
1.72× (`clash`) and 1.29× (`clash-matrix`): about 9 µs per rule of the
runner's fixed cost (selecting, binding the list's two selections and
references, finding its provider), which no pair outweighs there.

The runner work of [#292](https://github.com/axioval/engine/issues/292),
measured by the same gate on the inputs it targeted (time / peak heap,
before on the integration head, after with it):

| capability | input | before | after |
| --- | --- | --- | --- |
| space-validation | building architecture (IFC4) | 1.75× / 1.73× | 1.23× / 1.37× |
| space-validation | building architecture (IFC2x3) | 1.75× / 1.73× | 1.22× / 1.37× |
| space-validation | building architecture (IFC4x3) | 1.68× / 1.73× | 1.28× / 1.37× |
| door-swing | generated fixture, 400 walls | 1.45× / 1.01× | 1.27× / 1.02× |
| property-predicate | generated fixture, 400 walls | 1.47× / 1.03× | 1.23× / 1.03× |
| external-wall-validation | generated fixture, 400 walls | 1.26× / 1.00× | 1.13× / 1.00× |
| opening-spaces | building structural (IFC2x3) | 1.27× / 1.20× | 1.15× / 1.19× |
| keyed-limit | wall with opening and window | 1.61× / 1.89× | 1.29× / 1.04× |
| opening-zone | wall with opening and window | 1.53× / 1.53× | 1.37× / 1.22× |
| opening-zone | building structural (IFC2x3) | 0.85× / 1.72× | 0.76× / 1.73× |
| containment | generated fixture, 400 walls | 1.38× / 1.65× | 1.17× / 1.65× |

A member list one check alone reads is handed to it and each item dropped
once judged (`ObjectLeaves::members_once`), a list naming a selection is
bound once per rule and shared uncopied, and a shared selection's key is
written once per selector. With them, measured by the same gate:

| capability | input | time / heap |
| --- | --- | --- |
| containment | generated fixture, 400 walls | 1.15× / 1.27× (was 1.17× / 1.65×) |
| keyed-limit | wall with opening and window | 1.24× / 1.04× |
| space-validation | building architecture (IFC4, IFC2x3, IFC4x3) | 1.22 to 1.27× / 1.35× |
| door-swing | generated fixture, 400 walls | 1.26× / 1.02× |
| opening-zone | wall with opening and window | 1.35× / 1.20× |
| opening-zone | building structural (IFC2x3) | 0.76× / 1.69× |

Still over the budget: `door-swing` on the fixture, `keyed-limit` on the
wall model and `space-validation` on the architecture models, by up to 3%
of run time (judged with a stated confidence, below); `opening-zone` on the wall model,
and its heap on the structural model, whose provider measures a whole
rule's checks (about 2 KiB an item) before its openings read them, its
judge holding state that cannot be kept between reads; and the small
models under the floor (`distance`, `containment`, `clash`,
`clash-matrix` on the three-object column), whose rules select almost
nothing and leave a few microseconds of the runner's cost per rule:
binding the rule's selections into `MeasuredSelection`s and its lists,
and a provider's reading of the rule.

### Judged with a stated confidence

Three consecutive gates on one build under the build lock
([#293](https://github.com/axioval/engine/issues/293)), each 14 to 16
minutes (two full rounds of about six minutes, then 20 rounds of the few
inputs near their ceiling), gave every input the same outcome: the same
eight failed in each, every other input passed. Ratio and 99% interval
after 20 rounds, per gate:

| capability | input | gate 1 | gate 2 | gate 3 | verdict |
| --- | --- | --- | --- | --- | --- |
| door-swing | generated fixture, 400 walls | 1.266 (1.253 to 1.279) | 1.270 (1.262 to 1.279) | 1.265 (1.252 to 1.277) | over |
| keyed-limit | wall with opening and window | 1.250 (1.242 to 1.258) | 1.251 (1.244 to 1.257) | 1.253 (1.245 to 1.260) | undecided |
| space-validation | building architecture (IFC4) | 1.263 (1.250 to 1.277) | 1.262 (1.252 to 1.271) | 1.268 (1.258 to 1.277) | over |
| space-validation | building architecture (IFC2x3) | 1.246 (1.234 to 1.258) | 1.250 (1.241 to 1.259) | 1.247 (1.241 to 1.252) | undecided |
| space-validation | building architecture (IFC4x3) | 1.277 (1.266 to 1.288) | 1.276 (1.264 to 1.288) | 1.278 (1.270 to 1.286) | over |
| table-allocation | wall with opening and window | 1.254 (1.249 to 1.259) | 1.256 (1.250 to 1.262) | 1.248 (1.241 to 1.255) | undecided, once over |
| table-allocation | column, tessellated | 1.300 (1.252 to 1.350), 3 rounds | 1.285 (1.280 to 1.290) | 1.280 (1.272 to 1.288), 2 rounds | over |
| opening-zone | wall with opening and window | 1.381 (1.371 to 1.390) | 1.379 (1.324 to 1.436), 3 rounds | 1.369 (1.327 to 1.413), 3 rounds | over |

The inputs a single median had passed or failed by chance are real
failures, or sit on the line: `door-swing` on the fixture and
`space-validation` on the IFC4 and IFC4x3 architecture models are over
the budget by 1 to 3%, beyond the measurement's resolution, and
`keyed-limit` on the wall model and `space-validation` on the IFC2x3
model are at it. The runner owes them its fixed cost, as above. Inputs
that pass near the line (`body-extent` on the infra road, 1.23×;
`table-allocation` on most models, 1.20 to 1.23×; `opening-area` on the
fixture, 1.21 to 1.23×) passed in every gate.

A gate's rounds share its machine's state, so two gates differ by a little
more than one gate's interval: `door-swing` read 1.23× pooled over twelve
rounds of one earlier gate and 1.27× in these. Six inputs whose reference
takes about the floor's 100 µs fell above it in one gate and below it in
another; above, they passed alone, below, with their small inputs.
