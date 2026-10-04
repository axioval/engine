# Block editors

Block editors such as Blockly define blocks as JSON with a typed output
and typed input checks, and serialise a workspace through a generator of
their own. Axioval gives such an editor a documented mapping, not code:
every block is derived from the [authoring catalogue](./catalogue.md), and
every workspace is stored as an editor-neutral **block tree** that maps
losslessly onto the [expression](./expressions.md) contract. Two editors
implementing the mapping read and write the same trees.

The mapping is `axioval_ir::blocks`: `to_blocks(&Expression)` and
`from_blocks(&Block)`. Both are driven by the catalogue's field tables
(`expressionKinds`, `selectorKinds`, `aggregateSources`), so a new node
kind needs only its catalogue entry.

## The generator emits data, never code

An editor's generator writes the rule JSON (the expression, inside a
ruleset package) or `.mcs` source, and nothing else. It never emits
JavaScript, Rust or any other executable code, and nothing it writes is
ever run: the engine evaluates expressions as data, executes only the
trusted capabilities it was built with, and packages never carry code. A block that cannot be expressed in the contract has no place in the
editor's toolbox.

## Block types

| Block type | Node | Defined by |
| --- | --- | --- |
| `expression.<kind>` | an expression node | `expressionKinds` |
| `measured.<name>` | a read of a measured value (`property` of `axioval:measured`) | `measuredValues` |
| `selector.<kind>` | a selector: an aggregate's `where`, an `over` selector, a nested selector | `selectorKinds` |
| `source.<kind>` | what an aggregate ranges over | `aggregateSources` |
| `members.<name>` | a measured member list an aggregate ranges over (`over: {kind: measured}`) | `measuredMembers` |

## Fields and inputs

Each catalogue field of a node becomes either a *field* of the block (a
setting edited in place) or an *input* (a socket other blocks plug into),
by its `kind`:

| Catalogue field kind | Becomes | Block JSON |
| --- | --- | --- |
| `expression`, `selectorExpression` | an input of one expression block | `{"block": …}` |
| `expressions` | an input of a list of expression blocks (at least `minimum`) | `{"blocks": [ … ]}` |
| `expressionMap` | an input of expression blocks by name (a lookup's keys by column) | `{"map": {"<column>": …}}` |
| `branches` | the `if` branches | `{"branches": [{"when": …, "then": …}]}` |
| `selector` | an input of one selector block | `{"block": …}` |
| `selectors` | an input of a list of selector blocks | `{"blocks": [ … ]}` |
| `aggregateSource` | an input of one `source.` or `members.` block | `{"block": …}` |
| `literal`, `name`, `choice`, `expressionOperator`, `selectorOperator`, `flag`, `path`, `value` | a field, in its JSON form | the value itself |

A `name` field offers the choices its `refers` names (property sets,
parameters, tables, rules, classifications …); a `choice` its `options`;
an operator field the operators of `expressionComparisons` or
`selectorComparisons`; a `path` is the list of relationship steps a
`related` selector writes; a `value` is a parameter value in the package
form (`{"type": "string", "value": "F30"}`); a `literal` a scalar value of
the same form. An expression block's author label is the block's `label`.

### Output and input checks

A block's output check is its node's result type (`resultRule`): a
`fixed` type, the type a package or source `declared` for what it names,
the literal's own type, or the type joined, kept or combined from its
inputs. An input's check is its field's `accepts`, the value types of
`valueTypes`. Numbers carry a dimension: a measured value is answered in
the coherent SI unit of its `dimension` (`m`, `m2`, `m3`, or `rad`, which
is the `angle` type), a quantity literal in the unit it states, and
`add`, `subtract`, comparisons, `min` and `max` take one dimension on both
sides. An editor may narrow an output check as far as it knows the
types; the compiler type-checks the result either way, so an editor's
check is a convenience, never the last word.

Selector blocks have their own output check (a selection, not a value)
and plug only into selector inputs; source and member blocks only into an
aggregate's `over`.

## Truth has three outcomes

A boolean block yields true, false or **not evaluated**, and `null` is a
fourth value it may read, not a third truth. An editor shows a boolean
block's result with all three outcomes and never folds not evaluated into
false: a measurement straddling a bound, a property that could not be
read, or another rule that left the object open leaves the result not
evaluated, and that propagates through `and`, `or`, `not`, comparisons and
arithmetic as [Expressions](./expressions.md#truth) states. `null` (the
source states no value) makes a comparison false and is tested with
`isDefined` and `isUndefined`; it never stands for not evaluated.

## `if`, `else if`, `else` and the ternary

`expression.if` is one block with an `if`/`else if` list and an `else`:
its `branches` input lists the branches in order, the first the `if` and
each further one an `else if`, and its `else` input is required. A
ternary `condition ? a : b` is the one-branch `if`. The editor's mutator
adds and removes `else if` branches; it never drops `else`.

When a branch's condition is not evaluated, the `if` is decided only when
every value it might take agrees; otherwise the whole block is not
evaluated. An editor shows this path explicitly, beside the true and
false paths, rather than as a fall-through to `else`.

## Aggregates

`expression.aggregate` has a `function` field (`aggregateFunctions`: each
states whether it takes a `value` input), an `over` input taking one
source block, an optional `where` input taking a selector block (not with
a `members.` source), and an optional `value` input evaluated with each
member in scope:

- `source.path` ranges over the objects a relationship path reaches; its
  `path` field lists the steps.
- `source.group` ranges over the members of a derived group; its
  `grouping` field names the grouping.
- `source.selector` ranges over the objects its `selector` input
  selects.
- `members.<name>` ranges over the members built-in code measures (a
  flight's steps); its fields are the list's parameters, and the `value`
  reads the member fields of `measuredMembers` as `property` blocks of
  `axioval:member`.

## Measured values

A `property` read of `axioval:measured` is shown as a `measured.<name>`
block, `<name>` a name of `measuredValues`. Its fields are the parameters
the read states, keyed by the parameter's `key`, each the text as written
(`"to": "IfcDoor"`); the descriptor's parameter `kind` (path, source
kind, length with its minimum, choice with its options, vector, property,
text), `required` and `default` say how the editor offers it. A read of
the checked object inside an aggregate (`of: subject`) has the field
`propertyOf: "subject"`; parameter keys are lowercase, so this field
never collides with one. The block's output is a number of the
descriptor's `dimension`. A `members.<name>` block takes its list's
parameters the same way.

The mapping never rewrites a name. A read becomes a `measured.` block only
when its text is canonical: the registry's name, then each stated
parameter as `;key=value` in the registry's parameter order, keys
lowercase, values trimmed (`distance;to=IfcDoor;projection=horizontal`).
Any other spelling (`Area`, `distance;projection=horizontal;to=IfcDoor`)
stays an `expression.property` block holding the text as written, and a
member list a `source.measured` block with its `name` field. Going back,
a `measured.` block writes its parameters in the registry's order and is
checked by the registry's parser, so a missing required parameter, an
unknown key or a value of the wrong kind is refused.

## The block tree

A block is a JSON object:

| Member | What it holds |
| --- | --- |
| `type` | the block type, such as `expression.compare` |
| `label` | an expression node's author label; omitted when none |
| `fields` | the fields by name; omitted when empty |
| `inputs` | the inputs by name; omitted when empty |

Unknown members are refused. A field the node leaves at its default (a
`caseSensitive` that is true, an absent optional name) is omitted from the
canonical tree; a tree that states it explicitly reads the same.

```json
{
  "type": "expression.aggregate",
  "fields": { "function": "all" },
  "inputs": {
    "over": {
      "block": {
        "type": "source.path",
        "fields": { "path": ["IfcRelContainedInSpatialStructure:backward"] }
      }
    },
    "value": {
      "block": {
        "type": "expression.compare",
        "fields": { "operator": "lessThan" },
        "inputs": {
          "left": {
            "block": {
              "type": "measured.distance",
              "label": "distance to the nearest door",
              "fields": { "to": "IfcDoor", "projection": "horizontal" }
            }
          },
          "right": {
            "block": {
              "type": "measured.bottom_above_level",
              "fields": {
                "path": "IfcRelContainedInSpatialStructure:backward",
                "propertyOf": "subject"
              }
            }
          }
        }
      }
    }
  }
}
```

is the expression

```json
{
  "kind": "aggregate",
  "function": "all",
  "over": { "kind": "path", "path": ["IfcRelContainedInSpatialStructure:backward"] },
  "value": {
    "kind": "compare",
    "operator": "lessThan",
    "left": {
      "kind": "property",
      "propertySet": "axioval:measured",
      "property": "distance;to=IfcDoor;projection=horizontal",
      "label": "distance to the nearest door"
    },
    "right": {
      "kind": "property",
      "propertySet": "axioval:measured",
      "property": "bottom_above_level;path=IfcRelContainedInSpatialStructure:backward",
      "of": "subject"
    }
  }
}
```

A malformed tree is refused with the path of the offending block and what
is wrong there: `$` is the root, `.inputs.<name>` an input, `[n]` a list
entry or branch, `.<column>` a lookup key, `.when`/`.then` a branch part
and `.fields.<name>` a field (`$.inputs.operands[1]`: `expression.not`
needs the input `operand`). A node the contract refuses is named by its
block's path with the contract's reason.

## Fixtures

`crates/contracts/ir/tests/blocks/*.json` pairs an expression with its
block tree (`{"expression": …, "blocks": …}`): one per expression node
kind and literal type, and further ones covering every selector kind,
every aggregate source, measured values with parameters, member lists,
and the names kept as written. An editor implementing the mapping should
load each tree, generate the expression from it, and get the paired one;
and the reverse. The tests check both directions for every fixture, and
fail when a catalogued expression kind, selector kind or aggregate source
has none. Regenerate the trees with
`AXIOVAL_BLESS=1 cargo test -p axioval-ir --test blocks`.
