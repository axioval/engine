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
leaves unclassified. `axioval:capability.quantity-takeoff` groups
quantities by the class ([Information takeoff](./capabilities.md#information-takeoff)).

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
| `x`, `y`, `z` | the world coordinates of its placement origin, a length, always exact | `ObjectFrameService` |
| `bottom_above_level;path=<steps>` | its bottom above the one level the path reaches | `RelationshipSelectionService`, `ObjectFrameService`, `VerticalExtentService` |
| `boundary_area;kind=<kind>[;plane=<m>]` | a space's summed boundary area against elements of one kind | `BoundaryCoverageService`, `TypeHierarchyService` |
| `level_height` | a storey's height to the next storey, a length | the source's property resolver |

### Names with parameters

Two names take `;`-separated `key=value` parameters, part of the property
name so that every selector, property reference and table key can carry
them unchanged. Compilation checks their syntax; a malformed name is an
unknown concept.

- `bottom_above_level;path=<steps>`: the containment path is the rule's to
  state, never a host default, because only the package knows which
  relationship places its objects on a level (an IFC element in its storey
  is `IfcRelContainedInSpatialStructure:backward`, a space
  `IfcRelAggregates:backward`). Steps are `,`-separated and written as a
  `related` selector's steps (`Relationship[:forward|backward|either][+]`),
  walked one after another over the whole project. The level's elevation
  is its placement origin's `z`. No level reached is an exact absence (a
  loose object has no storey), several levels at different elevations are
  a conflict, and a path the relationship service cannot answer leaves the
  object not evaluated. The bottom keeps its interval.
- `boundary_area;kind=<kind>[;plane=<metres>]`: the area of every declared
  space boundary on the space body's face planes (within `plane`, default
  0) whose bounding element is of the source kind `kind` or a subtype
  (`IfcWall` includes `IfcWallStandardCase`, through the source's type
  hierarchy). The kind is the source's own vocabulary, as a host's location
  policy names storeys. A boundary naming no element may be of the kind,
  so it leaves the space not evaluated, and so does a subtype question
  without a type-hierarchy service.

### Stated rather than measured

`level_height` is a storey's height to the next storey of the same spatial
parent, which the IFC adapter states: storey elevations are the world
heights of their placements (in metres through the project's length unit),
siblings are the storeys the same `IfcRelAggregates` parent aggregates, and
the highest storey has none (an exact absence). A tilted storey, a sibling
at the same elevation or unreadable, and a storey aggregated twice are
refused. The engine forwards the name to the host's property resolver.

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
