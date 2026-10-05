# Composing rules

Many requirements are not one built-in check but a composition: a limit
taken from a table by a stated class, a measured slope compared with it,
and only for the objects a selector picks. This chapter shows how such a
rule is put together from the engine's layers, how the engine decides it
three-valued over intervals, and walks through four worked examples. The
requirements in them are generic and their numbers illustrative; none is
taken from a particular standard.

Every package shown here is a tracked file under `docs/examples/composing`,
and `crates/engine/rules/tests/composing_examples.rs` compiles each pair
against the built-in capabilities and runs it over a small in-memory
project, so the book shows exactly what is tested:

```sh
cargo test -p axioval-rules --test composing_examples
```

## The layers

A composed rule is built bottom-up, each layer reading only the one below:

| Layer | What it is | Where it is described |
| --- | --- | --- |
| Measured and stated values | Properties a source states, and values built-in code measures from geometry: `slope`, `cross_fall;axis=own_x`, `extent_z`, a flight's `steps` with their `riser` and `going`. Each is an interval with evidence. | [Derived properties](./derived.md#measured-values) |
| Expressions | A total, pure tree combining values: arithmetic with units, comparisons, `between`, `if`, `lookup` in a table, aggregates over members. | [Expressions](./expressions.md) |
| Requirements and selectors | An expression as the `requirement` of `axioval:capability.expression` judges each object; a selector (property, entity type, `expression`) decides which objects a rule applies to. | [Capabilities](./capabilities.md#expression-requirements) |
| Rulesets | A definitions package declares the concepts, their names in a type system and a definition with its parameters (tables included); a ruleset binds rule instances to it with values, severity and applicability. | [Source-neutral IR](./ir.md), [Concept binding](./concept-binding.md) |

Values are measured by trusted built-in code over the host's geometry
services, never by a package: a package only names them. The definitions
package of each example names its concepts in the IFC 4.3 type system
(`IfcSlab`, `IfcStairFlight`, a property set and its properties); a source
written in another type system needs the concepts' names there.

## Three values over intervals

Every number is an interval in coherent SI units: a point when known
exactly, wider when measured on a tessellated body or stated with a
tolerance. A comparison is decided only when every value the intervals
allow gives the same answer, so every requirement comes out one of four
ways:

| Requirement | Outcome |
| --- | --- |
| true | the object passes |
| false | a finding, naming the labelled subexpression that failed and every value read with its evidence |
| `null` | a missing-information finding: a value the requirement needs is stated absent, so it cannot be confirmed |
| undecided | the object is **not evaluated**, with the reason and the path to the subexpression that could not be decided |

A measurement straddling its bound is never a pass and never a finding.
`and` is false as soon as one operand is false and `or` true as soon as one
is true, whatever the others are; otherwise an undecided operand leaves the
whole undecided, and a `null` one leaves it `null`. `implies(a, b)` is
`or(not a, b)` under the same rules. The truth tables are in
[Expressions](./expressions.md#truth).

## `null` is not "not evaluated"

The two kinds of missing value never mix:

- **`null`** is a value: the source states the property without a value,
  or states that the object does not carry it. A lookup with no matching
  row, and a member field a measurement defines as absent (the going of a
  flight's first step), are `null` too.
- **Not evaluated** means a value could not be read or measured (an
  unreadable property, a refused or unavailable measurement), or a result
  cannot be decided (a straddling interval).

A comparison with `null` is `null`, and so is `not` of it: `not(x == 5)`
never holds for an object that states no `x`. `isDefined` of it is false,
and arithmetic on it is `null` again. A requirement that comes out `null`
is a missing-information finding, never a pass. A value that is not
evaluated keeps everything reading it not evaluated, unless the result is
decided either way (`and` with another operand false). In a selector, an
expression that is `null` selects nothing, so the object is not selected;
an unreadable one leaves the selection undecided and the object not
evaluated in that rule, never silently skipped.

Whether an absent value passes or fails is the author's decision, and the
expression states it:

- `implies(isDefined(x), test)` holds where `x` is `null`: the requirement
  applies only where a value is stated.
- `and(isDefined(x), test)` fails where `x` is `null`: a value is required.
- `coalesce(x, default)` reads an absent `x` as the default.
- Without either, a `null` `x` is a missing-information finding.

Neither guard turns an unreadable `x` into a pass or a finding:
`isDefined` of a value that cannot be read is itself not evaluated.

Over a measured member list, `all` holds when every member's value does
(and is false over no member), `none` when no member's does (and is true
over none). A member value that is `null` is never skipped: it leaves
`all`, `any` and `none` `null` unless another member decides. A guard
inside the value skips the members a field does not apply to:
`all(implies(isDefined(going), …))` judges every step that has a going,
and `none(and(isDefined(gap), gap > 0.15 m))` every piece followed by a
gap. A member value that is not evaluated leaves the aggregate not
evaluated.

## Example: an embankment slope limit by soil class

*Every embankment side slope is no steeper than the rise per run its soil
class allows: rock 1, gravel 0.5, sand 0.4, clay 0.25.*

The definition declares, beside the expression capability's own
`requirement`, `deviation` and `message`, a `table` parameter
`slope_limits` with a key column `soil` and a value column
`maximum_ratio`. The rule fills the table and reads it with `lookup`,
keyed by the stated soil class. A key cell is a wildcard pattern over the
key read as text, and the single most specific matching row applies; no
matching row gives `null`.

The requirement is an `and` of three labelled operands: the soil class is
stated, the table has a limit for it, and the steepest piece of the fill's
top is at most that limit. That operand is a `max` aggregate over the
measured member list `face_pieces`: each planar or smooth piece of the top
face, its `slope` restated by `convertSlope` from an angle to a rise per
run. Because the first two are separate operands, a finding says which of
the three failed.

The fill is one body: a level crest between two batters. The measured
value `slope` would be the hull over the whole top, from the crest's zero
to the batters' fall, and would leave every limit undecided. Piece by
piece, the crest's zero is only the least member, so the steepest batter
decides. A batter warped within itself is still one piece, and its own
range can straddle the limit. `all(implies(area > 1 m², slope ≤ limit))`
over the same list would judge every batter but leave small pieces out,
and `slope;face=facing;direction=1,0,0;tolerance=60` reads only the pieces
of the body facing east ([Slopes, falls and
tilts](./derived.md#slopes-falls-and-tilts)).

```json
{{#include ../examples/composing/embankment.definitions.json}}
```

```json
{{#include ../examples/composing/embankment.ruleset.json}}
```

| Object | Stated and measured | Outcome |
| --- | --- | --- |
| `steady` | sand, a level crest and batters of 0.3 | passes |
| `steep` | clay, a level crest and batters of 0.4 | finding: `slope within the limit` is false |
| `uneven` | sand, one batter warped from 0.3 to 0.5 | not evaluated: 0.3–0.5 straddles 0.4 |
| `unclassified` | soil class `null` | finding: `soil class stated` is false |
| `unlisted` | `peat`, no row | finding: `limit for the soil class` is false |
| `unread` | soil class unreadable | not evaluated |

## Example: a deck cross-fall band

*External deck slabs fall across their length by at least 1 % and at most
3 %: steep enough to drain, flat enough to walk on.*

The selector does the first half: applicability is an `allOf` of the deck
type and a property selector `IsExternal` equals `true`. The requirement is
one `between`: the measured `cross_fall;axis=own_x` (the fall across the
deck's own x axis, read from its placement) as a percentage, between two
authored `number` parameters, which the rule reads with `parameter`.

```json
{{#include ../examples/composing/deck.definitions.json}}
```

```json
{{#include ../examples/composing/deck.ruleset.json}}
```

| Object | Stated and measured | Outcome |
| --- | --- | --- |
| `draining` | external, 2 % | passes |
| `flat` | external, 0.5 % | finding |
| `warped` | external, pieces from 2.5 % to 3.5 % | not evaluated: straddles 3 % |
| `internal` | `IsExternal` false, level | not selected: no outcome |
| `unstated` | `IsExternal` `null`, level | not selected: no outcome |
| `unread` | `IsExternal` unreadable | not evaluated: its selection is undecided |

The two decks the selector leaves out are level and would fail were they
checked; only the one whose flag cannot be read is reported, as not
evaluated.

## Example: a stair riser and going relation

*Two risers and one going of every step of a flight measure between
0.60 m and 0.65 m, and no riser is higher than 0.19 m.*

The flight's `steps` are a measured member list: one member per riser,
bottom to top, with the fields `riser` and `going` in the reserved set
`axioval:member`. An aggregate ranges over them. The step formula is
`all` over the steps of `implies(isDefined(going), between(2 × riser +
going, …))`: the first step climbs from the floor and has no tread below
it, so its going is `null` and the guard skips it. The riser limit is
`none` over the steps of `riser > 0.19 m`. (The member field `step_length`
is `2r + g` already; the example spells it out to show the arithmetic.)
The bounds are authored `quantity` parameters, so their units are checked
when the ruleset compiles.

```json
{{#include ../examples/composing/stair.definitions.json}}
```

```json
{{#include ../examples/composing/stair.ruleset.json}}
```

| Object | Measured | Outcome |
| --- | --- | --- |
| `comfortable` | 12 risers of 0.17 m, goings 0.28 m: 0.62 m | passes |
| `steep` | 10 risers of 0.20 m, goings 0.28 m: 0.68 m | finding: `every step within the step formula` is false |
| `rough` | 0.18 m and 0.29 m, every position known to 3 mm: 0.632–0.668 m | not evaluated: straddles 0.65 m |
| `single` | one riser of 0.17 m, its going `null` | passes: the guard skips the step, no riser is too high |
| `unmeasured` | the service measures no flight | not evaluated |

## Example: a combined exposure and cover rule

*Where a slab states an exposure class, its stated cover is at least the
class's minimum (20, 30, 40 or 50 mm), 10 mm more when prestressed, and the
measured thickness leaves a core of at least 100 mm between both covers.*

The requirement is an `implies` whose antecedent is
`isDefined(ExposureClass)`: a slab without an exposure class has nothing
to meet. The consequent is an `and` of two comparisons:

- the stated cover against `lookup(minimum_cover, exposure)` plus an `if`
  on `coalesce(Prestressed, false)`: an unstated or `null` flag is read as
  false, so the `else` branch adds nothing (a bare `null` condition would
  decide neither branch);
- the measured `extent_z` less twice the stated cover against the authored
  `least_core`.

```json
{{#include ../examples/composing/cover.definitions.json}}
```

```json
{{#include ../examples/composing/cover.ruleset.json}}
```

| Object | Stated and measured | Outcome |
| --- | --- | --- |
| `sheltered` | 30 mm needed, 35 mm stated, 0.25 m thick | passes |
| `exposed` | 40 mm needed, 35 mm stated | finding: `cover meets the class` is false |
| `prestressed` | 30 + 10 mm needed, 35 mm stated | finding: `cover meets the class` is false |
| `thin` | cover enough, 0.16–0.18 m thick: a core of 0.09–0.11 m | not evaluated: straddles 0.1 m |
| `unclassified` | exposure class `null`, 10 mm stated | passes: nothing is required |
| `uncovered` | a class, cover `null` | missing-information finding: `cover meets the class` is `null` |
| `unread` | a class, cover unreadable | not evaluated |

`unclassified` and `uncovered` both lack a value, and the expression
decides each by what the author stated: a missing class lifts the
requirement, a missing cover leaves it unconfirmed, a finding either way.
`unread` lacks no value; the source could not read it, so no verdict is
given.

## Running the examples against a model

The test gives the in-memory project geometry through the typed host
services a geometry adapter provides: the vertical-extent service answers
`extent_z`, the face normals `cross_fall` is measured from and the face
pieces `face_pieces` lists, the object-frame service the placements `own_x` is read along, and
the walking-surface service the treads of each flight. A host checking a
real model registers its adapters' services the same way, and the same
packages run unchanged; a value no service can measure leaves its objects
not evaluated, never passed.
