# Expressions

An expression computes and combines values for one object: the contract is
`axioval_ir::contract::Expression` ([Source-neutral IR](./ir.md#expressions)),
and `axioval_engine::expression` checks and evaluates it. The language is
total and pure: no loops, no recursion, no user-defined functions and no
package code, so every expression terminates.

## Text form

An expression also has a text form for people to write, as takeoff columns
and editors take it: `area × 42.5 EUR/m²`,
`if(class == "office" and not open, area * 2, area)`.
`axioval_engine::expression::parse_text` parses it into the tree; it never
runs as code.

```text
expression := or
or         := and ("or" and)*
and        := not ("and" not)*
not        := "not" not | comparison
comparison := sum (("==" | "!=" | "<" | "<=" | ">" | ">=" | "≤" | "≥" | "≠") sum)?
sum        := term (("+" | "-" | "−") term)*
term       := factor (("*" | "×" | "·" | "/" | "÷") factor)*
factor     := ("-" | "−") factor
            | number [unit] | "text" | true | false | null
            | function "(" expression ("," expression)* ")"
            | name | "(" expression ")"
```

A number takes the unit written right after it (`42.5 EUR/m²`, `10 m2`);
a word that is no unit (`3 or …`) is read as what follows the number. A
name reads the parameter (in a takeoff, the column) of that name. The
functions are `min` and `max` (two values or more), `abs`, `sqrt`,
`floor(value[, step])`, `ceil(value[, step])`, `round(value, step)`,
`if(condition, then, else)` and `coalesce(…)`. Properties, aggregates and rule outcomes are written in the
tree. Text is at most `MAX_TEXT_LENGTH` (1000) characters and nests at most
as deeply as the tree.

## Values

A value is `null`, a truth, a number, text, an enumeration value, a date or
a date-time. Numbers are intervals in coherent SI units with their unit
(`m`, `m²`, `rad`, `EUR·m⁻²`; a plain number has unit `1`), exactly as
`axioval:measured` answers them: a point when known exactly, a wider
interval when measured on a tessellated body. A quantity literal is
converted to coherent units when read (`40 mm` is `0.04 m`).

`round(value, step)` with a decimal step (`0.001`, `1e-6`, `0.005`) gives
each multiple as the double nearest the decimal multiple, the same double
its literal is, so a riser measured within a few units in the last place of
0.17 m rounds to exactly the literal `0.17 m`. A comparison that must not
fail on binary rounding compares rounded values, as a standard states its
bounds to a precision.

`floor(value, step)` and `ceil(value, step)` round down or up to a multiple
of `step`, as a norm prescribes the direction: a required number of parking
spaces up (`ceil(4.2, 1)` is 5), a dimension down to whole centimetres
(`floor(0.987 m, 0.01 m)` is `0.98 m`). The step is one positive value of
the operand's unit. Both are monotone, so each bound of an interval goes
outward on its own: `[4.0, 4.2]` rounded up is `[4, 5]`, every value the
operand allows rounded into it. A decimal step stands for its decimal as
in `round`, and a bound that is the literal of a multiple is that multiple
(`floor(0.98 m, 0.01 m)` is `0.98 m`, not `0.97 m`); any other bound counts
its steps over the widened quotient, so a bound within rounding of a
multiple rounds to both neighbours. Without a step, `floor` and `ceil`
round to whole numbers as before.

Arithmetic is sound: every result holds every value its operands allow.
Where a floating-point operation rounds, the bound moves one step outward;
an exact operation on points stays a point. Division by an interval holding
zero, a result that is not finite, a square root of a negative value, a
tangent at a pole and the angle of a vector box around the origin are not
evaluated, each with its own reason.

| Operation | Units |
| --- | --- |
| `add`, `subtract`, `min`, `max`, `round`, `floor` and `ceil` with a step | one unit for all operands |
| `multiply`, `divide` | units multiply or divide (`m × m = m²`, `m ÷ m = 1`) |
| `sqrt` | halves every exponent; `m³` has no square root |
| `sin`, `cos`, `tan` | a plane angle in, a plain number out |
| `atan2` | two operands of one unit, a plane angle out |
| `convertSlope` | `ratio` and `percent` are plain numbers, `angle` a plane angle |
| `inUnit` | the operand's unit, checked against the written unit's dimension |

`inUnit(value, unit)` restates a value in a written unit of its dimension
(`mm`, `cm`, `m2`, `%`, `deg`): the value stays the same, in coherent
units, and is shown in that unit, in explanations and in a finding's
`{label}` (`0.3 m` in `mm` shows `300 mm`, `0.05` in `%` shows `5 %`).
The type checker refuses a unit of another dimension where the operand's
unit is known (`UnitMismatch`, "it restates …") and a unit that does not
parse (`InvalidUnit`); one known only when read is checked when evaluated.
An editor uses it to show a value in a display unit, and to state the unit
of a value only a read could tell, such as an aggregate's members' value.

## Truth

A truth's place holds one of four states: **true**, **false**, **`null`**
(a value it reads is stated absent, so the answer is unknown) and **not
evaluated** with a reason. The logic is Kleene's three-valued logic over
true, false and `null`, with not evaluated kept apart: it is the outcome
that could be any of the other three, so it outranks `null` wherever the
answer is not already decided.

- A comparison with `null` is `null`, never false: `x == 5`, `x != 5`
  and `not(x == 5)` are all `null` for an object that states no `x`.
- `null` in a truth's place (an operand of `and`, `or`, `not`,
  `implies`, `xor`, a condition of `if`) stays `null`: missing
  information, never false and never true.
- A requirement that is `null` is a **missing-information finding**,
  never a pass ([Expression requirements](./capabilities.md#expression-requirements)).
  The only ways past an absent value are explicit: `isDefined`,
  `isUndefined` and `coalesce`, which read `null` as a value.

In the tables, `T` is true, `F` false, `N` `null` and `U` not evaluated;
rows are the left operand (the antecedent of `implies`), columns the
right.

| `and` | T | F | N | U |
| --- | --- | --- | --- | --- |
| **T** | T | F | N | U |
| **F** | F | F | F | F |
| **N** | N | F | N | U |
| **U** | U | F | U | U |

| `or` | T | F | N | U |
| --- | --- | --- | --- | --- |
| **T** | T | T | T | T |
| **F** | T | F | N | U |
| **N** | T | N | N | U |
| **U** | T | U | U | U |

| `implies` | T | F | N | U |
| --- | --- | --- | --- | --- |
| **T** | T | F | N | U |
| **F** | T | T | T | T |
| **N** | T | N | N | U |
| **U** | T | U | U | U |

| `xor` | T | F | N | U |
| --- | --- | --- | --- | --- |
| **T** | F | T | N | U |
| **F** | T | F | N | U |
| **N** | N | N | N | U |
| **U** | U | U | U | U |

`and` and `or` take any number of operands: a decisive operand (false
for `and`, true for `or`) decides whatever the others are; otherwise the
first operand not evaluated decides, then any `null`. `implies(a, b)` is
`or(not a, b)`, and a false antecedent decides without evaluating the
consequent.

| Operand | `not` | `isDefined` | `isUndefined` | `coalesce(operand, v)` |
| --- | --- | --- | --- | --- |
| T | F | T | F | T |
| F | T | T | F | F |
| N | N | F | T | `v` |
| U | U | U | U | U |

Comparisons, `between`, `oneOf` and `noneOf` over a value `x`, `null`
and a value not evaluated (columns are the right operand of `compare`):

| `compare` | `x` | N | U |
| --- | --- | --- | --- |
| **`x`** | T or F | N | U |
| **N** | N | N | U |
| **U** | U | U | U |

- `between(x, low, high)` is `and(x ≥ low, x ≤ high)` (or the strict
  forms): `null` anywhere is `null`, unless the other bound already fails.
- `oneOf(x, values)` is `or` over `x == value`, and `noneOf(x, values)` is
  `not oneOf(x, values)`, so the two always agree: a `null` operand is
  `null` for both, a `null` value among the candidates leaves the answer
  `null` unless another candidate matches.
- `if(c, a, b)` is `a` when `c` is true and `b` when false. When `c` is
  `null` or not evaluated, the `if` is decided only when the branch it
  might take and the rest agree on one value; otherwise it is `null` for a
  `null` condition and not evaluated, naming the condition, for one not
  evaluated.

A comparison of numbers is decided only when every pair of values the
operands allow gives the same answer: `extent_z < 50 mm` holds for a pipe
measured at 39–41 mm, fails for one at 60 mm, and is not evaluated for one
at 45–55 mm. A straddling value is never a pass. Dates compare
chronologically, as XML Schema orders them, and date-times as instants:
`2026-09-28+12:00` equals `2026-09-27-12:00`, while a date stating a time
zone and one stating none within 14 hours of it are unequal and in no
order (an order of them is not evaluated). A date never compares with a
date-time. Text compares as written, or folded when `caseSensitive` is
`false`; `like` and `matches` match the whole value, case folded by the
pattern, never by lowercasing the pattern itself.

Every comparison here is the one every rule decides with
(`axioval_engine::comparison`): property selectors, the generic judges
(`property-predicate`, `property-comparison`, `property-value`) and the
templates' range judge decide through the same module, so a value
compares alike wherever a rule reads it. Only what a comparison with
nothing means differs by where it stands: `null` in an expression, no
match in a selector, a failed predicate.

## `null` is not "not evaluated"

`null` is a value the source states as absent. Not evaluated means a value
could not be read or measured, or a result cannot be decided. They never
mix:

| Expression | With `null` | With a value not evaluated |
| --- | --- | --- |
| any comparison, `between`, `oneOf`, `noneOf` | `null` | not evaluated |
| `isDefined` / `isUndefined` | false / true | not evaluated |
| arithmetic and text functions | `null` | not evaluated |
| `coalesce` | skipped | not evaluated, unless a value came first |
| a truth's place (`and`, `or`, `not`, `implies`, `xor`, `if`) | `null`, unless decided either way | not evaluated, unless decided either way |
| a requirement | a missing-information finding | the object is not evaluated |
| a selector's expression | not selected | the selection is undecided |

Whether an absent value passes or fails is the author's decision, stated
in the expression: `implies(isDefined(x), test)` lifts the requirement
where `x` is `null`, `and(isDefined(x), test)` requires a value, and
`coalesce(flag, false)` reads an unstated flag as false.

## Aggregates

An `aggregate` computes one value over the objects `over` reaches from the object in scope:

- `{"kind": "path", "path": [...]}`: the objects a relationship path reaches, its steps written as a `related` selector's;
- `{"kind": "group", "grouping": "…"}`: the members of the derived group the object is, or belongs to;
- `{"kind": "selector", "selector": {…}}`: every object of the project the selector selects, the counterparts of a pair rule.
- `{"kind": "measured", "name": "…"}`: the members built-in code measures of the object in scope, by a list of the registry `axioval_ir::measured::MEASURED_MEMBERS` written like a measured value (`steps;walking_line_offset=0.3`). The object in scope stays the owner, and `value` reads each member's fields in the reserved set `axioval:member` (`riser`, `going` of a flight's `steps`), typed by the registry; outside such an aggregate the set is refused when the ruleset compiles. It takes no `where`: a condition on the members is part of the `value`. See [Measured members](./derived.md#measured-members).

`where` keeps the members a selector selects, and `value` is evaluated with each member in scope; inside it a property with `"of": "subject"` reads the rule's checked object instead. The functions are `count` (no `value`), `sum`, `min`, `max`, `average`, `any`, `all`, `none` (a truth `value`) and `distinctCount`. "The openings of a wall take at most 40 % of its side":

```json
{"kind": "compare", "operator": "lessThanOrEquals",
 "left": {"kind": "aggregate", "function": "sum",
          "over": {"kind": "path", "path": ["IfcRelVoidsElement:forward"]},
          "value": {"kind": "property", "propertySet": "Pset_OpeningElementCommon", "property": "Area"}},
 "right": {"kind": "multiply",
           "left": {"kind": "property", "propertySet": "Pset_WallCommon", "property": "SideArea"},
           "right": {"kind": "literal", "value": {"type": "number", "value": 0.4}}}}
```

A member whose membership cannot be decided (its `where` or `over` selector undecided) is never dropped. `count` becomes an integer interval (2..3 for two members and one undecided), `sum` adds its value or nothing, `min` and `max` may reach it, and `distinctCount` may count it; `any`, `all` and `none` hold or fail only when every way the undecided members may belong agrees (`all` over one undecided false member is false either way), and are otherwise not evaluated; `average` is not evaluated. A member value that is not evaluated leaves the aggregate not evaluated, unless the answer is decided either way, and a path or group the run cannot list leaves the object in scope not evaluated. The evaluation cites every member's evidence.

A member value that is `null` is never skipped and never false. `any` is `or` over the members' values, `all` is `and` over them and `none` is `not any`, with Kleene's tables above, so a `null` member leaves them `null` unless another member decides (`any` with a true member, `all` and `none` with a false one, a true one for `none`). `sum`, `min`, `max`, `average` and `distinctCount` are `null` as soon as a certain member's value is; a `null` value of a member whose membership is undecided leaves them not evaluated. `count` counts members, whatever their values. To leave out the members that state nothing, the author says so: a `where`, or a `coalesce` or guard in the `value` (`none(and(isDefined(gap), gap > 0.15 m))`).

| Members | `any` | `all` | `none` | `sum` | `min`, `max`, `average` | `count` |
| --- | --- | --- | --- | --- | --- | --- |
| none | F | F | T | 0 in the members' unit | N | 0 |
| T | T | T | F | | | 1 |
| F | F | F | T | | | 1 |
| N | N | N | N | N | N | 1 |
| U | U | U | U | U | U | 1 |
| F and N | N | F | N | | | 2 |
| T and N | T | N | F | | | 2 |
| N and U | U | U | U | N | N | 2 |
| 1 and 2 | | | | 3 | 1, 2, 1.5 | 2 |
| 1 and N | | | | N | N | 2 |

`all` over no member is false: it never holds vacuously, as a `related` selector's `all`.

`sum` over no member is 0 in the unit its members' value is typed in, as the
type checker types it: a measured value or member field, a property of a
declared kind and dimension, a derived value, a literal, an `inUnit`, and
arithmetic over them. So `sum(openings' area) ≤ 3 m²` holds with 0 m² for
a wall without openings, never a unit mismatch. Only a value whose unit no
declaration tells (a property declared without a dimension, a parameter)
sums to a plain 0 over no member; `inUnit` states its unit.

## The least and the greatest member

`argmin` and `argmax` choose the member whose `by` is least (greatest)
and read its `value`, so a rule refers to the decisive member: the
longest escape route of a storey, the steepest roof face, the widest
opening. They range over members as an aggregate does (`over`, `where`),
and `by` (a number) and `value` (any value) are evaluated with each member
in scope. "The steepest face of a roof is covered with slate":

```json
{"kind": "compare", "operator": "equals",
 "left": {"kind": "argmax",
          "over": {"kind": "path", "path": ["IfcRelAggregates:forward"]},
          "by": {"kind": "property", "propertySet": "axioval:measured", "property": "slope"},
          "value": {"kind": "property", "propertySet": "Pset_RoofCommon", "property": "Covering"}},
 "right": {"kind": "literal", "value": {"type": "string", "value": "slate"}}}
```

Where intervals overlap, every member that may be the extreme is chosen:
a member may be the greatest when the upper bound of its `by` reaches the
greatest lower bound among the members surely belonging (`argmin`: its
lower bound reaches their least upper bound). Faces of slopes 20°, 35°
and [34°, 36°] choose the second and the third. The value is the one the
chosen members agree on: equal values, or numbers of one unit, whose
interval then holds each (35° and [34°, 36°] read as their slopes give
[34°, 36°]). Chosen members whose values differ (texts, truths, `null`
beside a value) leave the value not evaluated, never one of them picked.
A finding the requirement reports names the chosen members (`argmax
chose …` among the values read) and relates them, so a viewer shows the
faces, not only the roof.

- A member whose membership is undecided may be chosen, but bounds no
  other, since it may not belong. With no member surely belonging there
  may be none, which is not evaluated; no member at all is `null`.
- A member whose `by` is not evaluated may be the extreme whatever the
  others are: it is chosen and bounds no other, so the value stands only
  where every chosen member's value agrees, and otherwise is not
  evaluated for that member's reason.
- A member surely belonging whose `by` is `null` makes the value `null`,
  as it makes `max` `null`; an undecided member's `null` leaves it not
  evaluated. `null` and not evaluated never mix.
- A chosen member's `value` not evaluated leaves the value not evaluated;
  the value of a member that cannot be chosen is never read into it, nor
  cited.

Every member's membership and `by` evidence is cited, since it decides
which are chosen, and the chosen members' `value` evidence. A measured
member list names its members by their words (a text field), else their
place (`member 2 of 3`), and relates the objects its fields name.

## Consecutive members

`pairs` relates each member of an ordered sequence to the one before it:
consecutive risers, the plan radii one after another along a road, the
heights of storeys stacked by elevation. It ranges over members as an
aggregate does (`over`, `where`), orders them by `by` ascending (a number:
a station, an elevation, a sequence number), and applies its `function`
(any aggregate function) to `value` evaluated once per consecutive pair.
The later member of the pair is in scope; `previous` reads its `operand`
of the earlier one, and `next` of the later one, so a pair can be written
either way. "Consecutive risers differ by at most 5 mm":

```json
{"kind": "pairs", "function": "all",
 "over": {"kind": "path", "path": ["IfcRelAggregates:forward"]},
 "by": {"kind": "property", "propertySet": "ex:pset-stair-flight-common", "property": "ex:riser-sequence"},
 "value": {"kind": "compare", "operator": "lessThanOrEquals",
           "left": {"kind": "abs", "operand": {"kind": "subtract",
             "left": {"kind": "property", "propertySet": "ex:pset-stair-flight-common", "property": "ex:riser-height"},
             "right": {"kind": "previous", "operand":
               {"kind": "property", "propertySet": "ex:pset-stair-flight-common", "property": "ex:riser-height"}}}},
           "right": {"kind": "literal", "value": {"type": "quantity", "value": 5, "unit": "mm"}}}}
```

Risers of 172, 175 and 181 mm fail it at the pair 175 → 181. A finding
the requirement reports names the pairs that decide it (`pairs r2 → r3`
among the values read) and relates their members: the pairs failing an
`all`, holding an `any`, holding a `none` that fails, or whose value may
be the least or greatest of a `min` or `max`. `previous` and `next` stand
only within the `value` of a `pairs`, and not within a member scope
nested there (`Expression::validate` refuses them elsewhere). `count`
takes no `value` and counts one pair fewer than the members.

An order is never guessed. The members are listed by the lower bound of
their `by`, then its upper bound; two neighbours in that listing are a
*decided* pair when every order the intervals allow keeps them
neighbours: the earlier's upper bound lies below the later's lower bound,
and every other member surely lies before, after or between them. Equal
points are no order. A pair that is not decided is not evaluated (the
order of the two members is undecided), never ordered by a guess and
never a pass: an `all` it touches fails where a decided pair fails and is
otherwise not evaluated, and a sum, least or greatest over it is not
evaluated. Risers whose levels are [0.17, 0.18] m and [0.175, 0.185] m
may come either way round, so a flight holding them stays open.

- A member whose membership is undecided may be absent, and the pair
  bridging it is then consecutive: every pair some choice of present
  members makes consecutive is listed, possible unless both members surely
  belong and none lies between them, and the function widens or leaves
  the result open as an aggregate's does. Every possible pair holding, an
  `all` over two or more members surely belonging holds.
- A member whose `by` is not evaluated may lie anywhere, so no pair is
  decided: the result is not evaluated, for that member's reason.
- A member surely belonging whose `by` is `null` (the source states no
  position) makes the result `null`; an undecided member's `null` leaves
  it not evaluated. `null` and not evaluated never mix.

## Other rules' outcomes

Composite checks read how other rules of the same ruleset judged the object in scope:

| Node | Value |
| --- | --- |
| `{"kind": "ruleOutcome", "rule": "fire"}` | true when the rule passed the object, false when it reported a finding about it, `null` when it did not select it |
| `{"kind": "selected", "rule": "fire"}` | true when the rule selected the object (whatever it then concluded), false when it surely did not |
| `{"kind": "findingCount", "rule": "fire"}` | how many findings the rule reported about the object, 0 when it passed or did not select it |
| `{"kind": "deviation", "rule": "slope"}` | the greatest graded deviation of its findings about the object (how far a value misses its bound, relative to it), a plain number interval; `null` when none is graded |

An object the other rule left open, or whose selection it could not decide, leaves each of them not evaluated (`selected` only when the selection itself is undecided), so "fails if either the fire rule or the escape rule fails, unless the object is temporary" is `or(Temporary, and(ruleOutcome(fire), ruleOutcome(escape)))` with Kleene logic throughout.

`ruleOutcome` of an object the other rule did not select is `null`, and `null` decides nothing: `implies(ruleOutcome(fire), …)` over such an object is a missing-information finding, never a vacuous pass. Where the other rule's selection matters, the expression states it with `selected`: `implies(selected(fire), ruleOutcome(fire))` passes the objects the fire rule did not select, `and(selected(fire), ruleOutcome(fire))` fails them. The plan runs every rule a rule's expressions read first, as for `ruleOutcome` selectors (in its requirement, its computed parameters, its expression selectors and their aggregate filters); a cycle, a rule reading itself or a rule the ruleset does not define fails compilation, and combined rulesets rename the rules read as they rename the rules. A derived value reads no rule outcome: values are derived before any rule runs.

## Tables

A rule's table parameters are read three ways, `lookup`, `interpolate`
and `lookup2`. `lookup` selects the most
specific row whose key cells match its keys, as `keyed-limit` does
([Expression requirements](./capabilities.md#expression-requirements)).
`interpolate` reads a value between a table's rows, as norm tables state
values at grid points and say intermediate values are interpolated
linearly (a clear width by occupant count, a cover by span):

```json
{"kind": "interpolate", "table": "cover_by_span", "keyColumn": "span",
 "key": {"kind": "property", "propertySet": "Pset_SlabCommon", "property": "Span"},
 "column": "cover"}
```

- The key is read in `keyColumn`, whose cells are numbers of the key's
  unit, one per row; between the two rows whose keys bracket it, the
  `column` cell is interpolated linearly. Rows 2.0 → 10 and 3.0 → 20 give
  14 at 2.4 (within rounding), and a key on a row gives its cell exactly.
- A key interval gives every value its keys allow: linear interpolation
  is monotone between two rows, so the result is the hull of the values at
  the interval's ends and at every row key within. `[2.4, 2.6]` gives
  `[14, 16]`, and an interval across a peak row holds the peak.
- A key beyond the first or last row is not evaluated ("`span` 5 m lies
  beyond its last row's, 4 m, …"), never extrapolated. `"clamp": true`
  declares that the nearest end row's value applies there instead.
- A key of `null` is `null`. Not evaluated, each with its reason: a
  bracketing row stating no `column` cell (naming its key), two rows at
  one key, a row stating no key, and a key or cell of another unit (a
  mismatch). The type checker refuses a key column that holds no numbers,
  a key of another unit than its cells and a `column` that is not numeric;
  whole-number cells interpolate to a number.

`lookup2` reads a two-key table, such as a minimum value by exposure class
and component thickness or a gradient by speed and terrain category. The
table states one cell per row: a row key, a column key and the value.

```json
{"kind": "lookup2", "table": "cover_by_class_and_span",
 "rowKeyColumn": "class", "rowKey": {"kind": "property", "propertySet": "Pset_SlabCommon", "property": "ExposureClass"},
 "columnKeyColumn": "span", "columnKey": {"kind": "property", "propertySet": "Pset_SlabCommon", "property": "Span"},
 "interpolateColumn": true, "column": "cover"}
```

- Each key is matched exactly (text, an enumeration value, a truth or a
  number equal to a row's), or with `interpolateRow` or
  `interpolateColumn` interpolated linearly as `interpolate` does, between
  the bracketing keys of its numeric key column. Interpolated in both, the
  value is bilinear in the four surrounding cells.
- The grid lines of an interpolated key are every key its column states,
  so the grid is whole: a cell no row states is not evaluated, and the
  reason names its keys ("table `cover_by_class_and_span` has no row for
  `class` = `XC4` and `span` = 4 m"), whether it is read or needed to
  interpolate. A cell is never borrowed from a farther row.
- Interval keys give every value they allow: bilinear interpolation is
  linear in each key, so the result is the hull of the values at the
  corners the key intervals and the grid lines within them make. A key
  beyond an interpolated column's keys, a `null` key and repeated cells
  behave as for `interpolate`, and `clamp` applies to both keys. A key
  matched exactly that is an interval holding a row's key is not
  evaluated: it may or may not be that row's.

## Catalogues

Many requirements need material or product data that models leave out or
name only by reference: a material's thermal conductivity λ, its density
or building material class, a product's sound insulation. A ruleset
declares such a catalogue's shape under `catalogues`, by id, and the host
supplies its entries at check time, beside the model (`--catalogue` in
[the CLI](./cli.md#catalogues-supplied-beside-the-model),
`ExecutionPlan::supply_catalogue` in a host), so one rule package serves
every catalogue edition and the data is never code:

```json
"catalogues": {
  "materials": {
    "id": "materials",
    "name": {"default": "Materials", "translations": {}},
    "key": "name",
    "columns": [
      {"id": "name", "header": "material", "kind": "string"},
      {"id": "lambda", "kind": "quantity", "unit": "W/m·K"},
      {"id": "density", "kind": "quantity", "unit": "kg/m³"}
    ]
  }
}
```

`columns` declares every column of the file as a
[table file](./capabilities.md#tables-from-data-files) does: its `id`, its
`header` (the id when omitted), its `kind` and a quantity's `unit`. `key`
names the `string` column whose cells name the entries: a material name, a
classification code or a product id. The compiler refuses a key column that
is not declared or not text, a column that states no value (`reference`,
`textPattern`, `selector`), a unit it cannot read, an id holding `:`, `;`,
`|`, `/`, `#`, `=` or whitespace, and two rulesets declaring one id
differently (`EngineError::InvalidCatalogue`). The entries the host supplies
are checked against the declaration: every cell of its column's kind, every
entry naming a non-blank key, no key twice.

A `catalogue` node reads one entry's value:

```json
{"kind": "catalogue", "catalogue": "materials", "column": "lambda",
 "key": {"kind": "property", "propertySet": "axioval:material", "property": "Name"}}
```

- The entry is the one whose key equals the text `key` exactly; case and
  spaces count. An author who wants another match states it in `key`
  (`trim`, `lower`), never the engine by a guess.
- Its value is the catalogue's statement, typed by the column's
  declaration (`lambda` above is a quantity of W/(m·K), so comparing it
  with a length fails compilation) and composable like any other: `d / λ`
  is a thermal resistance, and a derived value may read a catalogue too.
- It is never the model's: its evidence cites
  `axioval:catalogue;id=<id>;version=<version>;sha256=<digest>#entry=<key>`
  (the version the host stated, the SHA-256 of the file), and a finding
  lists it as `catalogue materials[ex-clay-brick].lambda = …` beside the
  model values it read.
- An empty cell is a value the catalogue states as absent: `null`, never
  not evaluated. So is a `null` key, where the model states no material:
  no entry is named.
- A key no entry has is not evaluated, the reason naming the catalogue,
  its version and the key ("catalogue `materials` (version 2026-1) has no
  entry whose `name` is `ex-mystery-board`"); so is every read of a
  catalogue the host supplied no entries for. Neither is ever given a
  default.
- The type checker refuses a catalogue the ruleset does not declare, a
  column it does not declare or its key column read as a value, and a key
  that is not text.

### Values stated per range

Some data is stated per range of a position rather than per key: the
design speed or the road category of an alignment from one station to
another. A catalogue declares such ranges with `ranges`, naming its two
columns stating where each entry's range starts and ends:

```json
"design_speeds": {
  "id": "design_speeds",
  "name": {"default": "Design speeds", "translations": {}},
  "key": "alignment",
  "columns": [
    {"id": "alignment", "kind": "string"},
    {"id": "from", "header": "from station", "kind": "quantity", "unit": "m"},
    {"id": "to", "header": "to station", "kind": "quantity", "unit": "m"},
    {"id": "speed", "header": "design speed", "kind": "quantity", "unit": "km/h"}
  ],
  "ranges": {"from": "from", "to": "to"}
}
```

A key then names an entry per range, and a `catalogue` node reads one
`at` a position, such as an object's station along the alignment
([measured](./derived.md) `station;alignment=<kinds>`):

```json
{"kind": "catalogue", "catalogue": "design_speeds", "column": "speed",
 "key": {"kind": "literal", "value": {"type": "string", "value": "ex-axis-1"}},
 "at": {"kind": "property", "propertySet": "axioval:measured",
        "property": "station;alignment=IfcAlignment"}}
```

- `from` and `to` are two declared `number` or `quantity` columns of one
  unit, neither the key. Each entry states both, its start below its end;
  a range holds its start and not its end (`[from, to)`), and two ranges
  of one key that overlap refuse the file, naming both.
- `at` is a number of the ranges' unit (a length for stations; the type
  checker refuses another unit, and a ranged catalogue read without `at`
  or another read with it). A `null` position names no range: `null`.
- The value is that of the range holding the position, cited
  `…#entry=<key>;range=<from>..<to>`, and listed as
  `catalogue design_speeds[ex-axis-1 at 1500 m].speed = …`. A measured
  station is an interval: one reaching several ranges reads their value
  where they state one alike (citing each), and is not evaluated where
  they state different values, never a range picked. A position no range
  holds (before the first, in a gap, at or beyond the last one's end) is
  not evaluated, naming the key's ranges; never the nearest range's value.

Data stated per area rather than per key, a zone map of snow or wind load
zones, is read by the `zone` node from [context
zones](./context-zones.md#the-zone-expression):
`{"kind": "zone", "standsIn": "snow-load-zone", "value": "load_zone"}` is
what the zone the object stands in states under `load_zone`, a value of
any type, usable as a `lookup` key.

### Results of external calculations

Many requirements are calculation methods rather than checks on the
model: sound insulation, the energy balance, structural design, daylight
simulation, performance-based fire engineering. Axioval does not
reimplement them; a check reads their results, computed by an external
tool, as values that calculation states: the R'w of a partition, a
building's primary energy demand, a member's utilisation ratio.

Such results are a catalogue whose declaration adds `results`, naming the
columns that trace each entry to the calculation that computed it:

```json
"catalogues": {
  "sound": {
    "id": "sound",
    "name": {"default": "Sound insulation results", "translations": {}},
    "key": "wall",
    "columns": [
      {"id": "wall", "header": "GlobalId", "kind": "string"},
      {"id": "rw", "header": "R'w (dB)", "kind": "number"},
      {"id": "method", "kind": "string"},
      {"id": "report", "kind": "string"},
      {"id": "date", "kind": "date"},
      {"id": "revision", "header": "model revision", "kind": "string"}
    ],
    "results": {"method": "method", "report": "report", "date": "date",
                "revision": "revision"}
  }
}
```

A results catalogue is a catalogue, not a second kind of input: the host
supplies it the same way, its values are typed, read and cited the same
way, and an entry is keyed by whatever stable key the calculation used. A
result attached to a model object is keyed by the object's identity, read
by the expression like any key (an IFC object's `GlobalId` is its
attribute, `{"kind": "property", "propertySet": "axioval:attributes",
"property": <the GlobalId property>}`); a result of the whole project, such
as the primary energy demand, by a key the expression states as a literal.

- `method`, `report` and `revision` name declared `string` columns,
  `date` a `date` or `dateTime` column, none of them the key and no column
  twice (`InvalidCatalogue` otherwise).
- An entry is read only where it states all four: one stating no method,
  report, date or revision is *untraceable*, and reading it is not
  evaluated ("result `W1` of catalogue `sound` states no report (`report`),
  so it cannot be traced to the calculation that computed it").
- The checked model's revision is the SHA-256 of its file as given,
  written `sha256:<hex>`, and any revision the host states for it
  (`ExecutionPlan::supply_model_revision` with a `ModelRevision`; the
  CLI's `--model-revision`). An entry is read for an object only where its
  `revision` cell names the revision of the model the object belongs to.
  One computed on another revision is *stale*: reading it is not
  evaluated, the reason naming both revisions ("result `W4` of catalogue
  `sound` is stale: it was computed on model revision `rev-11`, and the
  checked model … is sha256:…"); so is every read where the host states no
  revision for the model. A result of another model is never read as the
  checked one's, and a stale result is never compared at all, so it can
  neither pass nor fail a requirement.
- `ExecutionPlan::stale_results` lists every supplied result no checked
  model's revision matches, and every untraceable one, before anything
  runs; the CLI reports each as an integrity warning
  (`calculation-result-stale`, `calculation-result-untraceable`).
- A value read is cited with its provenance after the entry:
  `axioval:catalogue;id=sound;sha256=<digest>#entry=<key>;method=<method>;report=<report>;date=<date>;revision=<revision>`,
  each field with `%`, `;`, `#`, `=` and control characters
  percent-encoded.

## Paths

Every not-evaluated outcome and type error names its subexpression by a
path from the place the expression sits: the node kind, then the field or
operand index. `requirement.and[2].compare.left` is the left side of the
comparison that is the third operand of the requirement's `and`. A node's
author `label` is shown with its path. The evaluation also lists every leaf
it read (property, parameter, derived value, table cell) with its path and
evidence, for findings to cite, and traces every subexpression it evaluated
with its value: an expression rule's findings and not-evaluated outcomes
carry the trace as their `explanation`, its deciding path marked (see
[Explanations](./ir.md#explanations)).

## Limits

Expressions come from packages and editors, so their size and their cost
are bounded:

| Limit | Value | Where |
| --- | --- | --- |
| nesting depth | `MAX_EXPRESSION_DEPTH` (64) | `Expression::validate`, the text parser |
| nodes, aggregate filters included | `MAX_EXPRESSION_NODES` (2048) | `Expression::validate` |
| aggregates, `argmin`, `argmax` and `pairs` within one another's `value`, `by` or `where` | `MAX_AGGREGATE_NESTING` (2) | `Expression::validate` |
| text | `MAX_TEXT_LENGTH` (1000 characters) | the text parser |
| nodes evaluated per run, every rule, value and aggregate member together | `DEFAULT_EVALUATION_BUDGET` (100 000 000) | `Runtime::with_evaluation_budget` |

The first four refuse the expression when the ruleset is compiled (a rule's
`InvalidExpression`, a value's `InvalidValue`), naming the limit. Once a
run has spent its evaluation budget, every further expression leaves its
object not evaluated (`resource_limit`, "the run's expression evaluation
budget is spent"), never passed: an expensive ruleset degrades to open
outcomes, not to wrong ones.

The evaluator, the type checker and the text parser are fuzzed in the test
suite with seeded random inputs (`fuzzing_*` in
`crates/engine/core/tests/expressions.rs`, a few thousand trees and texts
per run, so CI runs them in a bounded time): no input panics, and no
interval comes out reversed or infinite; the interval soundness test checks
that random points always land inside the computed interval.

## Types and units

`expression::check` infers every node's type before evaluation: a truth,
an integer, a number of a unit, text, an enumeration value (of its declared
values where known), a date or a date-time. A measured value's type comes
from its descriptor in the registry ([Measured values](./derived.md#the-registry));
a property whose type is known only when read is checked when evaluated,
where a mismatch is not evaluated. `+`, `−`, comparisons, `min`, `max`,
`round`, `floor` and `ceil` with a step and the branches of `if` and
`coalesce` need one unit, and a
quantity never meets a plain number. `check_as` also requires a type: a
requirement must be a truth.

A type error names its kind and path:

| Kind | Example |
| --- | --- |
| `Unknown` | a property, parameter, table or measured name that does not exist |
| `NotBoolean`, `NotNumeric`, `NotText`, `NotAngle` | `not` of a number, `sin` of a length |
| `UnitMismatch` | a length compared with a plain number |
| `Mismatch` | a truth compared with a date |
| `NoSquareRoot` | `sqrt` of a volume |
| `InvalidUnit`, `InvalidPattern` | a literal `parsec`, a `matches` pattern that does not compile |
| `Expected` | a requirement that is a quantity |

`quantity-takeoff` computed columns are expressions: they parse, check and
evaluate through this module, with the same units and intervals.
