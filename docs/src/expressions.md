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
functions are `min` and `max` (two values or more), `abs`, `sqrt`, `floor`,
`ceil`, `round(value, step)`, `if(condition, then, else)` and
`coalesce(…)`. Properties, aggregates and rule outcomes are written in the
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

Arithmetic is sound: every result holds every value its operands allow.
Where a floating-point operation rounds, the bound moves one step outward;
an exact operation on points stays a point. Division by an interval holding
zero, a result that is not finite, a square root of a negative value, a
tangent at a pole and the angle of a vector box around the origin are not
evaluated, each with its own reason.

| Operation | Units |
| --- | --- |
| `add`, `subtract`, `min`, `max`, `round` | one unit for all operands |
| `multiply`, `divide` | units multiply or divide (`m × m = m²`, `m ÷ m = 1`) |
| `sqrt` | halves every exponent; `m³` has no square root |
| `sin`, `cos`, `tan` | a plane angle in, a plain number out |
| `atan2` | two operands of one unit, a plane angle out |
| `convertSlope` | `ratio` and `percent` are plain numbers, `angle` a plane angle |

## Truth

Truth is three-valued: true, false, or **not evaluated** with a reason.
`and` is false when any operand is false and `or` true when any is true,
whatever the others are; otherwise the first operand that is not evaluated
decides. `not` of an undecided truth is undecided.

A comparison of numbers is decided only when every pair of values the
operands allow gives the same answer: `extent_z < 50 mm` holds for a pipe
measured at 39–41 mm, fails for one at 60 mm, and is not evaluated for one
at 45–55 mm. A straddling value is never a pass. Dates compare
chronologically and date-times as instants; a date never compares with a
date-time. Text compares as written, or folded when `caseSensitive` is
`false`.

An `if` whose condition is not evaluated is decided only when the branch it
might take and the rest agree on one value; otherwise it is not evaluated,
and the reason names the condition.

## `null` is not "not evaluated"

`null` is a value the source states as absent. Not evaluated means a value
could not be read or measured, or a result cannot be decided. They never
mix:

| Expression | With `null` | With a value not evaluated |
| --- | --- | --- |
| any comparison, `between`, `oneOf`, `noneOf` | false | not evaluated |
| `isDefined` / `isUndefined` | false / true | not evaluated |
| arithmetic and text functions | `null` | not evaluated |
| `coalesce` | skipped | not evaluated, unless a value came first |
| a truth position (`and`, `or`, `not`, `if`) | false | not evaluated |

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

A member whose membership cannot be decided (its `where` or `over` selector undecided) is never dropped. `count` becomes an integer interval (2..3 for two members and one undecided), `sum` adds its value or nothing, `min` and `max` may reach it, and `distinctCount` may count it; `any`, `all` and `none` hold or fail only when every way the undecided members may belong agrees (`all` over one undecided false member is false either way), and are otherwise not evaluated; `average` is not evaluated. A member value that is not evaluated leaves the aggregate not evaluated, and a path or group the run cannot list leaves the object in scope not evaluated. `null` member values are skipped by the numeric functions and are not true for the truth functions. `sum` over no member is 0, `min`, `max` and `average` over none are `null`, `all` over none is false (it never holds vacuously, as a `related` selector's `all`) and `none` over none true. The evaluation cites every member's evidence.

## Other rules' outcomes

Composite checks read how other rules of the same ruleset judged the object in scope:

| Node | Value |
| --- | --- |
| `{"kind": "ruleOutcome", "rule": "fire"}` | true when the rule passed the object, false when it reported a finding about it, `null` when it did not select it |
| `{"kind": "findingCount", "rule": "fire"}` | how many findings the rule reported about the object, 0 when it passed or did not select it |
| `{"kind": "deviation", "rule": "slope"}` | the greatest graded deviation of its findings about the object (how far a value misses its bound, relative to it), a plain number interval; `null` when none is graded |

An object the other rule left open, or whose selection it could not decide, leaves each of them not evaluated, so "fails if either the fire rule or the escape rule fails, unless the object is temporary" is `or(Temporary, and(ruleOutcome(fire), ruleOutcome(escape)))` with Kleene logic throughout. The plan runs every rule a rule's expressions read first, as for `ruleOutcome` selectors (in its requirement, its computed parameters, its expression selectors and their aggregate filters); a cycle, a rule reading itself or a rule the ruleset does not define fails compilation, and combined rulesets rename the rules read as they rename the rules. A derived value reads no rule outcome: values are derived before any rule runs.

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
| aggregates within one another's `value` or `where` | `MAX_AGGREGATE_NESTING` (2) | `Expression::validate` |
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
`round` and the branches of `if` and `coalesce` need one unit, and a
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
