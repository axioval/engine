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
model after model binds a rule once. A rule whose declaration binding
refuses is refused the same way every time, so its refusal is kept alike
and never bound again ([#296](https://github.com/axioval/engine/issues/296)).
A plain property read is filled in place, without the expression's JSON form. When the rule runs, a value that
is one plain property read is read straight from the object's leaf
(`ObjectLeaves`), spending the same evaluation budget; only a composed
value goes through the evaluator, and neither records an explanation the
template never shows (`evaluate_untraced`). A measured name is parsed once
per process, not once per read.

**Measurements memoized for the run.** Every run installs an empty
`MeasuredMemo` beside its measured-value providers; a provider keeps there
what several values or rules read, keyed by everything it depends on (the
object and the call's parameters, never a rule's). Nothing measured in one
run answers another. The memo's shared table is a view of the run's
derivation store ([ADR 0006](./adr-0006.md),
[#437](https://github.com/axioval/engine/issues/437)): each key and value
type is a kind of the store, measured at most once at a time. A thread
asking for a key another thread is measuring waits for that measurement
and reads its value, instead of measuring it a second time and keeping the
first, so a rule evaluated on several threads holds one copy of each value
while it is measured. A hit costs what it cost before: one lock, a table
found by type and a key hashed with `foldhash`, the value cloned under the
lock. The store is the one the run enters (#451), so the memo's tables
are bounded by `--keep-derivations` and the memory budget and listed by
`--stats` with every other kind. `body-extent`'s provider keeps one entry per body: its
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
per run for every selection of it. An outcome about an object is placed
on it as it stands, its id copied once; only a scope's stand-in is placed
anew (#296). What only one rule reads (a provider's rows, patterns or
whole-rule lists) is kept for that rule
(`MeasuredMemo::of_rule`) and dropped once it is evaluated, never held for
the whole run. A read finds its value's provider by one lookup in an index
the registry builds as providers are registered, not by asking each
provider for its names, and a provider keys what it keeps for the run by
its arguments' values and selections' identities, never by formatting them
([#446](https://github.com/axioval/engine/issues/446)).

**Members supplied, not walked.** No template walks a relationship path
per object: the runner supplies an anchor's members, and a scope's objects,
to the evaluator in place of an aggregate's source
(`ObjectLeaves::supplying`), and a search's or a provider's member list is
measured once and memoized. An `expression` rule's aggregate still walks
its path per object; a walk over one pass of the relationship index is left
until a rule needs it, and the benchmark would hold a template reading it
to the budget.

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

### Where the gate runs

A rebuild's gate runs where it was rebuilt, and every release runs it again
on the commit it publishes
([#446](https://github.com/axioval/engine/issues/446)):

```sh
./scripts/release-gate.sh    # on the pushed release commit
```

`scripts/release-gate.sh` builds the bench, runs `bench.py gate` on the
committed HEAD with the machine's build lock taken exclusively
(`AXIOVAL_GATE_LOCK`, default `~/.cache/axioval/gate.lock`), and records
the verdict on GitHub as the commit status `template-gate`, `success` or
`failure`. The `Release` workflow publishes only a commit whose latest
`template-gate` status is `success`; `verify-only` does not ask for it.
`--no-status` runs the gate without recording it.

Before #446 nothing ran the gate once a rebuild had landed: CI only
reports, and `./scripts/check.sh` never runs it. Two later changes moved
three inputs over the budget unseen: one made `slab-contact`'s reference
faster, the other made `effective-coverage`'s slower, lifting it above the
floor, where its template's cost, hidden among the small inputs until then,
was judged alone (see
[A faster reference, a reference over the floor](#a-faster-reference-a-reference-over-the-floor)).

The gate is not a CI job, blocking or not, because a shared runner cannot
judge a ratio to its resolution. The interleaved, paired runs absorb a
machine's drift within a process, not which machine it is: a runner's
processor and neighbours move every round alike, so more rounds only
narrow the interval around that runner's own ratio. The
`template-benchmark` records of CI runs on main show it: on two runs whose
code differed only in version numbers, `effective-coverage` on the IFC4x3
architecture model read 1.44× and 1.55×, and its reference took 84 µs on
one runner and 120 µs on the other, below the 100 µs floor and above it. A
blocking job would pass or fail the same commit by the runner it drew.

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

Eight more, recorded as known misses on 2026-10-07 (`door-swing` on the
fixture, `keyed-limit` on the wall model, `space-validation` on the three
architecture models and `table-allocation` on the wall, column and HVAC
models), were removed in
[#296](https://github.com/axioval/engine/issues/296): each now passes the
budget itself (see [Brought back under the budget](#brought-back-under-the-budget)).

Record an exception for a public model only as such an accepted known
miss: judged with a stated confidence, never on one gate's median, and
with an issue that removes it. Optimizing the runner or the capability's
provider comes first.

## Measurements

Each table below was measured by the gate (`flock … python3
scripts/bench.py gate`) when its rebuild landed, as the median of 21
interleaved runs; time and peak heap are the template's over the
reference's. Later runner work (#292, #294) moved several of them, and
the gate has since judged run time with a stated confidence: the last full
judgement is under [Judged with a stated confidence](#judged-with-a-stated-confidence),
and the inputs over budget are the recorded exceptions above. Run the gate
for current numbers.

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
and 1.04 to 1.36× its peak heap, `clash` on the architecture (IFC2x3)
model the highest. The list measures each pair once, as the capability did; it
states only what a pair states (a text without words, a false truth, a
switched-on class and a `null` are left out, and `clash`'s tolerances are
the rule's parameters rather than fields of every pair), lists each
object's open outcome once, and the judge lets each pair go once judged:
before that, the infra road held 2.0× the reference's peak heap. A pair
leaves out its group's types and storeys too where they state no words
([#318](https://github.com/axioval/engine/issues/318)): once the public
samples' meshes were registered as the closed solids they are (#308), the
infra road's pairs were measured rather than refused, and those two empty
texts took each pair's fields past one node of their map, 1.79× the
reference's peak heap (702.6 KiB against 392.2 KiB); without them it is
1.25× (491.2 KiB). On the
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

Still over the budget, recorded as known misses (#296): `door-swing` on
the fixture, `keyed-limit` on the wall model and `space-validation` on the
architecture models, by up to 3% of run time (judged with a stated
confidence, below). The small models under the floor (`distance`,
`containment`, `clash`, `clash-matrix` on the three-object column), whose
rules select almost nothing and leave a few microseconds of the runner's
cost per rule, pass within the small inputs' allowance:
binding the rule's selections into `MeasuredSelection`s and its lists,
and a provider's reading of the rule.

### Brought back under the budget

The eight known misses of #296, judged by the gate with a 99% interval
(before on the head they were recorded against, after with the change):

| capability | input | before | after |
| --- | --- | --- | --- |
| door-swing | generated fixture, 400 walls | 1.277 (1.262 to 1.293) | 1.229 (1.220 to 1.239) |
| keyed-limit | wall with opening and window | 1.245 (1.238 to 1.253) | 1.200 (1.191 to 1.209) |
| space-validation | building architecture (IFC4) | 1.021 (0.989 to 1.055) | 1.026 (0.917 to 1.147) |
| space-validation | building architecture (IFC2x3) | 1.001 (0.893 to 1.122) | 1.007 (0.953 to 1.064) |
| space-validation | building architecture (IFC4x3) | 1.017 (0.926 to 1.117) | 1.013 (0.899 to 1.142) |
| table-allocation | wall with opening and window | 1.243 (1.235 to 1.250) | 1.216 (1.211 to 1.221) |
| table-allocation | column, tessellated | 1.271 (1.266 to 1.275) | 1.237 (1.230 to 1.243) |
| table-allocation | building HVAC (IFC4) | 1.226 (1.215 to 1.237) | 1.176 (1.114 to 1.241) |

`space-validation` was back within the budget already, through the
bracketed evidence of #305. Of the rest, profiles of the template against
its reference showed two costs the reference never paid. A rule whose
declaration refuses (the parity cases hold such rules for both) was bound
again in every run, compiling its rows' patterns to refuse them once more:
about 3% of `table-allocation`'s and 4% of `keyed-limit`'s run time. Its refusal is now kept with the plans. And
every outcome placed on an object copied its id twice, once into a scope
only to be thrown away: on the fixture, where every wall of `door-swing`'s
rules is left open for want of a hinged leaf, 3% of the run time. What is
left of the gap is wording and judging items through the template's
messages; on the small models most of either side's time was compiling the
rows' wildcard patterns, which both did once per rule and run (see
[Patterns compiled once](#patterns-compiled-once)).

### Patterns compiled once

`table-allocation` on the tessellated column (three objects, no space)
stayed at the line after #296: 1.25× (99% interval 1.24 to 1.26), passing
or failing by chance
([#355](https://github.com/axioval/engine/issues/355)). A profile of the
pair put about 70% of both sides' time in compiling the rows' wildcard
patterns into regular expressions, done again in every run although a
rule's patterns never change. The allocation now compiles each pattern once
per process and keeps it (`TextPattern::kept`, at most 256 per case, all
dropped when one more would not fit; a pattern that does not compile is
refused afresh every time and never kept), as a template keeps its bound
plans. The reference still compiles them every run. Outcomes, messages and
the descriptors are unchanged. Three exclusive gates gave the pair
0.35× (0.31 to 0.40), 0.35× (0.16 to 0.75) and 0.36× (0.16 to 0.82), and
every other `table-allocation` input 0.41 to 0.72×.

### A faster reference, a reference over the floor

Three inputs passed when #296 landed and failed on main twelve days later
([#446](https://github.com/axioval/engine/issues/446)), none of their
templates changed. Bisected on the three inputs alone (`--only`, three
rounds per commit under the build lock):

- `slab-contact` on the infra road went from 1.15× to 1.25× with the
  shared relationship candidate universes of 97f0099f
  ([#429](https://github.com/axioval/engine/issues/429)): its reference,
  which walks each slab to its storey, went from about 670 µs to 430 µs,
  while the template's own cost per object stayed. Asking each closed
  chain once (b2fc5ff4, [#428](https://github.com/axioval/engine/issues/428))
  and the derivation store (#437) took it to 1.30×.
- `effective-coverage` on the IFC4 and IFC4x3 architecture models ran at
  1.40 to 1.50× all along, but its reference took 82 to 85 µs, under the
  100 µs floor, so it was judged with the small inputs, which hold
  together. Bounding plan overlays by a budget (a0928e14,
  [#419](https://github.com/axioval/engine/issues/419)) made both sides
  measure more, and the reference's 104 µs put the input above the floor,
  judged alone.

What the reference never paid was formatting: `effective-coverage`'s
provider keyed each element's measurement by its arguments written out
with `Debug` (eleven of them, at each of the four values every element
reads) and copied its sources' selection at each read; `slab-contact`'s
keyed its storey traversal by five arguments written out, for every
object. Both now key by the arguments' values and the selections'
identities (`effective-coverage` by one small key per argument, worded
nothing), borrow the bound selection, and word the element's locator
once. Every read found its provider by asking each of the registry's
providers for its names, twice; it now looks the name up in an index the
registry builds as providers are registered. A prepared read hashes its key
with `foldhash` rather than SipHash, and a contact measurement keeps the
service's evidence rather than copying it. Outcomes and reports are
unchanged; the parity harness holds them byte for byte.

Judged by the gate's rule over 20 rounds under the build lock (before, on
122f2b31, four alternating rounds beside the fix):

| capability | input | before | after |
| --- | --- | --- | --- |
| slab-contact | infra road (IFC4) | 1.306 (1.279 to 1.333) | 1.199 (1.192 to 1.206) |
| effective-coverage | building architecture (IFC4) | 1.387 (1.359 to 1.417) | 1.216 (1.209 to 1.224) |
| effective-coverage | building architecture (IFC4x3) | 1.389 (1.374 to 1.405) | 1.215 (1.206 to 1.223) |

The gate now runs on every release
([Where the gate runs](#where-the-gate-runs)).

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

`opening-zone`'s provider measured a whole rule's checks (about 2 KiB an
item) before its openings read them
([#294](https://github.com/axioval/engine/issues/294)). It now keeps only
what a rule's openings share (the placements, the hosts' bodies, the
table's selections, the supports read) and measures each opening's checks
when its list is read; each host's body and each opening's solid are read
once per run, shared by the rules, where the capability read them once per
rule. Measured by the same gate, the case's ten rules together (time /
peak heap):

| input | before | after |
| --- | --- | --- |
| wall with opening and window | 1.38× / 1.20× | 0.94× / 1.31× |
| building structural (IFC2x3) | 0.75× / 1.69× | 0.78× / 1.31× |
| building structural (IFC4, IFC4x3) | 0.51× / 1.17× | 0.52× / 1.17× |
| generated fixture, 400 walls | 0.21× / 0.90× | 0.22× / 0.90× |
| every other public model | 0.22 to 0.46× / 0.98 to 1.03× | 0.21 to 0.46× / 0.98 to 1.03× |

The heap on the wall model rises by the bodies kept for the run. One rule
run alone reads them all itself and stays 1.24 to 1.53× its reference's
time there: the runner's fixed cost per rule on a model of nine objects.

### Scenes and body indexes built once per check

Free-floor searches, largest-disc estimates and circulation maps read a
space's placement scene, and every pair a clash, clearance or distance
rule measures reads both bodies' triangle hierarchies. Both are
derivations of the check's store since
[#408](https://github.com/axioval/engine/issues/408)
([ADR 0006](./adr-0006.md)): a scene is built once for every rule and
thread reading the same space, obstacles, swept doors and band, and a
body is indexed once for every pair reading it. Before, the free-space
service kept the last two scenes of each searching thread, and each pair
indexed both its bodies afresh. The store's statistics, one line a kind,
as `check --timings` printed them when these were measured (`check
--stats` prints them now, with each kind's version, build time, peak
bytes and evictions, see [The reuse report](./adr-0006.md#the-reuse-report)):

```text
derivation: placement scene: 157 built, 989 read again (0 waited), 0 released, 5.7 MiB held
derivation: indexed body: 588 built, 30227 read again (0 waited), 0 released, 7.9 MiB held
```

Release builds, wall time and peak resident memory from `getrusage`
under a 20 GB cgroup limit, one run at a time on a 20-core machine, the
default memory budget. The rule sets: two circles (1.5 and 1.2 m) and two
rectangles at any orientation (1.8 by 1.5 and 1.2 by 0.9 m), all 2 m
high, among the spaces' furniture, or among every physical element; and
the six clash and clash-matrix rules of the parity case `clashes`.

| Model, rule set | Threads | Scenes or bodies built / read | Wall before | Wall after | Peak before | Peak after |
|---|---:|---|---:|---:|---:|---:|
| m13 (157 spaces), furniture | 1 | 157 / 915 | 5.3 s | 5.3 s | 1109 MiB | 1112 MiB |
| m13, furniture | 4 | 157 / 915 | 4.3 s | 4.3 s | 1157 MiB | 1161 MiB |
| m13, every physical element | 4 | 157 / 1146 | 1712 s | 1621 s | 3517 MiB | 3515 MiB |
| m07 (17 spaces), furniture | 1 | 17 / 127 | 4.3 s | 4.0 s | 633 MiB | 633 MiB |
| m07, every physical element | 1 | 17 / 144 | 94.1 s | 93.1 s | 799 MiB | 799 MiB |
| m07, every physical element | 4 | 17 / 144 | 69.8 s | 69.6 s | 1917 MiB | 1904 MiB |
| m07, clashes | 1 | 588 / 30,815 | 6.5 s | 6.0 s | 377 MiB | 393 MiB |
| public architecture (IFC4), clashes | 1 | 8 / 120 | 0.3 s | 0.3 s | 45 MiB | 46 MiB |

Each space's scene is now built once: on m13 among every physical
element, the three rules after the first spend 2.4 → 1.3 s
(`circle-150`) and 15.3 → 14.1 s (`rectangle-120x90`), and the check
5% less in all. Most of a scene is its obstacles' band footprints, which
the service already kept for every space of a floor, and most of these
checks is the rectangle search itself (1665 of the 1712 s on m13), which
reuse does not touch. On m07's clashes the 30,227 indexes no longer
built save 7% of the rules' time (5.4 → 5.1 s); the rest is the pairs' own
measurement. The kept scenes and indexes hold at most 8 MiB here, and
peaks are the same within a run's noise. Every report is the same byte
for byte as before.


### Derivations released after their last reader

Since [#440](https://github.com/axioval/engine/issues/440) the store
releases a kind's values once no rule still to run may ask a service it
is read through, and evicts, read least recently first, whenever kept
values and running steps would pass the memory budget (see
[Memory budget](./memory.md#for-a-derivation-store)). Every scene and
index of the checks below is released after its last reader, and nothing
is evicted under the default budget: at most 0.5 MiB of scenes and
7.9 MiB of indexes are held at once.

The same release build, measured as above, three ways: with the store
("store"), with a store keeping nothing past the value just built
(`--keep-derivations 0`, the services' own caches kept: "keep 0"), and
without reuse (`--no-reuse`, which also turns off the free-space
service's footprint and column caches: "none"). Two runs each where
two peaks are given.

| Model, rule set | Threads | Wall store / none | Peak store | Peak keep 0 | Peak none |
|---|---:|---:|---:|---:|---:|
| m13, furniture | 1 | 5.5 / 5.3 s | 1115, 1110 MiB | 1110 MiB | 1109, 1110 MiB |
| m13, furniture | 4 | 4.3 / 4.3 s | 1155, 1161 MiB | 1157 MiB | 1153, 1168 MiB |
| m07, furniture | 1 | 4.3 / 5.8 s | 635, 630 MiB | 631 MiB | 609, 612 MiB |
| m07, furniture | 4 | 2.0 / 2.5 s | 1294, 1274 MiB | 1307 MiB | 1212, 1145 MiB |
| m07, every physical element | 1 | 92.8 / 173.9 s | 796 MiB | 819 MiB | 829 MiB |
| m07, every physical element | 4 | 69.3 / 103.1 s | 1902 MiB | 1904 MiB | 1910 MiB |
| m07, clashes | 1 | 6.0 / 6.3 s | 386, 389 MiB | 378 MiB | 380, 378 MiB |
| m07, clashes | 4 | 6.0 / 6.3 s | 392, 393 MiB | 378 MiB | 380, 380 MiB |

The store's peak is the peak without it, within a run's noise, but in
two places. m07's clashes peak 8 to 15 MiB (2 to 4%) higher: the body
indexes every pair reads, kept until the last clash rule, which is the
point of keeping them. m07's furniture set on four threads peaks 60 to
150 MiB higher than with `--no-reuse`, and as high with a store keeping
nothing: that is the free-space service's footprints of each storey's
obstacles, which `--no-reuse` also drops, not the store. A store keeping
one byte (`--keep-derivations 1`, the determinism gate's variant) builds
m07's 17 physical scenes 97 times instead of 17, in the same 69.8 s on
four threads. Every report is the same byte for byte in every variant
and as the engine of #408.

### Adapter caches on the store

Since [#443](https://github.com/axioval/engine/issues/443) no adapter keeps
a cache of its own for its service's whole life: the free-space service's
obstacle footprints and columns, the space service's plan areas, the
proximity service's shells and composed volumes, the derived
relationships' space bodies, edges and pairs, metric routing's prepared
floors, doors' glazing items and the relationship service's universes
read by entity are derivation kinds, held against the memory budget and
released after their last reader. What stays with a service is part of
the source, listed in
[ADR 0006](./adr-0006.md#adapter-caches-on-the-store).

Measured as above, one release build per side: the engine before
("before"), after ("after"), and after with `--no-reuse` ("none"). The
public parity models are every case over every model it names, as the
parity harness runs them (162 checks, the least wall time of three runs
each).

| Model, rule set | Threads | Wall before / after / none | Peak before / after / none |
|---|---:|---:|---:|
| m13 (157 spaces), furniture | 1 | 5.3 / 5.3 / 5.3 s | 1109 / 1115 / 1111 MiB |
| m13, furniture | 4 | 4.0 / 4.3 / 4.3 s | 1158 / 1147 / 1153 MiB |
| m07 (17 spaces), furniture | 1 | 4.0 / 4.3 / 5.8 s | 634 / 634 / 636 MiB |
| m07, furniture | 4 | 2.0 / 2.0 / 2.5 s | 1313 / 1310 / 1143 MiB |
| m07, every physical element | 4 | 70.6 / 70.1 / 107.4 s | 1904 / 1506 / 1966 MiB |
| m07, clashes | 1 | 6.0 / 6.0 / 19.3 s | 396 / 391 / 375 MiB |
| m07, clashes | 4 | 6.0 / 6.0 / 19.3 s | 386 / 389 / 380 MiB |
| public parity models, 162 checks | 4 | 4.97 / 4.71 / 4.98 s | 111 / 104 / 105 MiB |

Run times are the same within a run's noise; the public cases spend 9%
less CPU (6.06 → 5.50 s), most of it in `openings` (0.70 → 0.40 s), whose
derived relationships no longer read every declared space's body again
for each request. m07 among every physical element builds 1398 band
footprints for 7547 reads, which the free-space service kept before as
well: at most 104 KiB of them are held, and the peak is the largest
step's (731 MiB admitted at once), which varies between runs (1432 to
1594 MiB after). The furniture set on four threads still peaks above
`--no-reuse`, by what four threads' scenes and footprints hold while they
are kept for the next rule, now counted against the budget.

One move had to keep a bound of its own: the relationship service's ring
of four universes read by entity found a universe only by identity, and
a rule relating each anchor to a universe built for it (the clash
matrices) found none again. Kept in the store by identity, m07's clashes
read 272 such universes once each and held 28.7 MiB of them to the last
relationship rule (440 MiB peak). Keyed by their objects
(`CandidateUniverse` hashes them once when built) they are one universe
read 272 times, 103 KiB held, and the kind keeps at most 64 MiB. Every
report is the same byte for byte before, after and with `--no-reuse`.

### A bounded geometry reach for every geometric capability

Since [#448](https://github.com/axioval/engine/issues/448) clash,
clash-matrix, containment, distance and every other built-in geometric
capability state the selectors whose bodies they measure and the services
they ask (`RuleCapability::geometry_reach`), and those that may ask
anything record why (`axioval_rules::reach`). Two things follow: `check
--geometry` meshes only the bodies some rule selects (#410), and the run
releases what a service built after the last rule that may ask it (#440).
Before, a clash rule may have asked anything, so every body was meshed and
every scene waited for the last clash rule.

Measured on one release build per side, the free-floor circles and
rectangles of the furniture set and then the public parity clashes
("floors, clashes"), and the clashes alone; one run each.

| Model, rule set | Threads | Wall before / after | Peak before / after | Bodies meshed before / after |
|---|---:|---:|---:|---:|
| m07, floors, clashes | 1 | 9.3 / 9.3 s | 719 / 655 MiB | 897 / 612 |
| m07, floors, clashes | 4 | 7.3 / 7.3 s | 1458 / 1355 MiB | 897 / 612 |
| m07, clashes | 1 | 7.5 / 5.8 s | 387 / 323 MiB | 897 / 595 |
| m07, clashes | 4 | 7.0 / 5.8 s | 382 / 319 MiB | 897 / 595 |
| m13, floors, clashes | 1 | 73.8 / 73.6 s | 1894 / 1904 MiB | 4781 / 4781 |
| m13, floors, clashes | 4 | 73.8 / 72.1 s | 1879 / 1901 MiB | 4781 / 4781 |
| m13, clashes | 1 | 78.6 / 71.1 s | 1834 / 1896 MiB | 4781 / 4781 |
| public architecture model, clashes | 1 | 0.3 / 0.3 s | 45 / 43 MiB | 14 / 9 |

What is held over the run: m07's 17 placement scenes (169 KiB) and m13's
157 (509 KiB) are now released before the first clash rule, where they
were held to the end of the check; the indexed bodies (8.2 MiB on m07,
24.4 MiB on m13) are read by every clash rule and released after the last,
as before. Nothing is evicted under the default budget. m13 meshed every
body still: it is an IFC2X3 model, and one clash rule selects a concept
bound to a class the IFC2X3 schema does not declare (`IfcDuctSegment`),
which the type hierarchy could not decide, so every object might have
been selected (on the public IFC2X3 architecture model that rule alone
meshed every body, any other of the set alone half of them; #449 below
bounds it). Every report is the same byte for byte before and after; the
result's `geometry` record and the model-data warnings now cover the
bodies meshed.

### A class the source's schema lacks selects nothing

Since [#449](https://github.com/axioval/engine/issues/449) the type
hierarchy tells the two refusals apart that made m13 mesh every body: an
object whose own kind it cannot place (`TypeHierarchyError::UnknownType`),
which may still be anything, and a class the source's schema does not
declare asked of a kind it does (`UnknownAncestor`), which no object of
that source is. The geometry demand keeps the first and drops the second,
so the IFC4 `IfcDuctSegment` of a clash rule no longer reaches every
object of an IFC2X3 model. The rule's selection reports both as before.

Measured as in #448, one release build per side (the side before is this
change with the demand reading `UnknownAncestor` as "may be"), one run
each, the parity clashes alone and after the free-floor circles and
rectangles ("floors, clashes"):

| Model, rule set | Threads | Wall before / after / `--mesh-all` | Peak before / after / `--mesh-all` | Bodies meshed before / after |
|---|---:|---:|---:|---:|
| m13, clashes | 1 | 71.6 / 68.8 s | 1872 / 1347 MiB | 4781 / 2720 |
| m13, clashes | 4 | 71.8 / 69.6 / 77.1 s | 1894 / 1354 / 1855 MiB | 4781 / 2720 |
| m13, floors, clashes | 4 | 72.3 / 69.8 / 72.8 s | 1884 / 1361 / 1837 MiB | 4781 / 2877 |
| m07, clashes | 1 | 5.8 / 5.8 s | 334 / 331 MiB | 595 / 595 |
| m07, clashes | 4 | 6.0 / 5.8 / 6.0 s | 332 / 330 / 384 MiB | 595 / 595 |
| m07, floors, clashes | 4 | 7.0 / 7.0 / 7.3 s | 1340 / 1360 / 1435 MiB | 612 / 612 |

m13 meshes 2720 of its 4781 bodies, those of the classes its clash rules
select, and peaks 520 to 540 MiB (28%) lower. Its wall time is the clash rules' own, so it drops by
2 to 3 s only. m07 is an IFC4 model, where `IfcDuctSegment` is declared,
and is unchanged. Every report is the same byte for byte as with
`--mesh-all`, and no run is checked again with every body.

A rule may still be checked again with every body where the host's answer
depends on bodies it left out: an exact body is registered only when some
tessellated body of the set meshed has one, so a check whose selected
bodies have none, while some other body has one, asks about every object
(a distance through a window, a model comparison of moved openings, in
`tests/check.rs`). The report is the same; the check runs twice.

### One geometry set for every service

Since [#450](https://github.com/axioval/engine/issues/450) the IFC-Axiolid
host holds each mesh once. It hands each of its geometry services a clone
of one `AxiolidGeometry`, and a clone used to copy every mesh: 18 copies
beside the set itself were held for the whole check. A set is now a handle
over data its clones share; a `with_` method on a shared clone copies the
set first, so nothing a service holds changes under it. Derivations over a
set (indexed bodies, plan footprints and shared areas, shells, composed
volumes) are keyed by the identity of that shared data, so services over
clones of one set read one entry for one body.

Measured as above, one release build per side, one run each; the public
parity models as in [Adapter caches on the store](#adapter-caches-on-the-store).

| Model, rule set | Threads | Wall before / after | Peak before / after | Steady before / after |
|---|---:|---:|---:|---:|
| m13 (4781 bodies, 32.4 MiB of meshes), clashes | 1 | 70.3 / 70.6 s | 1869 / 1285 MiB | 1813 / 1223 MiB |
| m13, clashes | 4 | 70.6 / 71.3 s | 1867 / 1285 MiB | 1814 / 1220 MiB |
| m13, every physical element | 4 | 1616 / 1593 s | 3471 / 2975 MiB | 1907 / 1294 MiB |
| m13, furniture | 1 | 5.3 / 5.3 s | 1111 / 1108 MiB | 1108 / 1107 MiB |
| m13, furniture | 4 | 4.3 / 4.3 s | 1164 / 1152 MiB | 1164 / 1149 MiB |
| m07 (595 bodies, 1.3 MiB of meshes), clashes | 1 | 5.8 / 5.8 s | 332 / 296 MiB | 311 / 284 MiB |
| m07, clashes | 4 | 5.8 / 5.8 s | 333 / 298 MiB | 313 / 285 MiB |
| m07, every physical element | 4 | 69.8 / 73.6 s | 1543 / 1365 MiB | 383 / 301 MiB |
| m07, furniture | 1 | 4.0 / 4.3 s | 635 / 625 MiB | 600 / 602 MiB |
| m07, furniture | 4 | 2.0 / 2.0 s | 1277 / 1317 MiB | 1131 / 1316 MiB |
| public parity models, 162 checks | 4 | 4.78 / 4.74 s | 102 / 98 MiB | |

The drop is the meshes times the copies removed: m13's clashes mesh every
body, 32.4 MiB of positions and indices, and 18 copies of them are
583 MiB, the 584 MiB its peak fell by. m07's clashes fell by 36 MiB, its
18 copies of 1.3 MiB of meshes and of the maps naming every object. The
furniture sets mesh only the furniture and spaces their rules select
(#448), so there is little to share, and their peaks are each run's
largest placement step, which varies between runs as before. Run times
are the same within noise; registering the services no longer copies
(0.23 s before, 0.001 s after on m13 among every physical element). Indexed bodies are built as often as
before on every case measured: no rule set here measures one body both
through the proximity service and through the derived relationships,
the two readers that held different copies. Every report is the same
byte for byte before and after.

### Independent rules at once

Until [#406](https://github.com/axioval/engine/issues/406) a check ran its
rules one after another; only each rule's subjects were judged on several
threads (#405). A rule set of many rules, each with few subjects or a
serial step, used one core. With `Runtime::with_rules_at_once` (the CLI's
default on more than one thread, `--serial-rules` to turn it off) the
runtime runs as many rules at once as it has threads: a rule starts once
every rule it reads has finished, the first in the plan's order among
those ready, and holds one of the run's threads while it runs. Its
subjects are judged on the threads no rule holds (`parallel::Pool`):
`map_ordered` starts a worker only on a thread it can take at once, before
each next subject, so a rule takes more threads as the rules beside it
finish, and rules and subjects together never keep more threads busy than
`--threads`. What each rule reports is merged in the plan's order.

Measured on a release build of each side, one run each, wall time and
peak resident memory of the whole check (`rss.py`, every run alone under
an exclusive lock and a 20 GB limit): *before* is the build of #450,
*serial* this build with `--serial-rules`, *at once* this build's default.
The rule sets are the public parity cases' and b448's free-floor circle
and rectangle searches among furniture plus the parity clashes, over the
private models m07 and m13.

| Model, rule set (rules) | Threads | Wall before / serial / at once | Rules phase before / at once | Peak before / at once |
|---|---:|---:|---:|---:|
| m13, routes (20) | 4 | 48.6 / 47.3 / 19.0 s | 42.2 / 12.6 s | 1324 / 1347 MiB |
| m13, routes | 8 | | / 7.7 s | / 1393 MiB |
| m13, openings (60) | 4 | 31.8 / 31.5 / 25.5 s | 25.4 / 19.1 s | 1264 / 1301 MiB |
| m13, openings | 8 | | / 13.8 s | / 1350 MiB |
| m13, furniture circles and rectangles, clashes (12) | 4 | 71.3 / 71.8 / 28.0 s | 65.0 / 23.4 s | 1295 / 1286 MiB |
| m13, the same | 8 | | / 17.4 s | / 1296 MiB |
| m07, furniture circles and rectangles, clashes | 4 | 7.3 / 7.0 / 3.5 s | 6.4 / 2.8 s | 1312 / 983 MiB |
| m07, floors (26) | 4 | 275.8 / 273.3 / 282.1 s | 274.9 / 281.1 s | 2618 / 2344 MiB |
| m07, floors | 8 | | / 250.2 s | / 3145 MiB |
| m13, spans (17) | 4 | 671.4 / 664.1 / 660.1 s | 664.6 / 653.6 s | 4386 / 4318 MiB |
| m13, spans | 8 | | / 411.1 s | / 4565 MiB |
| m13, judges (29, no geometry) | 4 | 3.0 / 2.8 / 2.8 s | 0.59 / 0.33 s | 797 / 820 MiB |
| m07, openings | 4 | 2.3 / 2.3 / 2.0 s | 1.24 / 1.09 s | 311 / 324 MiB |

Rule sets of many rules whose subjects are few or judged serially gain
most: the routes (an accessibility-like set of accessible-route,
escape-route and space-distance rules) ran 2.6 times as fast on four threads, and the free-floor and
clash set 2.5 times, the clashes running beside the searches. A set
dominated by one long rule whose subjects already filled the threads
gains nothing on four (spans, floors): its other rules end early and the
long rule takes their threads back only between its subjects, so floors
over m07 ran 2 to 4% slower (281 and 288 s against 273 and 277 s
serially, two runs each), within what one rule's subjects judged on one
thread fewer for a while cost. On eight threads both gain. CPU time is
the same either way; peaks moved by less than the runs' own spread except
where several heavy rules overlap (floors on eight threads, 3145 MiB
against 2618 on four before), which the memory budget bounds for their
steps but not for what each rule holds beside them. The public parity
models check in under a second either way. Every report, with
`--rule-status`, is the same byte for byte in every variant; the
integrity diagnostics of the before build differ on m13's clashes, from
the geometry it was built to mesh, not from the rules.

### Rectangle searches that learn as they go

The free-floor rectangle at any orientation was most of a check among
every physical element: on m13 (157 spaces) with two circles and two
rectangles, the 1.8 × 1.5 m rectangle took 1704 of the check's 1743 s
on four threads. Timed inside the search on that model (the rectangle
alone, 510 searches, 4560 s summed over the threads; axiolid-overlay
0.3.18, the build before the last change of the search):

| Where the time went | Before | After |
|---|---:|---:|
| Searches that ran out of their 512 intervals (75) | 3798 s | 618 s |
| Searches proving no placement (107) | 412 s | 286 s |
| Searches finding one (328) | 351 s | 256 s |
| Of all: the bracket of the largest rectangle | 4337 s | 998 s |
| Of all: the decision at the size asked | 223 s | 161 s |
| Absences over the coarse obstacles | 2603 s | 364 s |
| Absences over the obstacles as built | 986 s | 126 s |
| Centres among the obstacles simplified from around | | 112 s |
| Witness regions | 849 s | 322 s |
| Scenes, proofs over every angle | 121 s | 113 s |

Nearly all of every step was the overlay's Minkowski sum of the
obstacles with the rectangle (in one search 16.0 of 16.3 s; the
difference after it 0.1%), 30 to 120 ms per sum along tessellated
furniture of 150 to 300 corners within the rectangle's reach. The
searches that ran out of intervals descend towards an angle where the
rectangle neither fits nor is excluded until floating point halves
their intervals no more: about four in five of their sums repeated an
earlier interval's rectangle bit for bit, and 20 of the 510 searches
repeated an earlier search of the same space and size. Since
[#427](https://github.com/axioval/engine/issues/427) a scene keeps
every search asked of it and the coarse proofs' centres, and a search
learns within itself which steps cannot change its course: a centre
standing with room to spare shows an absence unprovable, centres among
the sure obstacles simplified from around show it too, and an interval
whose shrunk rectangle has no witness centres has none at any rotation
(see [Free space](./free-space.md)). What is left is one or two sums
per interval near the angle a search cannot decide.

Release builds, one run each, wall time and peak resident memory from
`getrusage` under a 20 GB cgroup limit on a 20-core machine; the rule
sets as in [Scenes and body indexes built once per check](#scenes-and-body-indexes-built-once-per-check),
the public parity case `floors` on the parity models.

| Model, rule set | Threads | Wall before / after | Peak before / after |
|---|---:|---:|---:|
| m13 (157 spaces), every physical element | 4 | 1712 / 489 s | 2486 / 2671 MiB |
| m13, every physical element | 1 | 4220 / 1191 s | 2148 / 2146 MiB |
| m13, furniture | 1 | 5.3 / 5.3 s | 1108 / 1105 MiB |
| m13, furniture | 4 | 6.8 / 4.5 s | 1120 / 1122 MiB |
| m07 (17 spaces), every physical element | 1 | 100.1 / 24.8 s | 717 / 745 MiB |
| m07, every physical element | 4 | 85.1 / 20.8 s | 1338 / 1363 MiB |
| m07, furniture | 1 | 4.3 / 4.0 s | 627 / 621 MiB |
| m07, furniture | 4 | 2.8 / 2.3 s | 1061 / 1052 MiB |
| public parity models, `floors` | 4 | 0.31 / 0.30 s | 98 / 99 MiB |

Both sides on axiolid-overlay 0.3.19, after independent rules run at
once (#406). On m13 the 1.8 × 1.5 m rectangle went from 1703 to 481 s
on four threads, and on m07 from 92.0 to 17.9 s on one. Four threads
gain less than the sum suggests: a space is searched on one thread, and
the spaces whose bracket runs out of intervals several times take
minutes each (on m07 one space is most of the rule). The furniture sets hold no such space and take what they took;
peaks vary with each run's largest placement step, as before. Every
report is the same byte for byte before and after, on one thread and
on four.

### Plan overlays under the kernel's memory budget

Since [#433](https://github.com/axioval/engine/issues/433) the Axiolid
adapter builds plan overlays and ring-soup unions through the kernel's
`overlay_within` and `union_soup_within` (axiolid-overlay 0.3.19, which
also made a circumscribed grown soup's overlay cost about its outline):
the subdivision is refused past 1 GiB, or the check's memory budget where
that is smaller, before it allocates more. The edge-pair budget of these
calls went from 4,000,000 to 50,000,000 pairs; it bounds only what the
kernel does not charge.

Calibrated on the kernel alone (release, heap high-water by a counting
allocator), unbudgeted unless stated:

| Input | Pairs | Time | Memory |
|---|---:|---:|---:|
| bar of 128 facets, circumscribed growth, union | 10.5 M | 0.38 s | 70 MB |
| the same, 256 facets, union / intersection with the bar | 41 / 43 M | 1.8 / 2.1 s | 289 / 300 MB |
| the same, 256 facets, union with itself | 41 M | 6.6 s | 756 MB |
| 1,000 slivers crossing at every pair | 4.0 M | 246 s | 5.2 GB |
| the same under a 1 GiB budget | 4.0 M | refused after 0.9 s | 40 MB |
| 500 such slivers under a 1 GiB budget | 1.0 M | refused after 22 s | 0.65 GB |
| 3,000 such slivers under a 256 MiB budget | 36 M | refused after 13 s | 0.36 GB |
| 3,000 parallel slivers, no crossing | 27 M | 3.9 s | 19 MB |
| 6,000 parallel slivers, no crossing | 108 M | 19 s | 38 MB |

The old pair budget admitted the 1,000 crossing slivers (246 s, 5.2 GB)
and refused the grown 128- and 256-facet bars. The kernel's budget bounds
the first; what it does not charge, the snap, the chain reduction and box
pairs tested without a crossing, grows with the pairs at 0.15 to 0.4 µs a
pair, which the raised pair budget keeps under some 20 s.

Through the adapter (`measure_uncovered_area`, a 6 m bar over another
grown by 2 cm, and #419's crossing combs grown by 1 cm; test profile with
optimized dependencies, one run each):

| Case | Before | After |
|---|---|---|
| bar of 64 facets | exact, 0.76 s, 45 MB | exact, 0.82 s, 47 MB |
| bar of 128 facets | united bracket 3.7·10⁻⁵ m² wide, 0.09 s, 8 MB | exact, 4.2 s, 167 MB |
| bar of 256 facets | united bracket, 0.19 s, 12 MB | united bracket (needs 1.35 GB), 3.3 s, 400 MB |
| combs of 100 teeth | exact, 3.7 s, 241 MB | exact, 3.7 s, 245 MB |
| combs of 600 teeth | refused by pairs, 1.4 s, 40 MB | refused by the kernel, 3.1 s, 735 MB |

On the private models and the public parity models (release builds, one
run each as in [Independent rules at once](#independent-rules-at-once)):

| Model, rule set | Threads | Wall before / after | Peak before / after |
|---|---:|---:|---:|
| m13, clashes | 1 | 70.6 / 68.1 s | 1252 / 1251 MiB |
| m13, clashes | 4 | 30.3 / 27.5 s | 1271 / 1261 MiB |
| m13, every physical element | 4 | 1752 / 1606 s | 2490 / 2526 MiB |
| m13, furniture | 4 | 4.8 / 5.8 s | 1152 / 1110 MiB |
| m07, clashes | 1 | 6.3 / 6.3 s | 286 / 282 MiB |
| m07, clashes | 4 | 2.8 / 2.8 s | 310 / 314 MiB |
| m07, every physical element | 4 | 84.8 / 78.8 s | 1325 / 1347 MiB |
| m07, furniture | 4 | 2.3 / 3.3 s | 1171 / 1073 MiB |
| public parity models, 162 checks | 4 | 4.38 / 3.73 s | 97 / 101 MiB |

No overlay of these models reached either budget, so the differences are
the runs' own spread. Every report is the same byte for byte, and the
determinism gate's 64 MiB budget refuses no overlay.
