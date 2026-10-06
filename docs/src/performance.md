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
   a global allocator (`peak_alloc`, a bench-only dependency).

Run time is wall time; the medians of interleaved runs absorb a machine's
drift, not its load, so a gate runs where nothing else builds.

`scripts/bench.py` runs the bench and judges its records against
`scripts/bench_budget.json`:

- the median run time over the reference's, per input whose reference
  takes at least `floor_ns` (100 µs); inputs below it are judged together,
  by the sum of their medians, since a run of a few microseconds is mostly
  timer noise;
- the median peak heap over the reference's, per input;
- the two sides agreeing under the parity contract;
- in a gate, a fixture and a public model measured, and every public model
  the case names fetched.

A gate whose only failures are run times over budget measures once more and
judges that measurement; a real regression fails both. `scripts/test_bench.py`
(in the `lint` gate) is its self-test.

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
| generated fixture, 400 walls | 421 | 1.21× / 1.43× | 1.24× / 1.11× | 1.38× / 1.00× | 1.11× / 1.20× |
| building architecture (IFC4) | 38 | 1.15× / 1.15× | 1.03× / 1.03× | 1.07× / 1.01× | 0.70× / 0.97× |
| building structural (IFC4) | 33 | 0.98× / 1.03× | 1.02× / 1.03× | 1.09× / 1.01× | 1.03× / 0.96× |
| building HVAC (IFC4) | 18 | 1.01× / 1.00× | 0.95× / 1.00× | 1.03× / 1.00× | 1.14× / 1.00× |
| infra road (IFC4) | 116 | 1.18× / 1.27× | 1.02× / 1.02× | 1.09× / 1.02× | 1.02× / 1.03× |
| building architecture (IFC2x3) | 36 | 1.15× / 1.15× | 1.03× / 1.03× | 1.08× / 1.01× | 0.72× / 0.96× |
| building structural (IFC2x3) | 30 | 1.05× / 1.03× | 1.03× / 1.03× | 1.13× / 1.01× | 0.51× / 0.97× |
| building architecture (IFC4x3) | 37 | 1.16× / 1.15× | 1.02× / 1.03× | 1.07× / 1.01× | 0.69× / 0.97× |
| building structural (IFC4x3) | 33 | 0.99× / 1.03× | 1.02× / 1.03× | 1.08× / 1.01× | 1.04× / 0.96× |
| wall with opening and window | 9 | 1.11× / 1.05× | 0.99× / 1.01× | 1.04× / 1.01× | 1.05× / 0.86× |
| tessellated column | 3 | 0.99× / 1.00× | 0.91× / 1.00× | 1.08× / 1.01× | 1.05× / 1.00× |

The templates of #290, measured by the same gate:

| input | objects | level-spacing | object-count | related-count | unique-value |
| --- | ---: | --- | --- | --- | --- |
| generated fixture, 400 walls | 421 | 0.83× / 1.00× | 1.20× / 1.24× | 0.99× / 1.00× | 1.14× / 1.12× |
| building architecture (IFC4) | 38 | 1.11× / 1.13× | 1.09× / 1.04× | 1.15× / 1.12× | 1.04× / 1.00× |
| building structural (IFC4) | 33 | 1.08× / 1.12× | 1.12× / 1.04× | 1.15× / 1.05× | 1.04× / 1.00× |
| building HVAC (IFC4) | 18 | 1.10× / 1.12× | 1.15× / 1.04× | 1.18× / 1.11× | 1.00× / 1.00× |
| infra road (IFC4) | 116 | 0.68× / 1.11× | 1.04× / 1.01× | 1.02× / 1.00× | 1.00× / 1.00× |
| building architecture (IFC2x3) | 36 | 1.14× / 1.13× | 1.10× / 1.04× | 1.15× / 1.12× | 1.05× / 1.02× |
| building structural (IFC2x3) | 30 | 1.10× / 1.13× | 1.11× / 1.05× | 1.16× / 1.05× | 1.04× / 1.00× |
| building architecture (IFC4x3) | 37 | 1.19× / 1.12× | 1.11× / 1.04× | 1.14× / 1.12× | 1.04× / 1.02× |
| building structural (IFC4x3) | 33 | 1.20× / 1.11× | 1.11× / 1.04× | 1.15× / 1.05× | 1.05× / 1.00× |
| wall with opening and window | 9 | 1.08× / 1.10× | 1.23× / 1.08× | 1.19× / 1.05× | 1.06× / 1.02× |
| tessellated column | 3 | 0.85× / 1.00× | 1.34× / 1.02× | 1.01× / 1.00× | 1.01× / 1.00× |

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
fixture (no spaces) and the public models whose spaces the service cannot measure it runs at 0.5
to 0.75× its reference, and at 1.64 to 1.68× on the three architecture models,
whose references take under 90 µs (judged together by the floor, where it
holds); peak heap 0.76 to 1.28×.

The generic judges of #287 run the `judges` case's rules of their
capability, measured by the same gate:

| input | objects | consistent-value | selector-conformance | relative-count |
| --- | ---: | --- | --- | --- |
| generated fixture, 400 walls | 421 | 1.12× / 1.15× | 1.01× / 0.99× | 1.09× / 1.08× |
| building architecture (IFC4) | 38 | 1.11× / 1.01× | 1.02× / 1.00× | 1.11× / 1.03× |
| building structural (IFC4) | 33 | 1.10× / 1.01× | 1.00× / 1.00× | 1.08× / 1.02× |
| building HVAC (IFC4) | 18 | 1.18× / 1.01× | 1.00× / 1.00× | 1.09× / 1.07× |
| infra road (IFC4) | 116 | 1.14× / 1.00× | 1.00× / 1.01× | 1.04× / 1.02× |
| building architecture (IFC2x3) | 36 | 1.11× / 1.01× | 1.00× / 1.00× | 1.09× / 1.02× |
| building structural (IFC2x3) | 30 | 1.10× / 1.01× | 0.97× / 1.00× | 1.10× / 1.02× |
| building architecture (IFC4x3) | 37 | 1.08× / 1.01× | 1.00× / 1.00× | 1.10× / 1.03× |
| building structural (IFC4x3) | 33 | 1.12× / 1.01× | 1.01× / 1.00× | 1.09× / 1.02× |
| wall with opening and window | 9 | 1.07× / 1.07× | 1.01× / 1.00× | 1.18× / 1.05× |
| tessellated column | 3 | 1.03× / 1.05× | 1.03× / 0.99× | 0.98× / 1.00× |

`consistent-value` runs six rules, `selector-conformance` and `relative-count` four each.

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
