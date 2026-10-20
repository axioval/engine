# Memory budget

A check's memory is mostly its models, their meshes and a few heavy
geometry steps: the overlays and morphologies of a free-floor search, each
building an arrangement of every pair of edges that meet, and dropping it
before the next. One such step can take gigabytes, and with several
threads each running its own, they used to add up: a check of four
free-floor rules over one private sample was killed at 24 GB on four
threads, and three checks in one host peaked at 29.6 GB
([#407](https://github.com/axioval/engine/issues/407)).

## The budget

`axioval_engine::MemoryBudget` states how much memory a check's heavy
steps may take at once. A host sets it on the runtime
(`Runtime::with_memory_budget(Arc<MemoryBudget>)`), or shares one budget
between the runtimes of the checks it runs at once; the CLI takes
`--memory-budget SIZE` (else `AXIOVAL_MEMORY_BUDGET`, else half the memory
the system reports available, at least 6 GiB; see
[Command line](./cli.md#axioval-check)).

The runtime enters the budget on the thread running the plan, on every
thread running rules at once (#406), and `Parallelism::map_ordered` on
every worker, so a step reads the budget of the check it belongs to
(`axioval_engine::memory::current`), and the heavy steps of rules run at
once are bounded together as those of one rule's subjects are. A geometry
adapter estimates each heavy step before building it and asks to be
admitted (`memory::admit`), holding the returned `Admission` while it
builds:

- A step that fits beside the steps already admitted runs at once.
- One that would pass the budget waits until another finishes. With
  nothing else admitted, it runs alone, so a step within the budget is
  always run.
- A step whose own estimate is more than the whole budget is refused, by
  name, and what needed it is not evaluated: `the erosion of the free
  floor by the rectangle was not built: at its 1900000 pairs of meeting
  edges its estimated 3.6 GiB is more than the check's memory budget of
  2.0 GiB`. The refusal depends on the step's estimate and the budget
  alone, never on what else is running, so it is the same on any number
  of threads.

Waiting only costs time. Which steps run, and what they find, is what a
serial run finds, so a report is the same byte for byte under any budget
and any number of threads, but for those refusals. The determinism gate
([#441](https://github.com/axioval/engine/issues/441)) holds it: every
case also runs on four threads under a budget of 64 MiB, below what the
`coverage` case's overlays take together on the public architecture
models (so they wait) and above their largest step (37 MiB). No step the Axiolid
adapter's own per-step budgets admit is estimated past 6 GiB, so the
default budget refuses none, on any machine.

An admission is held only while its step builds, never across a lock or a
wait for another thread, and a thread already holding one is admitted at
once (its nested step is part of the one it runs), so a waiting step
always waits for one that finishes.

### Estimates

The Axiolid adapter counts the pairs of edges whose boxes meet before every
overlay and Minkowski operation (`planar/budget.rs`, #419, #434), and
estimates its memory from them: 200 bytes a pair for an overlay, 2 KiB for
a sum or erosion by a polygon, 100 bytes for a disc morphology, and 256
bytes an edge. A circle search's disc morphologies are charged too, their
pairs counted over the region with every corner dropped that lies within
the fast mode's 1 mm of the chord across it, as the overlay fills such
notches; the search's step and pair budgets then apply to them as to the
rectangle's, and the one disc step of a private sample that asked for
20 GB is refused by name. Since #432 the region booleans and exact disc
morphologies of the other services are estimated and admitted the same
way, one step at a time.

Plan overlays and ring-soup unions are bounded by the kernel itself since
[#433](https://github.com/axioval/engine/issues/433) (axiolid-overlay
0.3.19, `overlay_within` and `union_soup_within`): the adapter hands the
kernel a memory budget, and its subdivision charges what it is about to
hold before allocating it and refuses past the budget, so an admitted
overlay holds at most what it was admitted with. The budget is at most
1 GiB, or the check's budget where that is smaller. A step is admitted
with the budget it is tried under and 16 bytes for every pair of meeting
edge boxes, for the snap and chain reduction that run before the kernel's
charges (0.36 GB at 36 million pairs, measured). It is first tried under
8 KiB an edge and 32 bytes a pair (a grown bar's soup needed 15 to 24
bytes a pair, a self-union 7.4 KB an edge), then, refused, under four
times what the kernel had charged, up to the cap. The kernel's budget is
never taken from what is free or held: the cap depends on the check's
budget alone, so the refusal does too.

The estimates are of a step's arrangement, not of the process. The models,
meshes, a session's caches, each thread's scene and what each rule run at
once holds outside its steps (its selection, its outcomes, what it keeps
for itself) come on top, and an
estimate can be below what a step takes: traced one by one on a private
sample, the rectangle search's steps took up to 1.8 times their estimate
at 1.5 million pairs and more for some smaller ones.

### For a derivation store

Values kept on purpose between steps, such as a derivation store's
(#437, #440), count against the same budget through `MemoryBudget::hold`.
The store holds each kept value's bytes, as its kind's `size_hint` states
them, while it keeps the value (#408): a placement scene by its regions'
corners, an indexed body by its triangles. A kind may also bound what the
store keeps of it (`DerivationKind::keep_bytes`); the Axiolid adapter's
scenes and indexed bodies keep at most a quarter of the budget each, and
past it the values read least recently are released. Held bytes are never
refused and never wait, and what they hold never takes a step's room
([#440](https://github.com/axioval/engine/issues/440)):

- **After the last reader.** A kind names the services it is read through
  (`DerivationKind::read_through`), and the runtime releases its values
  before the first rule after the last one that may ask any of them
  (`DerivationStore::release_unread`), as the rules' capabilities state
  what they may ask (#410, #448). A free-floor rule asks the free-space
  service, a clash rule the proximity service, a property rule no
  geometry, so scenes are released once the last free-floor or
  circulation rule has run, and body indexes once the last rule that may
  measure a pair has. Where a capability may ask anything (it records
  why), the values wait for it.
- **Within the budget.** Whenever a value lands or grows, the store
  releases the values read least recently, of any kind, while what it
  keeps and what running steps were admitted with pass the check's budget.
  The run registers its store with the budget (`MemoryBudget::reclaim_from`),
  so a step that does not fit beside the kept values has them released
  before it waits. A released value is built again when read again.
  The measurement memo's tables are kinds of the same store (#451): they
  name no services and state no bytes, so they wait for the run's end
  unless a bound or the budget evicts them with the values around them.

`MemoryBudget::headroom` tells a keeper what is left before it evicts,
and `MemoryBudget::check` refuses a step taken from a kept value exactly
as building it would be, so a refusal never depends on what was kept.
`MemoryBudget::usage` reports the most admitted and held at once, the
largest step, how many steps waited and how much kept memory was released
for them; `check --timings` prints it, and `check --stats` one line a
derivation kind with the most it held at once, its values released (by
name or after their last reader) and those evicted to stay within a bound
or the budget.

## Returning memory

The `axioval` binary uses mimalloc as its global allocator; no library
sets one. The system allocator kept what each thread freed in arenas of
its own, so the resident set stayed near the check's heaviest moment long
after it, and near the sum of every thread's. The allocator was chosen by
measurement (below): mimalloc and jemalloc both returned freed pages, with
about the same peak and run time; mimalloc builds from the crate without a
system library. A check's placement scenes are kept in its derivation store
(#408), bounded and held against the budget as above, each with at most
250,000 corners of found regions (#427), and dropped with the store at the
end of the run.

The model's meshes are not counted against the budget: they are the
source, held for the whole check. They are held once (#450): every
geometry service of the IFC-Axiolid host reads one shared set, where each
held a copy before: 18 beside the set itself, which on m13 were 583 MiB
of copies of its 32.4 MiB of meshes (see [One geometry set for every
service](./performance.md#one-geometry-set-for-every-service)).

## Measurements

Release builds, peak resident memory from `getrusage`, steady memory the
median of the last quarter of a run's samples (every 0.25 s), under a
20 GB cgroup limit, one run at a time on a 20-core machine. "Before" is
the engine before #407 with the system allocator.

m13 (157 spaces, one free-floor circle rule), the default budget:

| Run | Threads | Wall | Peak | Steady | At the end |
|---|---:|---:|---:|---:|---:|
| before | 1 | 99.2 s | 2.25 GiB | 2.09 GiB | 1.88 GiB |
| before | 4 | 40.3 s | 3.25 GiB | 3.05 GiB | 2.17 GiB |
| after | 1 | 89.4 s | 2.18 GiB | 1.98 GiB | 1.70 GiB |
| after | 4 | 33.0 s | 3.33 GiB | 2.66 GiB | 1.72 GiB |

No step waits on m13: at most 7 MiB is admitted at once. Its memory above
one thread's is each thread's own scene and search, which the budget does
not count, and its reports are the same byte for byte.

m11 with four free-floor rules (three circles, one rectangle), on four
threads but where stated:

| Run | Wall | Peak | Steady | At the end | Steps waited |
|---|---:|---:|---:|---:|---:|
| before | killed at the 20 GB limit after 72 s (100 s on one thread) | | | | |
| budget, system allocator | 282 s | 9.31 GiB | 8.52 GiB | 2.38 GiB | 0 |
| budget, jemalloc | 279 s | 9.69 GiB | 3.92 GiB | 0.89 GiB | 0 |
| after (budget, mimalloc), default 13.7 GiB | 269 s | 8.78 GiB | 4.78 GiB | 1.17 GiB | 12 |
| after, `--memory-budget 4G` | 588 s | 5.44 GiB | 2.87 GiB | 1.15 GiB | 289 |
| after, one thread | 832 s | 3.70 GiB | 2.29 GiB | 0.57 GiB | 0 |

Before, a circle of 12 m among a storey's walls and furniture built one
disc morphology past 20 GB on any number of threads. It is now refused by
the disc budget (its 20 million counted pairs), the check finishes, and
its report is the same on one and four threads and under the default and
the 4G budget. The 4G budget lowers the peak by 3.3 GiB at twice the wall
time; a budget of 2G refuses the five rectangle steps estimated past it,
by name, and nothing else changes. The allocator halves the steady
memory and returns all but the models' at the end. Trimming the system
allocator after each space (glibc's `malloc_trim`) was not measured: an
allocator that returns freed pages itself needs no call from the engine
and does so on every platform. The rows comparing allocators were taken
before the polygon estimate was raised from 1 KiB to 2 KiB a pair, which
changes when steps wait, never what they find.
