# Derived properties

Some values a check needs are not stated by any source: a class a ruleset
assigns by its own rules, or a size measured from an object's body. The engine derives them and answers them as
properties of reserved sets, through the same property resolver as stated
properties, so every selector, property reference, table key and category
reads them unchanged. Their names are the engine's or the ruleset's own and
bind to no concept.

## Classifications

A ruleset declares `classifications`, by id: ordered rows, each a selector
and the class name it assigns.

```json
"classifications": {
  "space-use": {
    "id": "space-use",
    "name": {"default": "Space use", "translations": {}},
    "mode": "firstMatch",
    "rows": [
      {"selector": {"kind": "property", "property": "t.Name", "operator": "like",
                    "value": {"type": "string", "value": "Office*"}}, "class": "office"},
      {"selector": {"kind": "property", "property": "t.Name", "operator": "like",
                    "value": {"type": "string", "value": "*Lab*"}}, "class": "lab"}
    ]
  }
}
```

Every object is read as the property `space-use` in the set
`axioval:classification` (`axioval_ir::CLASSIFICATION_SET`):

- `firstMatch` (the default): the class of the first matching row, a string.
  A row decides only once every row before it surely does not match.
- `allMatch`: every matching row's class, distinct, in row order, a list of
  strings; every row must be decided.
- An object no row matches has no value, an exact absence: `exists` is false
  and a keyed table finds no key.
- An object whose deciding row cannot be decided (its property unreadable,
  its concept unbound) cannot be read: every rule reading it leaves it not
  evaluated, never unclassified.

```json
"key_1": {"type": "propertyReference", "propertySet": "axioval:classification",
          "property": "space-use"}
```

The runtime classifies every object before any rule runs, each
classification after those its rows read; its rows are evaluated by the
host's outcome refiner. `axioval:capability.unclassified-object` (parameter
`classification`, the id) reports every selected object a classification
leaves unclassified.

Compilation refuses (`EngineError::InvalidClassification`) a classification
declared under another key than its id, without rows, with a blank class,
with a row reading a rule's outcome (classes are derived before any rule
runs), classifications reading one another in a cycle, a classification
declared by two rulesets compiled together with different rows, and any
classification when the host registered no outcome refiner. A reference to
an undeclared classification is an unknown concept
(`EngineError::UnknownConcept`, kind `axioval:classification`).

## Measured values

The set `axioval:measured` (`axioval_ir::MEASURED_SET`) holds values the
host's geometry services measure, by the engine's own names (matched
ignoring ASCII case):

| Name | Value | Service |
|---|---|---|
| `extent_x`, `extent_y` | the body's extent along the world x or y axis, a length | `VerticalExtentService::measure_directional_extent` |
| `extent_z` | its height, a length | `VerticalExtentService` |
| `bottom`, `top` | the elevation of its lowest and highest point, a length | `VerticalExtentService` |
| `area` | its footprint area, overlaps counted once | `PlanAreaService::measure_footprint` |
| `volume` | its enclosed volume | `ProximityService::measure_body_volume` |

A value measured exactly is a quantity with exact evidence. Anything
coarser (a tessellated body) is a `measured` value, an interval sure to hold
the exact value, whose evidence is never exact:

```json
{"type": "measured", "value": {"lower": 0.045, "upper": 0.055, "dimension": "length"}}
```

A property selector or `property-comparison` compares an interval with its
bound only where every value it may take gives one answer; one that
straddles the bound leaves the object not evaluated
(`incomplete_evidence`). `extent_z < 0.05 m` selects a 40 mm pipe, not a
100 mm one, and leaves a mesh between 45 and 55 mm undecided. A comparison
of two measured values (a door's `bottom` against its space's) takes every
pair the intervals allow. `exists` and `isNotEmpty` hold for any measured
value. Other capabilities reading values (`property-value` literals, sums)
leave an interval not evaluated rather than pick a point in it.

Without the service a value needs, the resolver answers
`PropertyResolutionError::MissingService`, the same for every object; the
runtime reports it once per rule and source (`missing_service`), as it
collapses every object-level missing service. A body the service cannot
measure is unavailable for that object.
