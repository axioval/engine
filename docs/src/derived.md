# Derived properties

Some values a check needs are not stated by any source: a class a ruleset
assigns by its own rules, a group its members imply, or a size measured
from an object's body. The engine derives them and answers them as
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

### Class trees

A classification may declare `classes`: a tree of classes, each with an
`id`, an optional `code`, a localized `name` and an optional `parent`. A
class without a parent is a root, at level 1; every other class is one
level below its parent. Every row then assigns a declared class id, a
leaf or an inner class, as far as its selector can tell. Without
`classes` a classification is flat, its classes the names its rows
assign, and it behaves and serializes exactly as before.

```json
"cost-group": {
  "id": "cost-group",
  "name": {"default": "Cost group", "translations": {}},
  "classes": [
    {"id": "kg-300", "code": "300", "name": {"default": "Building construction", "translations": {}}},
    {"id": "kg-330", "code": "330", "name": {"default": "External walls", "translations": {}}, "parent": "kg-300"},
    {"id": "kg-331", "code": "331", "name": {"default": "Load-bearing external walls", "translations": {}}, "parent": "kg-330"}
  ],
  "rows": [
    {"selector": {"kind": "property", "property": "t.LoadBearing", "operator": "equals",
                  "value": {"type": "boolean", "value": true}}, "class": "kg-331"},
    {"selector": {"kind": "entityType", "objectType": "t.Wall"}, "class": "kg-330"}
  ]
}
```

A hierarchical classification reads in `axioval:classification` as:

- `cost-group`: the class a row assigned, as for a flat one (`kg-331`).
- `cost-group;level=<n>`: the class at level `n` on the way from the
  assigned class to its root: the class itself at its own level, an
  ancestor above it (`kg-300` at level 1). An object whose class lies above
  the level (an inner class at level 2 read at level 3) has no class there,
  an exact absence, as an unclassified object has none. An all-match
  classification lists the distinct classes at that level, in row order.

`n` is a positive integer written without leading zeros, at most the
tree's depth; any other parameter, and a level on a flat classification,
is an unknown concept. The value is the class id, so a takeoff grouped by
`cost-group;level=1` has one group per root.

A `derivedClass` selector selects the objects a classification assigns a
class, or with `includeDescendants` that class or any below it, as the
`classification` selector's `includeDescendants` does for a source's
classification codes:

```json
{"kind": "derivedClass", "classification": "cost-group", "class": "kg-330",
 "includeDescendants": true}
```

An all-match classification's object is selected when any class it
assigns is. An unclassified object is not selected, and one whose class
cannot be derived is not evaluated. The class must be declared (for a
flat classification, assigned by a row, and it has no descendants), or
compilation refuses it as an unknown concept (kind
`axioval:classification class`, concept `<classification>/<class>`). The
selector reads its classification, so a classification's rows may use it
on another classification, never in a cycle.

### Deriving and compiling

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
declared by two rulesets compiled together with different rows or classes,
declared classes with a blank or repeated id or code, a parent that is not
declared, parents forming a cycle, a row assigning an undeclared class
(all checked by `axioval_ir::contract::ClassTree::of`), and any
classification when the host registered no outcome refiner. A reference to
an undeclared classification is an unknown concept
(`EngineError::UnknownConcept`, kind `axioval:classification`).

## Derived groups

Groups such as flats, departments or zones are often implied by a value
every member states (a flat number on every room) rather than stated as
groups. A ruleset declares `groupings`, by id: the objects grouped
(`members`, a selector) and what they are grouped `by`.

```json
"groupings": {
  "flats": {
    "id": "flats",
    "name": {"default": "Flats", "translations": {}},
    "members": {"kind": "entityType", "objectType": "t.Space"},
    "by": {"kind": "property", "propertySet": "t.Pset", "property": "t.FlatNumber"}
  }
}
```

- `{"kind": "property", "propertySet": …, "property": …}` groups by equal
  values of one property, read as a property selector reads it: text as
  stated (case and spaces count), an integer, a boolean or a number. A
  derived class is read in the set `axioval:classification`, so rooms are
  grouped by their space use with `"propertySet": "axioval:classification",
  "property": "space-use"`.
- `{"kind": "classification", "system": "Uniclass"}` groups by the code the
  member's assignment in that system carries, as its source states it.

- `{"kind": "compartment", "separators": …, "boundary": …}` forms
  compartments, below.

Members of one source sharing a value form one group. Each group is a
derived object, never a model object: its identity is
`axioval:group/<grouping>/<value>` in its members' source, its kind
`axioval:group`, and it states nothing a source states.

- A `derivedGroup` selector selects a grouping's groups, `{"kind":
  "derivedGroup", "grouping": "flats"}`. Only it reaches them, as only an
  `entityType` selector reaches resource objects, so no rule written before
  groupings existed selects a group. A report carries the groups its
  outcomes name in `resources`.
- The relationship `axioval:derived.group;by=<grouping>` runs from each
  member to its group, so `backward` from a group reaches its members, and
  a shared-group query from a member reaches the others of its group.
  `group-composition` checks each flat's rooms with `relationship`
  `axioval:derived.group;by=flats`, `direction` `backward`, and reports the
  rooms in no flat through `ungrouped_selector`.
- The reserved set `axioval:group` states a group's `key` (the value its
  members share, as text) and `members` (how many, an integer); no other
  object has either. `axioval:measured` `area` is the union of its members'
  footprints, each overlap counted once, so `keyed-limit` with `plan-area`
  bounds a flat's area.

A selected object without a value (absent, null or blank text, or no code
in the system) is ungrouped: a member of no group. One whose selection or
value cannot be read (an unreadable or undecided property, a list or a
quantity, several codes in the system, an assignment naming no system)
could belong to any group of its source, or to one not listed: every group
of that source is then undecided (its members, its count and its area are
not evaluated), and the source's list of groups is incomplete, so a rule
selecting them is not evaluated for that source as a whole.

### Compartments

Fire compartments are often implied by fire-rated walls and slabs rather
than stated as zones. A `compartment` grouping derives them: the connected
regions of its members not separated by an element `boundary` selects.

```json
"by": {
  "kind": "compartment",
  "separators": {"kind": "anyOf", "operands": [
    {"kind": "entityType", "objectType": "t.Wall"},
    {"kind": "entityType", "objectType": "t.Slab"}]},
  "boundary": {"kind": "property", "propertySet": "t.Pset_WallCommon",
               "property": "t.FireRating", "operator": "matches",
               "value": {"type": "string", "value": "EI ?(60|90|120)"}},
  "tolerance": 0.05,
  "overlap": 0.3
}
```

- Two members join across a `separators` element that `boundary` surely
  does not select when they lie on opposite faces of it, by the geometric
  adjacency `axioval:derived.adjacent-across;tolerance=…;overlap=…`
  (defaults 0.05 and 0.3; see [Relationships derived from
  geometry](./capabilities.md#relationships-derived-from-geometry)). Two
  members on the same face are not joined by it. Compartments read the
  geometry's faces directly, since stated space boundaries record none.
- Two members also join where they touch (within `tolerance`, by the
  proximity service) without lying on opposite faces of a boundary element,
  so an open-plan zone modelled as several spaces is one compartment.
- Each connected region is a group, keyed by its least member's identity
  (`axioval:group/<grouping>/<least member>`); a member joined to nothing is
  a compartment of its own. Its `area` is the union of its members'
  footprints, so `keyed-limit` with `plan-area` bounds the compartments'
  area, and every member lies in exactly one compartment.

Nothing undecided is assumed. A join that may or may not hold (an element
whose selection by `separators` or `boundary` is undecided, a contact the
measurement cannot settle, a contact beside a boundary element whose faces
are unknown) leaves the compartments it could merge undecided. An element
that may join members whose faces cannot be read (a room straddling the
tolerance, an element neither a straight wall nor a flat slab), a member
whose extent cannot be measured, and a missing derived-relationship or
proximity service leave every compartment undecided. A tolerance must be
finite and at least zero and an overlap finite and positive, or the
grouping is invalid.

The runtime derives the groupings after the classifications and before any
rule runs. Compilation refuses (`EngineError::InvalidGrouping`) a grouping
declared under another key than its id, an id that is blank or holds `:`,
`;`, `|`, `/` or whitespace, members reading a rule's outcome or a derived
group, a key in the set `axioval:group`, a blank system, a grouping two
rulesets compiled together declare differently, and any grouping when the
host registered no outcome refiner. A classification row reading derived
groups is refused too (`InvalidClassification`), and a `derivedGroup`
selector naming an undeclared grouping is an unknown concept (kind
`axioval:grouping`).

## Declared relations

Users often relate objects the model does not relate: this pump serves
that room, this detail belongs to that wall. A ruleset's `relations`
declare such relations, by id, so rules can follow them like any
relationship the model states:

```json
"relations": {
  "serves": {
    "id": "serves",
    "name": {"default": "Serves", "translations": {}},
    "from": {"kind": "entityType", "objectType": "ex.pump", "includeSubtypes": true},
    "to": {"kind": "entityType", "objectType": "ex.room", "includeSubtypes": true},
    "by": {"kind": "pairs", "scheme": "ifc-globalid", "pairs": {
      "type": "tableFile", "path": "serves.csv", "sha256": "…",
      "columns": [
        {"id": "from", "header": "pump", "kind": "string"},
        {"id": "to", "header": "room", "kind": "string"}
      ]}}
  }
}
```

The relationship `axioval:derived.relation;id=<id>` runs from each object
`from` selects to the objects `to` selects that `by` pairs it with, and
`backward` from a to-object to its from-objects. It is usable wherever a
relationship is: a traversal `relationship` or `path` (so
`property-comparison` compares each pump's capacity with the requirement of
the rooms it serves), a `related` selector, takeoff `related` columns and
categories. An object is never related to itself, and `follow_chain` (or a
`+` step) follows the relation transitively.

`by` pairs objects in one of three ways:

- `pairs`: a `table` or `tableFile` value (see
  [Tables from data files](./capabilities.md#tables-from-data-files)) with
  the required text columns `from` and `to`, one pair per row. Without
  `scheme` each cell names an object by its identity as reports write it
  (`<system>:<document>/<local id>`, such as
  `ifc-step:model.ifc/#4711`); with `scheme` by the external id the object
  carries in that scheme (such as `ifc-globalid`). A blank cell or a cell
  of another kind is refused when the package is bound.
- `supplied`: pairs the host supplies at check time, beside the model,
  so the rule package stays the same for every project. The rows name
  objects as `pairs` rows do (with the optional `scheme` alike) and are
  read from a CSV file or one sheet of an xlsx workbook, by the table file
  rules (header row, blank rows skipped, every header declared). The
  optional `columns` declare the text columns `from` and `to` and their
  headers as a `tableFile`'s do; omitted, the headers are `from` and `to`.
  The package states no pairs and no digest itself:

  ```json
  "by": {"kind": "supplied", "scheme": "ifc-globalid", "columns": [
    {"id": "from", "header": "pump", "kind": "string"},
    {"id": "to", "header": "room", "kind": "string"}
  ]}
  ```

  A host reads a file with `SuppliedPairs::read` (or builds rows it read
  itself with `SuppliedPairs::new`) and hands them to
  `ExecutionPlan::supply_relation` before the run; `axioval check` does
  this for each `--relations <id>=<file>` (see [the CLI](./cli.md)). Every
  pair's evidence cites the SHA-256 of the file it was read from
  (`axioval:derived.relation;id=<id>:pairs;sha256=<digest>#row=<n>:…`), so
  a run is reproducible. Supplying pairs to a relation no ruleset
  declares, to one that states its own pairs, twice, or rows that are not
  non-blank text `from` and `to` cells is refused
  (`EngineError::InvalidRelation`). A supplied relation given no pairs is
  never an empty relation: every object's partners are undecided, so
  every rule walking it is not evaluated with the reason. Supplied pairs
  naming unknown objects behave exactly as listed ones (below).
- `property`: equal values. A from-object relates to every to-object whose
  `to` property states the value of its `from` property, read as a grouping
  key is (text as stated, an integer, a boolean or a number), across
  sources. An object without a value relates to none.

Nothing undecided is guessed. A from-object whose selection or key cannot
be read could relate to any to-object: its own partners and every
to-object's from-objects are then undecided (and the other way round), so
the rules walking them are not evaluated. A listed pair naming an object
the model does not hold, an identity several objects carry, or an object
the end's selector does not select leaves the other end's partners
undecided (every answer, when neither end names a single object); it is
never dropped. `axioval_engine::unknown_relation_objects` lists every end
of a listed pair naming no single object, and `axioval check` reports each
as an integrity issue `relation-object-unknown`.

The runtime derives the relations after the groupings and before any rule
runs; their selectors may read classifications and groups. Compilation
refuses (`EngineError::InvalidRelation`) a relation declared under another
key than its id, an id that is blank or holds `:`, `;`, `|`, `/` or
whitespace, selectors reading a rule's outcome or walking a declared
relation, undeclared property concepts, pairs that are not a table of
non-blank text `from` and `to` cells, supplied `columns` other than the
text columns `from` and `to` with distinct headers, a blank `scheme`, a
relation two
rulesets compiled together declare differently, and any relation when the
host registered no outcome refiner. The listing of every edge
(`RelationshipSelectionService::edges`) is refused for declared relations,
as for every derived relationship.

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
| `slope[;face=top\|bottom]` | a face's steepest gradient, an angle | `VerticalExtentService::measure_face_normals` |
| `slope_along;direction=x\|y\|own_x\|own_y[;face=…]` | a face's signed gradient in a plan direction, an angle | `VerticalExtentService::measure_face_normals`, `ObjectFrameService` for own axes |
| `cross_fall;axis=x\|y\|own_x\|own_y[;face=…]` | a face's unsigned gradient across a plan axis, an angle | as `slope_along` |
| `gradient_direction[;face=…]` | the plan bearing of a face's steepest descent, an angle | `VerticalExtentService::measure_face_normals` |
| `inclination;axis=own_x\|own_y\|own_z` | the tilt of one of the placement's axes, an angle | `ObjectFrameService` |

### Names with parameters

Several names take `;`-separated `key=value` parameters, part of the property
name so that every selector, property reference and table key can carry
them unchanged. Keys are matched ignoring ASCII case.

### The registry

Every measured name is declared once, in `axioval_ir::measured`
(`MEASURED_VALUES`, sorted by name). A descriptor states the name, its
typed parameters (`path`, a `sourceKind` or a `length` with a minimum;
required or with a default), the dimension and SI unit of the value, the
services a run needs, its exactness (`stated` or `measured`), what leaves
it not evaluated, and an English and German label and help text. Editors
and catalogues read the same descriptors; a new measured value is
registered there, never parsed anywhere else.

`axioval_ir::measured::parse` reads a name against the registry, and the
engine resolves only what it accepts. Compilation refuses a rule reading
an unknown name, an undeclared or repeated parameter, a missing required
one or a value of the wrong kind with `EngineError::InvalidMeasured`,
whose detail is the registry's message:

```text
rule `r` reads measured value `boundary_area;kind=wall;depth=1`:
`boundary_area` takes no parameter `depth`; it takes `kind`, `plane`
```

- `bottom_above_level;path=<steps>`: the containment path is the rule's to
  state, never a host default, because only the package knows which
  relationship places its objects on a level (an IFC element in its storey
  is `IfcRelContainedInSpatialStructure:backward`, a space
  `IfcRelAggregates:backward`). Steps are `,`-separated and written as a
  `related` selector's steps (`Relationship[|Relationship…][:forward|backward|either][+]`),
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

### Slopes, falls and tilts

The slope family measures a face of the body from the normals the
vertical-extent service certifies for each planar piece of it
(`FaceNormals`: a box per piece sure to hold its normal). `face=top`, the
default, is the pieces looking up and `face=bottom` those looking down, on
a closed body by its outward normal (a mesh wound inside out is read by the
sign of its volume); an open surface, such as a terrain sheet, is both.
Vertical sides belong to neither face.

Every value is an angle, in radians, computed with the expression
language's sound interval arithmetic, and is the hull over the face's
pieces. A curved or warped face therefore gives the range of its slopes,
never one triangle's. A level piece of exactly stated vertices has an exact
zero slope, and `convertSlope` turns an angle into a ratio or percent.

- `slope` is each piece's steepest gradient, in `[0, π/2)`.
- `slope_along;direction=…` is the gradient along a plan direction, rising
  positive: world `x` or `y`, or the object's own `own_x` or `own_y`
  projected to plan.
- `cross_fall;axis=…` is the magnitude of the gradient a quarter turn from
  the axis.
- `gradient_direction` is the compass bearing of steepest descent,
  clockwise from plan north (the y axis); its interval's midpoint lies in
  `[0, 2π)`, so a face descending about north may read `[-0.1, 0.1]`.
- `inclination;axis=own_z` is the own z axis's tilt from the vertical;
  `own_x` and `own_y` are measured from the horizontal, unsigned.

A face that may stand vertical somewhere has no gradient there, and
`gradient_direction` has no answer when a piece is level or the pieces
descend more than a half turn apart: each leaves the object not
evaluated, as does a body without the face asked for.

On a mesh the pieces are its triangles. The Axiolid adapter boxes each
cross product by a bound on its rounding, so exactly stated vertices give
exact normals. A tessellated mesh lies within its chord deviation `d` of
the true surface, so its normal over a triangle may lean by about `2d / h`
for the triangle's least height `h`. Each box widens by that much, and a
triangle too small for its deviation leaves the face unmeasured. On a
ramp, the steepest piece of the top face is the run that
`WalkingSurfaceService::measure_sloped_runs` measures, and the landings
are level.

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

## Derived values

A ruleset's `values` name expressions once ([Expressions](./expressions.md)), evaluated for every object, and every reader reads them like a property of the reserved set `axioval:value` (`axioval_ir::VALUE_SET`): selectors, expressions (also as a `derived` node), `property-predicate` and the other property capabilities, takeoff columns, grouping keys and classification rows.

```json
{"values": {
  "margin": {"name": {"default": "Cover margin", "translations": {}},
             "expression": {"kind": "subtract",
               "left": {"kind": "property", "propertySet": "Pset_Concrete", "property": "Cover"},
               "right": {"kind": "literal", "value": {"type": "quantity", "value": 30.0, "unit": "mm"}}}},
  "short": {"name": {"default": "Cover too short", "translations": {}},
            "expression": {"kind": "compare", "operator": "lessThan",
              "left": {"kind": "derived", "name": "margin"},
              "right": {"kind": "literal", "value": {"type": "quantity", "value": 0.0, "unit": "m"}}}}}}
```

- A value may read stated, measured, classified and other derived values. Compilation orders them so each follows those it reads, and refuses one reading an undeclared value or a cycle, naming it: `value `a`: the values read one another: a → b → c → a`. It type checks each value as it would a requirement, and the types reach every expression that reads them.
- A value reads no rule parameter: it is the ruleset's, not one rule's.
- The engine evaluates each value at most once per object in a run and caches it. Classifications and groupings derived before the rules read the values as far as they are derived then; rules read the final ones.
- A number with a unit is a quantity: exact when it is a point, otherwise a `measured` interval, whose evidence is never exact. Text, enumeration values, truths, dates and date-times are themselves, and `null` is an exact absence. An interval of plain numbers is read only by expressions; anywhere else it cannot be read. A value that cannot be computed leaves its reader not evaluated with the reason.
- Its evidence is located at `axioval:value/<name>` and cites the locators of every read it was computed from.

A decimal literal states a value as a source does: `30 mm` is the double nearest 0.030 m, the same one a source stating `0.030` holds, so a cover of exactly 30 mm meets a 30 mm bound rather than straddling it.
