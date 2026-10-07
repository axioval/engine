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
| `slope[;face=top\|bottom\|facing;direction=<x,y,z>;tolerance=<degrees>]` | a face's steepest gradient, an angle | `VerticalExtentService::measure_face_normals`, `measure_face_pieces` for `facing` |
| `slope_along;direction=x\|y\|own_x\|own_y[;face=…]` | a face's signed gradient in a plan direction, an angle | `VerticalExtentService::measure_face_normals`, `ObjectFrameService` for own axes |
| `cross_fall;axis=x\|y\|own_x\|own_y[;face=…]` | a face's unsigned gradient across a plan axis, an angle | as `slope` |
| `gradient_direction[;face=…]` | the plan bearing of a face's steepest descent, an angle | as `slope` |
| `inclination;axis=own_x\|own_y\|own_z` | the tilt of one of the placement's axes, an angle | `ObjectFrameService` |
| `extent;axis=own_x\|own_y\|own_z\|x\|y\|z` or `extent;direction=<x,y,z>` | the body's extent along an axis or direction, a length | `VerticalExtentService::measure_directional_extent`, `ObjectFrameService` for own axes |
| `length[;axis=own_x\|own_y\|own_z]` | a member's extent along its own sweep axis (`own_x` by default), a length | as `extent` |
| `thickness;direction=…` or `thickness;face=top\|bottom` | every local thickness along an axis or square to a planar face, a length | `VerticalExtentService::measure_thickness` |
| `perimeter` | the length of the footprint's boundary, holes included | `PlanAreaService::measure_footprint_perimeter` |
| `headroom;obstacles=<kinds>` | the least vertical clearance above a walking surface | `WalkingSurfaceService::measure_headroom` |
| `clearance_below;spaces=<kinds>` | the least clearance below a flight or ramp over the floors beneath | `WalkingSurfaceService::measure_clearance_below` |
| `clear_width;obstacles=<kinds>;band_to=<m>[;band_from=<m>][;along=flight\|runs]` | the narrowest clear width along a flight or a ramp's runs | `WalkingSurfaceService::measure_clear_width` |
| `clear_height` | a space's clear height | `SpaceService::measure_clear_height` |
| `distance;to=<kinds>[;mode=nearest\|farthest][;projection=…][;direction=…][;subject_surface=…;counterpart_surface=…][;within=<m>]` | the nearest or farthest counterpart's distance, as `distance` measures it | built in, over `ProximityService` |
| `count_within;to=<kinds>;radius=<m>[;from=<m>][;projection=…][;direction=…]` | how many counterparts lie within a radius, a plain number | built in, over `ProximityService` |
| `duplicate_count`, `boundary_gap[;at_least=<m>][;measure=total\|longest][;elements=<kinds>]`, `intersection_count[;tolerance=<m>][;elements=<kinds>]`, `cap_coverage;cap=top\|bottom[;elements=<kinds>]`, `support_count[;of=slabs\|roofs]` | a space's `space-validation` aspects | `SpaceService` |
| `largest_unallocated_region`, `unallocated_share` | a storey's floor no space covers | `SpaceService::measure_unallocated_regions` |
| `facade_area`, `face_area` | the facade area, or the largest plane face's area | `FacadeAreaService` |
| `plan_overlap;with=<kinds>[;measure=largest\|total]`, `uncovered_area;by=<kinds>[;growth=<m>]` | the footprint's overlap with, or the area left uncovered by, objects of kinds | `PlanAreaService` |
| `contact_area`, `contact_share` `[;with=<objects>][;side=below\|above][;gap=…;intersection=…;polygon=…]` | a face's contact with the objects named (kinds or `@` a selector parameter; every other object without `with`), or its share; objects the selector cannot decide may add contact, up to the whole face | built in, over `ContactService` |
| `effect_covered_area`, `effect_covered_share` `;sources=<kinds>[;blockers=<kinds>][;reach=grown\|travel\|visible][;range=<m>]` | the part of the footprint the sources' effect areas cover, or its share | `PlanAreaService::measure_coverage` |
| `boundary_covered_share`, `boundary_uncovered_area`, `boundary_overlap_area` `[;plane=<m>]` | a space's declared boundaries over its body's surface | `BoundaryCoverageService` |
| `boundary_off_surface_count[;plane=<m>]` | how many of a space's declared boundaries lie on no face of its body, a plain number | `BoundaryCoverageService` |
| `counterpart_uncovered_share;by=<objects>[;measure=plan\|height\|elevation\|plan_and_height][;horizontal=<m>][;vertical=<m>][;axis_tolerance=<degrees>][;frame=<objects>][;infill_above=<share>]` | the share of the footprint, height or elevation outside every counterpart, as `counterpart-coverage` measures it (kinds, or `@` a selector parameter, whose undecided objects may cover) | built in, over `PlanAreaService`, `ProximityService`, `VerticalExtentService`, `PlanSpanService` |
| `counterpart_uncovered`, `counterpart_whole` (the same arguments) | the part left uncovered and the whole it is a share of: an area in plan and in the elevation, a height for `height` | the same |
| `counterpart_covering` (the same arguments) | how many counterparts cover part of the element, from those surely covering to every one that may, those that cannot be read included | the same |
| `effective_reaching`, `effective_area`, `effective_covered`, `effective_share`, `effective_capacity`, `effective_unread` `;sources=<objects>[;blockers=<objects>][;mode=grown\|touching\|travel\|visible];range=<m>[;touch_tolerance=<m>][;area_property=<set/name>][;access_path=<steps>;door_selector=…;opening_selector=…;space_selector=…][;capacity_property=<set/name>;capacity_multiplier=<n>\|capacity_multiplier_property=<set/name>]` | as `effective-coverage` measures an element: how many sources reach it (stated absent where it states no area under `area_property`), the area the cover is a share of, the part covered and its share (citing the sources surely contributing, noting why it may be covered more), the summed capacity over the contributions read and how many cannot be read | built in, over `PlanAreaService::measure_coverage` and `ProximityService` |
| `distance_items` (every `distance` parameter under its own name, `selection`) | what `distance` judges of a subject: one item | `apart_checked`, `apart` (undecided for why it is open, the item's `reason` stating why where not incomplete evidence), `apart_words`, `apart_related`; `reach_checked`, `reach`, `reach_words`, `reach_related`, `none_within`; `count_checked`, `count`, `count_words`, `count_related` |
| `distance_open` (the same arguments; of the project) | each object `distance`'s selections and broad phase leave open beyond its subjects | `object`; `open` (undecided for why), `reason` |
| `effective_missing` (members, the same arguments) | each surely contributing source stating no capacity or multiplier: its `source`, the `property` and `missing` | the same |
| `counterpart_infill` (the same arguments) | in the elevation, whether the frame's infill covers: 1, 0, or between where undecided; it cites the frame's members | the same |
| `opening_area;path=<steps>[;length_axis=…;height_axis=…][;minimum=<m²>][;openings=<kinds>\|@selector]` | the summed section areas of a host's openings on its middle plane (among `openings`, every object where unstated; one reached that they cannot decide leaves it not evaluated), citing every opening reached | built in, over the body facts |
| `opening_section_area;host_path=<steps>[;length_axis=…;height_axis=…]` | one opening's section area on its host's middle plane | built in, over the body facts |
| `opening_count;path=<steps>[;length_axis=…;height_axis=…][;minimum=<m²>][;openings=…]`, `middle_face_area[;length_axis=…;height_axis=…]` | how many openings take area from a host's middle plane (citing them), and that face's area, as `empty-host` compares them; both opening values place a host's openings once per run | built in, over the body facts |
| `door_clear_width[;stated=<set/name>][;from_leaves=passage\|widest-leaf][;overall=<set/name>;deduction=<m>]`, `door_clear_height[;stated=…][;overall=…][;lining=…][;threshold=…]` | a door's clear width or height, as `keyed-limit` reads it | built in, over the door's properties and leaves |
| `sill_height;floor_path=<steps>[;measure=greatest\|least]`, `threshold_step;floor_path=<steps>[;threshold=<set/name>][;ramps=<kinds>;ramp_reach=<m>][;measure=…]` | above the floors a path reaches | built in, over `VerticalExtentService` (and `ProximityService` for ramps) |
| `leaf_count`, `leaf_width[;measure=widest\|narrowest\|total]`, `swing_area`, `swings_into;path=<steps>` | a door's or window's leaves and swing | built in, over `ObjectFrameService::leaves` and `FreeSpaceService` |
| `hinged_leaves` | how many hinged leaves a door or window has; refused for an object with none, so a template guards a check of its swing with it | built in, over `ObjectFrameService::leaves` |
| `profile_dimension;name=<column>`, `profile_slope;name=<column>` | a dimension or slope of the member's swept profile, by `allowed-profile`'s column names | built in, over the body set |
| `section_area`, `section_modulus[;axis=strong\|weak]` | the profile's section area and elastic section modulus, where its family defines them | built in, over the body set |
| `angle_to;path=<steps>[;between=axis\|face_normal]` | the angle to the objects a path reaches, an angle | `PlanSpanService::measure_rectangle` or `VerticalExtentService::measure_face_normals`, `RelationshipSelectionService` |
| `skew;path=<steps>` | how far the long axis is from square to the reached objects', an angle | `PlanSpanService::measure_rectangle`, `RelationshipSelectionService` |
| `bearing;axis=own_x\|own_y\|long[;reference=project_north\|true_north]` | an axis's plan bearing clockwise from north, an angle | `ObjectFrameService` or `PlanSpanService`, `CoordinateSystemService` for true north |
| `rectangle_side;side=width\|length` | the shorter or longer side of the footprint's least-area rectangle, a length | `PlanSpanService::measure_rectangle` |
| `obstruction_count;obstacles=<kinds>;reach=<m>;at=ends\|sides\|within[;side_zone=<m>]` | how many ends or sides of that rectangle obstacles obstruct, or how many stand within it, a plain number | built in, over `PlanSpanService`, `ProximityService`, `VerticalExtentService` |
| `band_uncovered_area;members=<kinds>;member_path=<steps>;angle_tolerance=<degrees>;maximum=<m>;footprints=<kinds>;footprint_path=<steps>` | the largest area of a reached footprint outside every band between parallel members at most `maximum` apart | built in, over `PlanSpanService`, `ProximityService`, `VerticalExtentService`, `PlanAreaService::measure_outside_bands` |
| `plan_area[;measure=footprint\|facade]` | the object's own area as `plan-area` measures it, an empty footprint refused | built in, over `PlanAreaService` or `FacadeAreaService` |
| `contact_gap[;with=<objects>][;side=…;gap=…;intersection=…;polygon=…]` | the distance to the nearest object named the contact service reports, none where it reports none; refused while the selection leaves an object undecided | built in, over `ContactService` |
| `level_rise;levels=<kinds>;order=<set/name>[;anchor=<steps>][;path=<steps>][;lowest=…][;highest=…][;contents=<steps>;content_kinds=<kinds>]`, `prevailing_rise;…[;tolerance=<m>]` | a level's rise to the next one up, and the prevailing rise among its anchor's levels, as `level-spacing` measures them | built in, over the order property and `VerticalExtentService` |
| `prevailing_elevation;side=bottom\|top;spaces=<steps>[;kinds=<kinds>][;tolerance=<m>]` | the prevailing bottom or top elevation among the spaces of the space's level | built in, over `VerticalExtentService` |
| `levels_above`, `levels_below` `;levels=<kinds>;path=<steps>` | how many storeys of the source lie above or below the object's one storey by their `Elevation` attribute | built in, over the property and relationship services |
| `storey_end;end=top\|bottom;storeys=<objects>[;relationship=…;direction=…\|path=<steps>][;follow_chain=…][;skip_absent_relationship_ends=…]` | 1 where the object's one storey is the highest or lowest of its source by the storeys' `Elevation` attribute, 0 where not, as `slab-contact` leaves it out | built in, over the property and relationship services |
| `undecided_count[;objects=<objects>]` | how many objects other than this one a selection cannot decide; none without `objects` | built in, over the rule's selection |
| `stack_distance;measure=top_to_top\|bottom_to_bottom\|top_to_bottom;slabs=<objects>;ratio=<share>` | the distance to the next slab up in the slab's stack, none where none stacks above | built in, over `VerticalExtentService`, `PlanAreaService` |
| `stack_prevailing;measure=…;slabs=<objects>;ratio=<share>[;tolerance=<m>]` | the distance the consecutive slabs of the slab's stack prevailingly share within the tolerance, none where it has none above, its stack fewer than two pairs or none prevails | built in, over `VerticalExtentService`, `PlanAreaService` |
| `shelf_length`, `shelf_clear_height` `;depth=…;horizontal=…;vertical=…;bottom=…;top=…;clearance=…;access=<steps>[;doors=<objects>][;openings=<objects>][;spaces=<objects>]` | a space's running metres of shelving and its clear height, as `shelf-capacity` measures them | built in, over `LinearQuantityService` |
| `body_extent;axis=right\|forward\|up` | the body's depth along one of its own placement axes, as `body-extent` measures it | built in, over `ObjectFrameService`, `VerticalExtentService` |
| `body_position;axis=right\|forward\|up;end=low\|high` | where the body begins or ends along one of its own placement axes, from the origin of the coordinates along it | built in, over `ObjectFrameService`, `VerticalExtentService` |
| `triangle_count` | how many triangles the host's mesh of the body holds | built in, over `TriangleCountService` |
| `coordinate_shift;of=world\|site\|map`, `coordinate_turn;of=world\|site\|north\|map`, `map_scale_change`, `map_target_change`, `map_conversion[;of=own\|reference]` `[;reference=<discipline>]` | of a source: how its coordinate system departs from the reference source's | built in, over `CoordinateSystemService` |
| `coordinate_reference[;reference=<discipline>]` | of the project: how many sources are compared with the reference source, which it cites | built in, over `SourceDisciplines` |
| `envelope_size;derivation=all-spaces\|gross-area-groups[;bounding=<objects>][;groups=<objects>;group_path=<steps>]` | of the project: how many objects the derivation places on the building envelope | built in, over `EnvelopeMembershipService` |
| `on_envelope`, `declared_external`, `bounds_envelope` `;derivation=…[;bounding=…][;groups=…;group_path=…]` | whether the derivation places the object on the envelope, whether the model declares it external, whether the envelope is derived around it | built in, over `EnvelopeMembershipService` |
| `external_declarations;derivations=<derivations>[;bounding=…][;groups=…;group_path=…];objects=<objects>` | of a source: how many of its objects the model declares external in any derivation, up to those of unknown declaration | built in, over `EnvelopeMembershipService` |
| `any_external;host_selector=<objects>;external_property=<set/name>`, `host_walls`, `undeclared_hosts`, `possible_hosts` (the same arguments) | of a source, as `opening-spaces` reads it: whether any of its walls declares itself external (1, 0, or between where a wall declaring nothing or an object that may be a wall could; read in identity order up to the first that does); how many walls it holds; how many declare nothing usable where none declares itself external; how many more objects may be walls. `any_external` cites the walls where none does | built in, over the property resolution |
| `station`, `offset[;side=left\|right]`, `height_above_gradient` `;alignment=<kinds>[;path=<steps>]` | the reference point's station along an alignment, its signed plan offset from it and its height above the gradient line, each a length | `AlignmentService::measure_alignment_position`, `TypeHierarchyService`, `RelationshipSelectionService` for a path |
| `alignment_curvature`, `alignment_radius`, `alignment_gradient`, `alignment_cant` `;alignment=<kinds>[;path=<steps>]` | the alignment's plan curvature (per metre) and radius, gradient and cant at the reference point's station | `AlignmentService::measure_alignment_parameter` |
| `station_section_area`, `station_section_thickness[;direction=lateral\|up]` `;alignment=<kinds>[;path=<steps>];station=<m>` | the area and the vertical or horizontal reach of the object's own body in the section normal to the alignment at a station | `AlignmentService::measure_section` |
| `envelope_intrusions;bodies=<kinds>;envelope=<lateral:up,…>;from=<m>;to=<m>;step=<m>` | of an alignment: how many bodies of the kinds reach into a clearance envelope swept along it, a count from the sure to the possible intrusions | `AlignmentService::measure_envelope`, `TypeHierarchyService` |

### Names with parameters

Several names take `;`-separated `key=value` parameters, part of the property
name so that every selector, property reference and table key can carry
them unchanged. Keys are matched ignoring ASCII case.

### Rule parameters and the anchor as arguments

A rule's expression may hand a measured value its own parameters instead
of literals: `@name` in a parameter's place names the reading rule's
parameter `name`, and `@anchor` the object the rule checks (the anchor
whose members an aggregate reads a value on, otherwise the object itself).
This is how a measurement receives what a selector picks, or is taken
between two objects:

```json
{"kind": "property", "propertySet": "axioval:measured",
 "property": "shelf_length;depth=@shelf_depth_metres;horizontal=0.3;vertical=0.35;bottom=0.1;top=2;clearance=0.9;access=@access_path;doors=@door_selector"}
```

Which parameters take a reference, and of which kind, is the registry's
(`MeasuredParameterKind::reference`, listed in the catalogue as each
parameter's `references`):

| Measured parameter | `@name` names a rule parameter that is |
| --- | --- |
| `length` | a `number` or `integer` of metres, or a `quantity` of length, at least the parameter's minimum |
| `path` | a `stringList` of steps |
| `paths` | a `stringList` of paths, each its steps separated by spaces; an empty list names none |
| `choices` | a `stringList` of its options, each once |
| `choice`, `text`, `sourceKind` | a `string` (a choice among its options) |
| `objects` | a `selector`; or `@anchor` |
| `property` | a `propertyReference` |
| `table` | a `table`, its rows as stated; a `table` parameter is written only as a reference |
| `number` | a `number` or `integer`, at least the parameter's minimum |
| `area` | a `number` or `integer` of square metres, or a `quantity` of area, at least the parameter's minimum |
| `truth` | a `boolean` |

A `vector` or `polygon` takes none, and only an `objects`
parameter takes `@anchor`. An `objects` parameter (the objects a value is
measured against) also still takes source kinds, `,`-separated, all of
them surely picked.

**Binding.** The compiler type-checks every reference against the rule's
parameters (`EngineError::InvalidExpression` at the read: a parameter the
rule does not state, or of another kind); a selector and a derived value
name none. A member list names them as a measured value does
(`clearances;obstacles=@headroom_obstacles`), bound for each object before
it is measured. An expression rule declares the parameters its
measured values read as authored parameters of its definition, a
`selector` among them. When the rule runs, the reference is bound before
anything is measured (`MeasuredCall::bind`): a selector to the objects it
picks (`MeasuredSelection`: those surely picked and those it cannot
decide, each sorted by source-qualified identity, read once per rule
through the run's one selection), the anchor to itself, a length, path,
text, property reference, table, number or truth to the value stated. The provider receives the bound call, never the
selector, and decides what undecided objects leave (`shelf_length` leaves a
space open where an undecided door may reach it). A reference that cannot
be bound (a parameter of another kind or not realisable, a selection whose
objects cannot all be listed) leaves the value not evaluated for its
reason, an invalid declaration for the parameter's own fault, never a
default. A bound read is measured once per run for its object, its name as
written and its bound arguments (the run's `MeasuredMemo`), so two rules
naming the same selection share it; a template reads one naming no anchor
for a chunk of objects together, bound once per rule
(`MeasuredValues::read_bound_batch`).

**What it was measured against.** A provider may cite the objects a value
was measured against and the evidence that reached them
(`MeasuredProvider::measure_cited`, `Citation`); a value read with bound
arguments (`MeasuredValues::read_bound_batch`) carries them, so an
expression rule's finding relates them and a template's `related` names
them.

### The registry

Every measured name is declared once, in `axioval_ir::measured`
(`MEASURED_VALUES`, sorted by name). A descriptor states the name, its
typed parameters (`path`, `paths` (several paths, `,`-separated, each
its steps separated by spaces), a `sourceKind`, a `length` with a minimum, a
`choice`, `choices` (several options, `,`-separated, each once), a
`vector`, a `property`, a `text`, such as a discipline, or a
`polygon` of `lateral:up` vertices, `,`-separated, at least three and never
crossing or touching itself;
required or with a default), the dimension and SI unit of the value, the
services a run needs, its exactness (`stated` or `measured`), its subject
(`subject`: `source` or `project`, each object's where left out; see
[Subjects](#subjects)), what leaves it not evaluated, and an English and
German label and help text. Editors
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

`face=facing;direction=<x,y,z>;tolerance=<degrees>` (not taken by
`slope_along`, whose `direction` is a plan axis) is the pieces of a closed
body's whole boundary (`VerticalExtentService::measure_face_pieces` with
`FacePieceSet::Boundary`) whose outward normal lies within the tolerance,
at most 180°, of the direction: a batter facing east is
`direction=1,0,0;tolerance=60`. A piece faces the direction when the
normal box of every part of it surely does, judged with sound interval
arithmetic against the cosine of the tolerance; a part surely outside
leaves the piece out. A piece that may or may not face the direction
leaves the value not evaluated, and so does a body with no piece facing
it or an open surface, which has no outside. `direction` and `tolerance`
are stated with `face=facing` and with nothing else, or the name is
refused.

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
  the axis, taken per piece, so a crowned face falling equally to both
  sides measures that one fall.
- `gradient_direction` is the compass bearing of steepest descent,
  clockwise from plan north (the y axis); its interval's midpoint lies in
  `[0, 2π)`, so a face descending about north may read `[-0.1, 0.1]`.
- `inclination;axis=own_z` is the own z axis's tilt from the vertical;
  `own_x` and `own_y` are measured from the horizontal, unsigned.

The values above are the hull over the whole face: a single-body fill
whose top is a level crest between two batters measures a `slope` from
zero to the batters' fall, and every limit between stays undecided. The
member list `face_pieces[;face=…]` lists the face's pieces one by one, so
an aggregate judges each ([Measured members](#measured-members)): `max`
of their `slope` is the steepest piece, decided where its own interval
is, and `all(implies(area > 1 m², slope ≤ limit))` judges every piece but
the small ones. A piece is planar, or smooth across the triangles of a
tessellation whose normals may be one surface's; where the pieces part is
the service's, and any partition is sound, since a piece's values hold
over all of its parts. With `face=facing` a piece whose facing cannot be
decided is a possible member. Its `slope` is the hull over its parts in
`[0, π/2]` (`π/2` where a part may stand vertical), its `area` the
service's interval, and its `gradient_direction` as above, `null` for a
piece lying exactly level and undecided where it may be level or vertical.

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

#### Which slope a value reads

The engine has two slope definitions, and each reader takes one on
purpose:

| Reader | Slope | Form |
| --- | --- | --- |
| `slope`, `slope_along`, `cross_fall`, `face_pieces`' `slope` | the face's normals, per piece (`measured/surface.rs`) | an angle, the hull over the pieces |
| `ramp-geometry`'s `slope_limits` and `slope_tolerance`, the `runs` list's `slope` | the run's rise over its horizontal length (`SlopedRun::slope`) | a ratio, one per run |

A ramp is judged on its runs because what a ramp rule bounds is a run: the
gradient between two landings, together with that run's length and rise
in the same `slope_limits` row, and the spread between runs. The
walking-surface service decides where a run ends and a landing begins, and
the run's slope is derived from its end positions over its whole length,
so a tessellation's chord deviation `d` costs about `d / length`, where the
normals of a mesh triangle of least height `h` lean by about `2d / h`. On
a planar run whose direction is its steepest ascent (as the service
measures it) the two agree: the run's ratio is the tangent of its piece's
`slope`. They differ where they should: a run that also falls across has
a steeper face `slope` than its run slope, and a warped run's face `slope`
is the range over its pieces.

The face slopes read any face of any body, with no walking direction and
no runs: a roof, a batter, a cross fall, a terrain sheet. Use `runs` (or
`ramp-geometry`) to judge a ramp's gradient, and `slope` or `cross_fall`
for a face's steepest or sideways fall, including a ramp's cross fall.

### Dimensions

`extent` is the body's depth along an axis: its highest less its lowest
point projected onto it, as `body-extent` measures it. The axis is an own
axis of the placement (`own_x`, `own_y`, `own_z`), a world axis, or a
stated direction `x,y,z`; exactly one of `axis` and `direction` is stated.
`length` is the extent along a member's own sweep axis: `own_x` unless
stated, `own_z` for a column.

`thickness` is an interval holding every local thickness of the body
along a direction: the length inside the body of each line along it,
between the faces looking along it (within 60°) and those looking back.
The body's steeper sides are not crossed. A tapered member therefore
spans its thinnest and thickest, and a sloped slab measured square to its
top (`face=top`, whose pieces must share one normal) is as thick
everywhere. The body must be closed. Exactly one of `direction` and `face`
is stated.

`perimeter` is the length of the footprint's boundary in plan, holes
included.

### Clearances

`headroom`, `clearance_below` and `clear_width` ask the walking-surface
service exactly what `stair-geometry` and `ramp-geometry` ask it, so the
value is the one they judge. `clear_height` asks the space service what
`space-validation` asks it.

The obstacles (or, for `clearance_below`, the spaces whose floors lie
below) are the project's objects of the source kinds named,
`,`-separated: `obstacles=IfcBeam,IfcDuctSegment`. Subtypes are included
where the source declares a type hierarchy; a source without one has no
subtypes, and its kinds match exactly. Headroom with nothing selected
above is an exact absence, and so is a flight standing above no selected
floor.

`clear_width` is measured between two heights above the pitch line
(`band_from`, default 0, and `band_to`, in metres) along a stair flight,
or with `along=runs` along each of a ramp's runs, the least over them.

### Doors and windows

The door values take `keyed-limit`'s steps:

- `door_clear_width` is the clear width the door states (`stated`, a
  property written `set/name`), else the clear width from its leaves
  (`from_leaves`), else the overall width less `deduction`. Each step is
  taken only after an exact absence.
- `door_clear_height` is the stated clear height, else the overall height
  less its head lining and threshold. A thickness the door does not state
  leaves only an upper bound.
- `sill_height` and `threshold_step` are measured above each floor the
  path reaches: the greatest by default, the least with `measure=least`.
  The threshold step adds a stated threshold. With `ramps` and
  `ramp_reach`, a ramp of those kinds surely within reach of the door and
  surely over a space is that side's floor, measured at its top; one that
  only may be leaves both floors possible, so the side's step is any of
  theirs. A floor that cannot be measured leaves the value unknown.

Each is cited as the capability cites it: exact where its evidence is (a
stated width, exact extents), inexact where it rests on the rule's
deduction or a derivation from the door's statements, even when the
interval is a point (`Measurement::Cited` with `exact: false`).

`keyed-limit`'s `quantity: measured` reads any of them, and judges each
fixture as the built-in quantity does. So does an `expression` rule: the
limit table becomes a `lookup` of the row the door's own keys select (a
`row` column telling a row without bounds from no row) and each bound a
comparison of the value rounded to a micrometre; a key reached along a path
becomes one conjunct per row, applying where every reached object has the
row's key. The parity harness holds on the clear-width, clear-height,
sill-height and threshold-step fixtures, ramps included, but where a path
key cannot be read as one value (the reached objects disagree, or there are
none), which the capability leaves open and the expression finds without a
row, and where one floor cannot be measured beside a failing one, which the
capability finds and the expression leaves open. The door-type defaults table is a
rule's and stays with the built-in quantities; a measured value reads what
the door states. A glazing ratio is a stated property and is read as one.

`leaf_count` and `leaf_width` read the door's leaves. `swing_area` is the
plan area they sweep, bracketed by inscribed and circumscribed polygons.
`swings_into` counts the spaces the path reaches that a leaf swings into,
probed as `door-swing` probes them. The `swing_spaces` members state per
space whether the door swings into it and whether it surely swings away
from it, so `door-swing` is an expression: `swing_not_into` is `none` of
the picked spaces swung `into`, `swing_into` the negation of `all` of them
swung `away` (a space neither probe lies in decides nothing).

### Profiles

A member's swept profile is read as `allowed-profile` reads it: one item,
a mirrored profile followed to its parent, a derived one refused. Its type
and name are stated (`axioval:body` `Profile.Type` and `Profile.Name`) and
are read as properties; each dimension and slope is a measured value under
the column names `allowed-profile` uses for every family, with none when
the family has no such dimension or the source leaves it unset. A profile
table is therefore an expression: an `or` over its rows, each an `and` of
the type, a `like` on the name and every dimension within the tolerance.

`section_area` is defined for rectangles, hollow rectangles (inner and
outer fillets counted), circles, hollow circles, ellipses and I-sections
with parallel flanges (fillets and flange edge radii counted);
`section_modulus` for rectangles and solid and hollow circles. Any other
family leaves it not evaluated. Both are computed over intervals rounded
outward from the stated dimensions, so they hold the exact value and cite
exact evidence. A radius the formula reads but the source leaves unset is
unknown, never zero: the value widens over every radius the family
allows (a fillet from none up to the smaller of the flange's outstand and
half the web's clear height, a flange edge up to the outstand and the
flange's thickness, a hollow rectangle's corners up to half its narrower
side), and its evidence is no longer exact. An I-section stating no flange
slope may taper, so its area is not evaluated.

### Areas, shares and coverage

Each area capability's numerator and denominator is a measured value from
the same request, so its verdict is an expression ratio:

| Capability | As measured values |
|---|---|
| `plan-area` | `plan_area` (`measure=facade`), or an `aggregate` `sum` of it over the member path, filtered by the member selector |
| `area-ratio` | the `divide` of two `aggregate` `sum`s of `area`, `facade_area` or a stated area, each filtered by its selector, or over `plan_area` or a stated area of the anchor |
| light area | `area-ratio`'s light-area numerator is an expression over the stated sizes: a `lookup` of the light area table, or `W × H − 2(W + H) × frame` |
| `plan-coverage` | `plan_overlap;with=<kinds>` over `plan_area` |
| `opening-area` | `opening_area;path=…` against the stated gross less net area, or an `aggregate` `sum` of each opening's `opening_section_area` over the path |
| `empty-host` | `opening_count;path=…` above zero and `opening_area;path=…` not short of `middle_face_area` by more than the tolerance |
| `slab-contact` | `contact_share;with=<kinds>;side=…` against the minimum ratio, one rule per severity band over `contact_area`, `contact_gap` and the share's part of the minimum, and `levels_above` or `levels_below` 0 for a top or bottom storey left out |
| `counterpart-coverage` | `counterpart_uncovered_share;by=<kinds>;measure=…` at most each band's threshold, one rule per band at its severity (in plan alone, also `uncovered_area;by=<kinds>;growth=<tolerance>` over `area`) |
| `effective-coverage` | `effect_covered_share;sources=<kinds>;reach=…;range=…` against the minimum |
| `space-boundary-coverage` | `boundary_covered_share`, `boundary_uncovered_area` and `boundary_overlap_area` against their bounds, and `boundary_off_surface_count = 0` |

The objects a capability selects are named here by source kind. A
capability's undecided members widen or leave open what a measured value
over the kinds' objects decides; an off-surface space boundary, which
`space-boundary-coverage` always reports, is no area but is counted by
`boundary_off_surface_count`.

`counterpart_uncovered_share` is measured by the built-in code of
`counterpart-coverage`: the cover from the plan broad phase, the least
cover (sure counterparts surely overlapping) bounding the share from above
and the most (possible ones too) from below, a counterpart without a
readable extent dropping the lower bound to zero, only axis-compatible
counterparts with `axis_tolerance`, and in the elevation a frame's infill
above `infill_above`. For `height`, `horizontal` is the growth by which a
counterpart overlaps in plan; a negative growth switches its check off,
as the capability's tolerances do (nothing grows, and the switched-off
check's share is refused). `plan_and_height` measures as `plan` and needs
the vertical-extent service where `vertical` is not negative: what a
coverage rule declaring both checks needs, read once for both. The share
cites the counterparts that surely cover part of the element, and notes
why it may be covered more (a counterpart whose extent cannot be read,
one whose cover cannot be measured, axes that are not proven). Each
element's cover is measured once per run, its counterparts' extents once
per selection, for every value and rule reading them.
Measured from exact evidence, it is cited exact, as the capability cites
it. One expression rule per band, `share <= threshold` at the band's
severity, reaches the capability's grades on its fixtures, except where a
share above the lowest threshold straddles a higher one: the capability
grades it by the band its upper bound reaches, the band rules by the band
it surely exceeds.

`space-validation` grades its aspects with fixed severities (a duplicate
or an intersection is an error, a low or uncovered boundary a warning, a
cap shortfall an error below 1 %, a warning up to 15 % and informational
below 98 %), so its rewrite is one expression rule per severity, each the
conjunction of the aspects graded at it. Its values read a refusal of the
space service as the capability does: unavailable or unmeasured is
incomplete evidence.

`opening_area` and `empty-host`'s values reach those capabilities'
verdicts on every fixture through the parity harness, but one: a wall
stating only one of its side areas, which `opening-area` leaves open,
reads the missing side as `null` in an expression, so the comparison is
`null`, a missing-information finding. The summed `opening_section_area` checks no two openings
against each other and knows no minimum area.

Expressions over these values reach `plan-area`'s, `area-ratio`'s,
`plan-coverage`'s and `slab-contact`'s verdicts on their fixtures, check by
check: own and summed footprints and facade areas within a range, ratios
of measured, facade and stated areas over a relationship or a path, an
anchor reaching no numerator object (a `count` of at least one), the
largest overlap's share of the footprint, the graded shortfall and absence
of contact, and the top or bottom storey left out. Ratios are rounded to
`1e-9` before they are compared, as the capability compares the quotient
the literal bound names. A sum over no member is 0 in its members' unit,
so a sum of areas divided by `1 m²` is a plain 0. These differ:

- an `area-ratio` member stating no area leaves its anchor open; in an
  aggregate it makes the sum `null`, so the rewrite requires every member
  to state one (`all` of `isDefined`) and finds the anchor instead;
- `plan-area` leaves an anchor with undecided members open unless its sum
  already exceeds the maximum; the aggregate measures the undecided members
  too, so a sum that stays within the bounds either way passes;
- `slab-contact` counterparts named by more than a kind need the rule's
  selector (`with=@counterparts`, as the template binds it): an undecided
  one leaves a shortfall open in the capability, while a rewrite naming
  kinds sends every object of them and finds it;
- `area-ratio`'s light-area numerator (a stated light area, else a size
  table row by type pattern, else a frame allowance, with an oversized
  member reported on its own) stays with the capability.

### Space aspects

Each aspect `space-validation` judges is a value from the same space
service request, so a comparison reproduces its verdict:

| Aspect | Measured value and comparison |
|---|---|
| duplicate bodies | `duplicate_count = 0` |
| clear height | `clear_height >= required` |
| uncovered boundary | `boundary_gap;at_least=<segment> = 0` (the total of gaps at least the segment long; `measure=longest` for the longest) |
| containment and intersection | `intersection_count = 0`: bodies containing or contained by the space, or partly overlapping it by area and higher than `tolerance` (default 5 mm) |
| cap coverage | `cap_coverage;cap=top >= 0.98`, a share from 0 to 1 |
| unallocated region | `largest_unallocated_region <= allowance`, on the storey |
| unallocated share | `unallocated_share <= maximum`, on the storey |

`elements` names the element kinds a request counts, as the capability's
element selectors do; without it the service's default applies.
`cap_coverage` with `elements` naming no existing object is an exact
absence, as the capability skips that cap. `support_count` counts the
model's slabs or roofs.

### Distances

`distance` and `count_within` are measured by the built-in code of the
`distance` capability: its broad phase, projections, surfaces and
counting of undecided counterparts. A comparison of the value therefore
reaches the capability's verdicts:

- `at_least` is `count_within` at least the count;
- `none_closer_than` is the nearest `distance` at least the minimum, none
  within it holding;
- `nearest` with a maximum is the nearest `distance` at most it, none
  within it failing.

The counterparts are the objects of the source kinds `to` names. The
search runs within `within` (default 1000 m) or the `radius`, so a
counterpart beyond it is proven out of reach, and none within it is an
exact absence. An undecided counterpart widens the value towards the
bound it could move: one whose distance could not be read, or whose
interval straddles. The nearest distance's lower bound is the least any
counterpart may come, and its upper bound the least of those surely
counted. `count_within` runs from those surely within to those possibly
within, and a decided count is an integer; it is cited exact when every
counterpart that may count was measured exactly, as `distance` cites a
shortfall.

A measured value without a dimension, such as a count, is a plain
number. Known only to an interval, it is a `measured` value without a
`dimension`:

```json
{"type": "measured", "value": {"lower": 2, "upper": 3}}
```

### Measured by built-in code

Some measured values reuse a capability's own measurement, such as a
distance in a mode or a clear width with its deductions, so the value and
the capability never disagree. They are measured by trusted built-in code
(`MeasuredProvider`) that a host registers with the capabilities:
`CapabilityRegistry::register_measured` refuses a name the registry does
not list, one the engine measures itself, and one another provider
measures. A run installs the providers. `CapabilityRegistry::install_measured`
installs them for `axioval_engine::measured_value` outside a run. Reading
a name no provider measures leaves the object not evaluated: a missing
service, never a value.

### Exactness of built-in measurements

A provider states each value's exactness ([Stated rather than
measured](#stated-rather-than-measured) explains the variants) from the
evidence it measured from, never from the value. Every provider
(`CapabilityRegistry::measured_providers` lists them) is held to
approximate evidence by a test that `axioval-rules`'
`tests/measured_exactness.rs` names; a provider without one fails it. The
sites that cite a value exact:

| Provider | Exact when | The interval holds |
| --- | --- | --- |
| `distance`, `count_within` | every counterpart that may be the nearest (farthest) or may count was measured exactly | the undecided counterparts, as `distance` cites them (`Cited`) |
| `door_clear_width`, `door_clear_height`, `sill_height`, `threshold_step` | every extent, leaf and stated value read is exact | rounding of the differences, widened outward |
| `leaf_count`, `leaf_width`, `swings_into`, `hinged_leaves` | the leaves (stated, exact by contract) and every probe are exact | a total width summed outward |
| `swing_spaces` (members) | the leaves and every containment probe are exact (both exact by contract) | nothing |
| `connected_spaces` (members) | the hosts' declarations and the spaces' relationship evidence are exact (an inexact one is refused) | nothing |
| `zone_checks` (members) | every body read is stated exactly (an inexact one is refused), and the proximity service's contact evidence a support cites is exact | nothing; a distance to a free outline known only from below reaches up to infinity |
| `limit_row`, `limited_values` (members) | every key, extent, leaf and stated value read is exact; a sill height or step cites its floor's extent on its own item | rounding of the differences, widened outward; a member area with undecided members, from its lower end up |
| `space_connections` (members) | the relationship evidence of every link and element read is exact (refused otherwise, which leaves the link undecided) | nothing |
| `profile_dimension`, `profile_slope`, `section_area`, `section_modulus` | every dimension the formula reads is stated | rounding, π included; an unset radius widens the value and makes it inexact |
| `flight_*`, `landing_*`, `stair_rise`, `end_width`, `walking_line_turns` | the flight's, run's or landing's evidence is exact | rounding of derived positions |
| `steps`, `runs`, `handrails` (members) | the flight's, runs' and landings' evidence is (handrails never are) | rounding |
| `guard_edges` (members) | always: the guard service refuses inexact evidence | nothing |
| `plan_diameter`, `well_*`, `centre_line_distance`, `recesses`, `end_walls`, `exit_pairs` | the plan-span, section or extent evidence (and for the centre line its rectangle and side distances) is exact | differences rounded outward; the centre line's walls that may lie nearer |
| `travel_distance` | walked with exact evidence | the possible exits, as `escape-route` cites them (`Cited`) |
| `free_placements` (members) | a placement is surely found (exact witness and support by contract) | nothing; an open search's placement is never exact |
| `rectangle_side`, `obstruction_count`, `axes_within` | the rectangles and every obstacle measured are exact | the undecided obstacles |
| `band_uncovered_area`, `parallel_pairs` | every pair and area is exact and every member was read | the possible bands |
| `level_rise`, `prevailing_rise`, `prevailing_elevation` | the heights' or extent's evidence is exact (points decide the prevailing value, as `level-spacing` decides it) | rounding |
| `shelf_length`, `shelf_clear_height` | always: the linear-quantity service refuses inexact evidence | nothing |
| `levels_above`, `levels_below`, `storey_end` | always: counted over stated elevations, which the property service answers exactly | nothing |
| `contact_area`, `contact_gap`, `contact_share` | always: the contact service refuses inexact evidence | the share's division, rounded outward; the contact undecided candidates may add |
| `undecided_count` | always: counted over the selection | nothing |
| `stack_distance`, `stack_prevailing`, `body_extent`, `plan_area`, `triangle_count` | every extent, frame, area or count is exact (for `stack_prevailing`, the prevailing pair's) | rounding |
| `counterpart_uncovered_share`, `counterpart_uncovered`, `counterpart_whole` | every evidence is exact and every counterpart that may cover was read | the undecided cover, as `counterpart-coverage` cites it (`Cited`) |
| `counterpart_covering`, `counterpart_infill` | always: counted over what was read | the undecided cover, or whether the infill applies |
| `effective_area`, `effective_covered`, `effective_share`, `effective_capacity` | the area's, the coverage's or the sum's evidence is exact (a stated value is) | the undecided cover or contributions (`Cited`) |
| `effective_reaching`, `effective_unread`, `effective_missing` | always: counted over what was read; a missing value as exactly as its absence was read | nothing |
| `coordinate_*`, `map_*` | always: coordinate systems are exact by contract | the arithmetic's rounding, widened by a bound on it |
| `opening_area`, `opening_count`, `opening_section_area`, `middle_face_area`, `opening_placements` | the body facts read are exact (they are, unless a source cites an estimate) | a sum of areas widened by its rounding; a single area's own arithmetic is not widened, which the capability's rounding allowance covers |
| missing-tactile and other defect counts | every finding's evidence is exact and nothing is open | the open checks |

The engine's own values follow the service's evidence the same way; a
`boundary_area`, a total `plan_overlap`, a `boundary_gap` total and the
space shares sum and divide outward, and `bottom_above_level` subtracts
outward.

### Angles between objects

`angle_to`, `skew` and `bearing` measure an object against other objects
or a reference direction. Every value is an angle in radians, rounded
outward.

- `angle_to;path=<steps>` (`between=axis`, the default) is the acute angle
  in plan between the long axes of the object's footprint and of each
  object the path reaches, in `[0, π/2]`. The long axes are those of each
  footprint's least-area rectangle, the axes `parking-bay` and
  `wall-spacing` judge, so the measured angle and a capability's
  alignment agree.
- `between=face_normal` measures between the top faces' normals instead,
  each turned to look up, in `[0, π]`: the hull over every pair of pieces,
  at most 250 000 pairs.
- `skew;path=<steps>` is how far the long axis is from square to the
  reached objects' long axes, `π/2` less their angle: a pier's skew to the
  deck it carries.

The path is written and walked as for `bottom_above_level`. Several
objects reached give the hull over them; none reached is an exact
absence. A footprint without a long axis of its own leaves the object not
evaluated: a square, a tie between least-area orientations, or a
tessellation, whose orientation is unproven.

`bearing;axis=…` is the plan bearing of an axis, clockwise from north:

- `own_x` or `own_y` is the placement's axis projected to plan, its
  midpoint in `[0, 2π)`;
- `long` is the footprint's long axis, which has no direction, its
  midpoint in `[0, π)`.

`reference=project_north`, the default, is the plan y axis.
`reference=true_north` is north as the source's coordinate system states
it, and a source stating none leaves the object not evaluated.

`axioval_engine::measured_value` reads one measured value of one object
with a run's services, outside a run, for a host previewing a value or a
test comparing it with a capability's judgement.

### Stairs and ramps

`flight_rise` and `flight_width` are a stair flight's rise and width,
`walking_line_turns` whether it turns (1) or is straight (0), and
`stair_rise` a whole stair's rise from its lowest flight's base to its
highest flight's top. `landing_depth`, `landing_width` and
`landing_clear_width` (between two heights above it, among the `obstacles`)
are the landing at an `end` (`bottom` or `top`, a ramp's with `of=ramp`)
among the objects of the `landing` kinds, none when none carries one;
`landing_count` is 1 or 0, and `end_width` the width such a landing is
compared with (the flight's, or a turning flight's tread meeting it).

The checks that are fixed-size searches count what they find:
`obstructed_end_spaces`, `landing_door_conflicts` (with `swing=yes` doors
swinging over too), `missing_tactile_strips` (on a whole stair's flights
with `within`, the path to it), `handrail_breaks` across a whole stair's or
a ramp's landings, and `rails_over_surfaces`. Each sums the items of the
search's list (`end_spaces`, `landing_doors` and `landing_swings`,
`rail_continuity`, `rail_obstructions`, below) with the sizes and kinds the
value states: an item found is a sure defect, one left undecided a
possible one, so the count is an interval from what was found to what was
found or left open, and `= 0` holds, fails or is not evaluated exactly as
the check does. `run_count` is how many sloped runs the service measures
of a ramp, cited as the service cites them. They are measured through the walking-surface
service as `stair-geometry` and `ramp-geometry` measure them, each a
flight measured first; the steps, runs and handrails are measured members
(below). Headroom, the clearance below and the clear width are
[clearances](#clearances).

### Plan spans

`centre_line_distance` is how far the centre line of a footprint's
least-area rectangle (`centre_line` `long`, `short` or `against-wall`) lies
from the nearest wall of the `walls` kinds beside it, square to it, within
`reach` and with the strip narrowed by `inset`: the nearer of the two
sides, or with `side=farther` the farther (every side has its wall within
it). Its interval runs from every wall that may lie there to the nearest
sure one, just past `reach` where a wall may but need not lie, and it is
none where none may. `plan_diameter` is the footprint's longest diagonal.
A light well's stacked spaces (the `members` path) give
`well_section_area` and `well_section_width` (the shared section, the
width none for an empty one), `well_height` and `well_gap`, the largest
gap between consecutive spaces. Recesses, corridor end walls and exit pairs
are measured members (below). Each is measured as `centre-line-distance`,
`exit-separation`, `recess-width`, `light-well` and
`corridor-end-openings` measure it, and expressions over them reach those
capabilities' verdicts on their fixtures (a recess whose depth straddles a
row boundary but is wide enough under either row passes as an expression,
where the capability leaves it open).

### Searches

Searches that must stay algorithms answer values the decision is taken
over. `travel_distance` is the longest walk from a start (`start`
`farthest-point`, or each of the `doors` of the `door_kinds`) to the
nearest exit (the `exits` path, of the `kinds`), along a walking profile
(`walking_height`, `walking_step`), walked exactly as `escape-route` walks
it: from the sure starts' least walk to every start's greatest, at most
`f64::MAX` where a walk is unbounded, and none where the space has no exit
or a start reaches none; walked with exact evidence, it is cited exact, as
`escape-route` cites it. "At most 20 m for offices, 35 m for labs" is one
expression rule per use row (or one `if`), and reaches `escape-route`'s
verdicts on its fixtures. Multiplied sections, common paths, compartments
and passages stay in the capability.

Whether a shape fits is `count` of the measured `free_placements` at least
1: the free-space service searches exactly as `free-floor-circle` and
`free-floor-rectangle` do, with their obstacles, band, merged spaces, door
swings and entrance path, and a search the selections leave open is one
undecided member, so the count straddles 1 and the fit is not evaluated.

### Measured members

Some measurements are many per object: a flight's steps, a ramp's runs, the
rails along either. An aggregate over a measured member list ranges over
them ([Expressions](./expressions.md#aggregates)); its `value` reads each
member's fields in `axioval:member` (`axioval_ir::MEMBER_SET`). The lists,
`axioval_ir::measured::MEASURED_MEMBERS`, are written like measured values
and declare their parameters and typed fields:

| List | Members | Fields |
| --- | --- | --- |
| `steps` (`walking_line_offset`) | one per riser of a flight, bottom to top | `riser`, `going`, `step_length` (`2r + g`), `nosing`, `winder_angle`, `open_riser` |
| `runs` (`landing`) | a ramp's sloped runs, lowest first | `run`, `slope`, `length`, `rise`, `width`, and with `landing` `bottom_landing`, `top_landing` and their `_depth` and `_width` |
| `handrails` (`rails`, `reach_across`, `reach_above`, `level_over`, `from`, `of`) | each rail along a flight or each run of a ramp | `run`, `left`, `right`, `height_lowest`, `height_highest`, `extension_bottom`, `extension_top`, `bottom_rise`, `top_rise`, `first_on_side`, `last_on_side`, `gap_after` |
| `face_pieces` (`face`, `direction`, `tolerance`) | the planar or smooth pieces of a face of the body, as the vertical-extent service lists them; with `face=facing`, a piece whose facing is undecided is a possible member | `slope`, `area`, `gradient_direction` |
| `recesses` (`requirements`) | the pockets between a footprint's boundary and its convex hull | `place`, `width`, `depth`, and with `requirements` the first row whose depth range holds the recess (`row`, `null` for none, undecided where the depth straddles a row's bound) and the width it requires (`required`) |
| `wall_spacing` (every `wall-spacing` parameter under its own name) | what `wall-spacing` judges of a storey | `close`, `distance`, `apart`, `pair` (a pair surely parallel and facing); `doubt`, `spacing` (undecided, why some pair may stand closer); `cover`, `uncovered` (undecided for why it cannot be measured), `what`, `unknown`, `related` (a footprint) |
| `well_gaps` (`members`) | each pair of consecutive spaces of a light well, ordered by their bottoms | `gap` (none below zero), `below`, `above`, `members` |
| `well_requirements` (`members`, `requirements`) | one item: the plan section a light well's spaces share and the row of `requirements` its height selects | `count`, `area` (`null` for no shared section), `width`, `height`, `row`, `required_area`, `required_width`, `members` |
| `centre_line_sides` (`walls`, `centre_line`, `sides`, `reach`, `inset`) | each side of the centre line a `centre-line-distance` rule judges, or the nearer of the two | `label`, `distance` (from every wall that may lie there to the nearest sure one, or just past `reach`; `null` for none within it), `lower`, `sure`, `wall` |
| `sight_view` (`targets`, `blockers`, `eye_height`, `radius`) | one item: the targets within the radius in view from an eye above the component, as `component-visibility` sees them | `visible` (from those surely in view to every one that may be), `sure`, `hidden`, `seen`, `found`, `within`, `undecided` |
| `containment_counts` (every `containment` parameter under its own name, `selection`; of the project) | each outer element's count where the rule bounds counts, and each object `containment` leaves open beyond its inner elements | `count_checked`, `object`, `count`, `held`, `may`, `related`; `open_checked`, `open` (undecided for why), `reason` |
| `containment_items` (the same arguments) | what `containment` reads of an inner element, ordered as it reported what it left open | `orphan_checked`, `orphan` (undecided for why), `orphan_words`; `band_checked`, `distance` (undecided for why it could not be measured), `low`, `high`, `below_words`, `above_words`, `straddle_words`, `related`; `open_checked`, `open` |
| `coordinate_differences` (`reference`, `length`, `angle`, `scale`, `require_map`), of a source | each statement in which the source's coordinate system departs from the reference's, or cannot be compared, then the georeference ([Coordinate systems](#coordinate-systems)) | `found`, `finding`, `recorded` |
| `end_walls` (`corridor`, `kinds`) | the walls the ends of the corridors an opening faces run into | `gap`, `facing` |
| `exit_pairs` (`exits`, `kinds`, `between`) | every pair of a space's exits | `separation` |
| `exit_separation` (every `exit-separation` parameter under its own name) | an item counting a space's exits (with `minimum_exits`) and one with the separation of their pairs, as `exit-separation` measures them | `counted`, `exits`, `sure`, `possible`, `relation`, `undecided`, `named`; `separated`, `separation` (the greatest or least pair, `null` where none is measured), `required` (an interval over both shares where the flag is unknown), `short`, `undecided_fail`, `undecided_pass`, `open`, `requirement`, `failed`, `related` |
| `name_sequence` (every `name-sequence` parameter under its own name) | the members an anchor reaches, in order, as `name-sequence` reads them | `member`, `shown`, `set`, `whole`, `value` (`null` where it states none), `previous` (`null` for the first), `expected`, `above`, `below` |
| `numbering` (every `numbering-consistency` parameter under its own name, `selection`) | what `numbering-consistency` judges of an object's number among those of its scope | `unread`, `number` (refused, for why it cannot be read), `prefixed`, `lead`, `departs`, `stepped`, `step`, `fillable`, `shown`, `below`, `missing`, `related` |
| `free_placements` (`shape`, `diameter`, `width`, `length`, `height`, `obstacles`, `band_from`, `band_to`, `merge`, `swings`, `entrance_width`, `access`, `doors`, `openings`) | a placement of the shape on a space's free floor: one found, none possible, or one undecided | none |
| `guard_edges` (`barrier_gap`, `platform_gap`, `landing_gap`, `landing_width`, `climb_distance`, `climb_side`, `measure_from` or `from_curb`, `climb_height`, `barriers`, `landings`, `climbables`, `surfaces`) | the exposed edges of a walking surface, as the guard service samples them; with `surfaces`, measured together with the others in one request | `guarded_height`, `tallest_barrier`, `tallest`, `tallest_top`, `partial_height`, `barrier_share`, `landing_fall`, `nearest_gap`, `nearest_fall`, `nearest_width`, `nearest`, `climbable_height`, `climbable` |
| `axes_within` (`of`, `reach`) | the objects of the kinds named within reach of the footprint in plan | `angle`, `centre_angle` |
| `parking_bay` (every `parking-bay` parameter under its own name, `selection`) | what `parking-bay` judges of a bay, in its order | `counting`, `count`, `allowed`, `found_words`, `open_words`, `related`; `sized`, `size` (undecided for why it cannot be measured), `measured`, `low`, `high`, `suffix`, `doubtful`, `applies_why`, `later`; `judged`, `found`, `message` |
| `parallel_pairs` (`members`, `member_path`, `angle_tolerance`, `reach`) | the parallel pairs of members the object reaches, as `wall-spacing` pairs them | `distance` |
| `swing_spaces` (`path`, `kinds`) | the spaces a door opens onto, probed as `door-swing` probes them | `into`, `away` |
| `connected_spaces` (`host_path`, `host_selector`, `external_property`, `space_path`, `space_selector`) | one item: an element's host walls and their exposure, how many spaces it relates to against how many the exposure needs, and whether the derived adjacency places them on the faces it needs, as `opening-spaces` reads them | `relates`, `count`, `possible`, `expected`, `known`, `sides`, `placed`, `wall`, `hosts`, `requirement`, `related` |
| `zone_checks` (every `opening-zone` parameter by its own name, `openings`) | what `opening-zone` checks of an opening in each host its path reaches, in its order, host by host: the placement, the allowed zone it misses least, each row of `dimensions`, each requirement on the supports, and the nearest other opening closer than `opening_spacing` | `check`, `placed`, `host`, `inside`, `outside`, `end`, `edge`, `bottom`, `top`, `far_open`, `zone`, `needed`, `distance`, `minimum`, `maximum`, `slack`, `words`, `required`, `label`, `open`, `fails`, `message`, `spacing`, `related`, `reason` and the words findings show |
| `limit_row` (`limits`, `key_1`…`key_4` and their paths, `pair_key`, `case_sensitive`) | one item: the row of `limits` the object's keys select, as `keyed-limit` selects it, or that none matches; refused where the keys cannot decide it or rows tie | `listed`, `row`, `keys`, `related` |
| `limited_values` (the row's arguments, `quantity` and every parameter a quantity reads) | what the row bounds, with its bounds: one value of the object, the sill height above each floor beside a window, or the step onto each floor a side of a door may step onto; none where no row with a bound applies | `what`, `unit`, `value`, `row`, `keys`, `minimum`, `maximum`, `side`, `sure`, `named`, `related` |
| `space_connections` (`connections`, `access_path`, `door_selector`, `opening_selector`, `space_selector`) | each requirement of each row of `connections` whose `from` picks the space, its access then its exit, as `space-connection` reads them | `row`, `access`, `required`, `kind`, `via`, `links`, `linked`, `related` |
| `opening_placements` (`host_path`, `hosts`, `length_axis`, `height_axis`, `zone`, `minimum`) | an opening's placement in each host its path reaches, as `opening-zone` places it | `inside`, `end_distance`, `edge_distance`, `bottom_distance`, `top_distance` |
| `clearances` (`of`, `side`, `obstacles`) | the headroom above a flight or ramp, or the clearance below it over the floors of the spaces `obstacles` names: one item | `clearance` (`null` where nothing governs it), `governing`, `noun` |
| `landings` (`of`, `walking_line_offset`, `landing`) | the landing at each end of a flight, or at both ends of each of a ramp's runs | `label`, `noun`, `present`, `depth`, `width`, `carrier`, `carriers`, `walking`, `outermost`, `scale` |
| `clear_widths` (`of`, `walking_line_offset`, `obstacles`, `band_from`, `band_to`, `landing`, `ends`, `stretch`) | the clear width of a flight and the landings at its ends, or of each of a ramp's runs | `label`, `above`, `width`, `governing`, `place` |
| `handrail_stretches`, `rail_heights`, `rail_extensions`, `rail_gaps` (`of`, `walking_line_offset`, `rails`, `reach_across`, `reach_above`, `level_over`, `from`) | each stretch (a flight, or a ramp's run) and the sides its rails run along; each rail's height; each end extension of the handrail along a side and of each rail over the middle; each gap between consecutive pieces | `stretch`, `label`; `measured`, `sides`, `side`, `on_sides`, `width`; `rail`, `rails`, `lowest`, `highest`; `end`, `other`, `from`, `place`, `reach`, `rise`, `over_middle`; `pair`, `gap`; `scale` |
| `end_spaces` (`of`, `obstacles`, `depth`, `width`, `height`), `landing_doors`, `landing_swings` (`of`, `landing`, `doors`, `height`), `rail_continuity` (`rails`, reaches, `tolerance`, `gap`), `rail_obstructions` (`rails`, `surfaces`, reaches) | one search per end, per landing, per broken side or per rail over a surface | `found`, `finding`, `objects`, `label` |
| `flights` (`walking_line_offset`) | the flight itself | `rise`, `width`, `scale`, `turning` |
| `tactile_strips` (`walking_line_offset`, `tactiles`, `offset`, `depth`, `stair`, `path`, `flights`) | one search per end of a flight | `found`, `finding`, `objects`, `label`, `intermediate` (the end lies between two of the stair's flights) |
| `stairs`, `stair_continuity`, `stair_clear_widths` (`walking_line_offset`, `path`, `flights`, …) | a whole stair: itself; each side broken across a landing between its flights; each flight's and intermediate landing's clear width | `rise`, `scale`, `missing`, `complete`; the search's fields; `width`, `owned`, `governing`, `owner` |

The stair and ramp lists are what `stair-geometry` and `ramp-geometry`
judge item by item ([Capability templates](./templates.md)). A text field
(`label`, `carrier`) words the item as a message names it, and an objects
field (`governing`, `carriers`, `objects`) names the objects a finding on
the item relates; read in an expression, either is text, the objects'
identities joined by `, `. `scale` is the largest magnitude among the
positions measured, which the binary rounding of their coordinates grows
with, and the runs' `slope_rounding` how far that rounding may move a
run's slope. A search's item states `found` true with what it found
(`finding`), false, or undecided with why; a search over a selection that
leaves objects undecided treats them as possible finds. A list that names
the rule's selectors and lengths (`@landing_objects`) is measured once per
object, however many checks read it.

Step `j` climbs riser `j` onto tread `j`; its `going`, `nosing` and
`winder_angle` are measured from the tread below, so the first step and a
final riser have none (`null`), which an expression guards with
`isDefined` where it should skip them. A field the measurement cannot decide (an
unmeasured winder angle, whether a riser is closed, the order of pieces
lying beside one another) leaves an expression reading it not evaluated.
So does a rail that may reach over the middle of the walking surface or lie
wholly in either half: its `left`, `right`, `first_on_side`,
`last_on_side` and `gap_after` are undecided, never false or none. Only a
rail surely over the middle runs along neither side (`false`, and no gap).
A `runs` list without `landing` kinds leaves every landing field
undecided, its depth and width too.
Built-in code registered beside the capabilities measures each list
(`MeasuredProvider::member_lists`), never a package. An aggregate over a list cites the measurement the list comes from
(`MeasuredProvider::members_cited`), even when it lists none. A member states
whether the measurement it comes from is exact: its fields' evidence is
then exact, an interval holding only the rounding of exact arithmetic, as
the capability measuring it cites it.

`opening-zone`'s margins are one `none` over `opening_placements`: no
placement outside its host, or inside it nearer an end or edge than
`end_distance` and `edge_distance` (zero with `zone=web`, below which an
opening reaches into a flange) or farther from an edge than
`edge_distance_maximum`. A distance measured to a free outline from the
box an opening may lie in is a lower bound and is stated as the interval
up to the host's extent, so it passes but never finds; a placement that
may be below `minimum`, or a host among several whose body cannot be read,
states every field undecided, and an opening none of whose placements can
be read is not measured. These reach the capability's verdicts on every
margin fixture; spacing, zones, dimensions and supports relate an opening
to others and are not fields of one placement.

"Every riser at most 0.19 m, and risers within 5 mm of one another":

```json
{"kind": "and", "operands": [
  {"kind": "aggregate", "function": "all", "over": {"kind": "measured", "name": "steps"},
   "value": {"kind": "compare", "operator": "lessThanOrEquals",
     "left": {"kind": "round", "operand": {"kind": "property", "propertySet": "axioval:member", "property": "riser"},
              "step": {"kind": "literal", "value": {"type": "quantity", "value": 0.001, "unit": "m"}}},
     "right": {"kind": "literal", "value": {"type": "quantity", "value": 0.19, "unit": "m"}}}},
  {"kind": "compare", "operator": "lessThanOrEquals",
   "left": {"kind": "subtract",
     "left": {"kind": "aggregate", "function": "max", "over": {"kind": "measured", "name": "steps"},
              "value": {"kind": "property", "propertySet": "axioval:member", "property": "riser"}},
     "right": {"kind": "aggregate", "function": "min", "over": {"kind": "measured", "name": "steps"},
               "value": {"kind": "property", "propertySet": "axioval:member", "property": "riser"}}},
   "right": {"kind": "literal", "value": {"type": "quantity", "value": 0.005, "unit": "m"}}}]}
```

An exposed edge's fields leave out the thresholds a standard varies by
use: `guarded_height` is the greatest height such that barriers at least
that tall (within the gaps) cover the whole edge, and `landing_fall` the
least fall such that landings no deeper (wide and close enough) cover it.
`horizontal-guard` finds an edge guarded exactly when `guarded_height`
reaches the barrier height and no `climbable_height` is low enough to
defeat it, or else, with barriers reaching at most half of it
(`barrier_share`), when `landing_fall` is within the fall allowed; an
expression over the edges states that per use class and reaches the
capability's verdicts on its fixtures.

Expressions over these values reach `stair-geometry`'s and
`ramp-geometry`'s verdicts on their fixtures, check by check: risers,
goings, step lengths, the riser count and spread, the rise, the width,
landings, winders, open risers, ramp slope limits and spread, handrail
heights, extensions, gaps and sides.

### Parking bays

A bay's own measurements are measured by `parking-bay`'s own code, so the
values and the capability never disagree:

- `rectangle_side;side=width|length` is the shorter or longer side of the
  footprint's least-area rectangle; its height is `extent_z`. A footprint
  with several least-area rectangles, or a tessellated one, is not
  evaluated.
- `obstruction_count;obstacles=<kinds>;reach=<m>;at=…` counts, among the
  objects of the kinds within `reach` of the footprint in plan, how many of
  the two `ends` (across the long axis) or the two `sides` (along it) they
  obstruct, or how many obstacles stand `within` the rectangle, past none of
  its edges. With `side_zone=<m>` a side counts only when an obstacle
  overlaps its central stretch that long. The count is an interval from the
  edges surely obstructed to those possibly obstructed; an obstacle that
  cannot be placed may obstruct any edge. A rectangle without a long axis
  (a square) has no ends or sides once an obstacle is near.
- `axes_within;of=<kinds>[;reach=<m>]` lists the objects of the kinds within
  `reach` of the footprint in plan (default 0, meeting it), each with the
  acute `angle` between the long axes and the `centre_angle` between the
  footprint's long axis and the direction to the member's centre, both in
  `[0, π/2]`; one only possibly within reach is an undecided member, one
  without a readable extent or long axis has undecided angles, and centres
  too close to tell a direction leave `centre_angle` undecided.

The orientation to an aisle is then an expression with a tolerance:
"perpendicular to an aisle" is `any` over `axes_within;of=aisle` of
`angle ≥ 85°`, parallel `angle ≤ 5°`, angled both strictly between; a size
bound for perpendicular bays only is an `implies` whose antecedent is `all`
over the same list. Expressions over these values reach `parking-bay`'s
verdicts on its fixtures, check by check: width, length and height bounds,
the orientation to the aisle within a reach, obstructed ends and sides
with and without a side zone, an obstacle within a bay, and size bounds
filtered by orientation or obstructed sides. A bay's orientation inferred
from neighbouring bays (`neighbour_reach`) reads both angles, as
`parking-bay` does: a neighbour counts when its long axis is parallel
(`angle ≤ 5°`), and its `centre_angle` gives the state, perpendicular for a
neighbour beside the bay (`≥ 85°`), parallel for one end to end (`≤ 5°`);
the bay is in a state when some counted neighbour gives it and none gives
another. The long-axis `angle` cannot tell the two apart, since both
neighbours of a row and of a line are parallel to the bay.

### Parallel members

`parallel_pairs;members=<kinds>;member_path=<steps>;angle_tolerance=<degrees>;reach=<m>`
lists the pairs of members a storey reaches along `member_path` whose long
axes lie within `angle_tolerance` degrees (below 45) of parallel and that
face each other, exactly as `wall-spacing` pairs them, among the pairs
within `reach` of each other in plan (at least the widest spacing judged).
Each pair's `distance` is its plan distance, closest points to closest
points. A pair only possibly parallel or facing is an undecided member, and
a member whose extent cannot be read is an undecided member of undecided
distance, so `none` of the pairs closer than a minimum is a pass only when
no undecided pair may be.

`band_uncovered_area;…;maximum=<m>;footprints=<kinds>;footprint_path=<steps>`
takes the same members and the bands between parallel pairs at most
`maximum` apart, and measures the largest area of a footprint the storey
reaches along `footprint_path` that lies outside every band: at most what
the sure bands leave, at least what every possible band leaves, and at
least nothing while a member is undecided. A storey reaching no footprint
is not evaluated. Expressions over both reach `wall-spacing`'s minimum
spacing and coverage verdicts on its fixtures.

### Levels

`level_elevation`, `level_index` and `height_above_ground` place an object
among its source's levels. The level is the object itself, or the one level
its `path` reaches (a space up its aggregating storey,
`["IfcRelAggregates:backward"]`); a path reaching levels at different
elevations is a conflict, and one reaching none has no value. A source's
levels are its objects of that level's kind, at the elevations of their
placement origins, read through the object-frame service. The ground level
is the lowest at or above `datum` (default `0`, the source's own zero): its
`level_index` is 0, the levels above count up and those below count down
(levels at one elevation share an index), and `height_above_ground` is the
level's elevation above it. Each model of a federation counts its own
levels, so "on storeys more than 7 m above ground" is one expression,
`height_above_ground;path=IfcRelAggregates:backward > 7 m`, over every
model; restricted to one discipline it reads the source's
`axioval:source` `discipline` (see [Source facts](#source-facts)).

Level heights are measured as `level-spacing` measures them.
`level_rise;levels=<kinds>;order=<set/name>` is a level's rise to the next
level up: its `order` length (`Levels/Elevation`) less the next one's, among
the levels of its anchor (the one object the `anchor` path, a building to
its storeys, reaches back from the level) or, without `anchor`, of its
source. The object is the level, or the one its `path` reaches. With
`lowest=ignored` the lowest level has no rise; the highest has none with
`highest=ignored`, is measured from the highest top of what its `contents`
path reaches (of the `content_kinds`) less its order, and is left open
otherwise. `prevailing_rise` (same parameters, and `tolerance`, default
1 mm) is the rise most of the anchor's checked levels share, the lowest of
equally common ones, from exact rises only; none with fewer than two.
`prevailing_elevation;side=bottom|top;spaces=<steps>` is the bottom or top
elevation most spaces of a space's level share (the level its `spaces` path
reaches back from the space, the spaces of the `kinds` it reaches), from
exact elevations only, within `tolerance`; none for fewer than two spaces.
Expressions over them reach `level-spacing`'s verdicts on its fixtures: a
bounded rise, `abs(rise - prevailing_rise)` within the tolerance, each
space's `extent_z` against its level's rise (`path` up to the level), and
each space's `bottom` or `top` against the prevailing elevation, storeys and
spaces each judged by a rule of their own. A level whose anchor's levels
cannot be ordered is open in the rewrite, where the capability leaves the
anchor open.

`levels_above` and `levels_below` (`levels=<kinds>;path=<steps>`) count the
storeys of the object's source above and below its one storey by their
`Elevation` attribute, as `slab-contact` leaves out the top or bottom
storey: 0 on the top or bottom one. An object on no storey or several, or a
storey without a length elevation, is open.

### Stacks, shelving, extents and meshes

`stack_distance;measure=…;slabs=<objects>;ratio=<share>` pairs the slabs of
the kinds as `slab-stack-spacing` does (stacked when their footprints
overlap by at least `ratio` of the smaller, ordered by their tops) and
measures the rise from top to top, bottom to bottom, or the clear gap from
the slab's top to the next one's underside; none where nothing stacks above.
Whether two slabs stack, or which is the next one up, left open by the
intervals, and any slab of the kinds without an extent leave it open. A
bound on it, rounded to the micrometre, reaches each of the capability's
thresholds on its fixtures; consistency within a stack stays with the
capability.

`shelf_length` and `shelf_clear_height` are the running metres of shelving
the arrangement (`depth`, `horizontal`, `vertical`, `bottom`, `top`,
`clearance`, in metres) fits into a space, and the space's clear height,
from one request carrying the doors and openings the `access` path reaches
the space from among the `spaces` (every object without it), as
`shelf-capacity` measures them. `doors`, `openings` and `spaces` are source
kinds or a rule's selectors (`doors=@door_selector`); an element that may
reach the space unreadably, or whose kind the selector cannot decide,
leaves both open. Both cite the doors and openings sent. `shelf_clear_height`
at least `top` and `shelf_length` at least the minimum reach the
capability's verdicts on its fixtures.

`body_extent;axis=right|forward|up` is the body's depth along one of its own
placement axes, refused as `body-extent` refuses it (an unplaced object is
incomplete evidence), and cited as exactly as its frame and extent are.
`abs(body_extent - thickness)`, rounded to the micrometre, within a
tolerance, or the extent within a range, reaches the capability's verdicts
on its fixtures; a stated length that is absent fails, one that is no
length is invalid evidence.

`body_position;axis=…;end=low|high` is the position of the body's lowest
or highest point projected onto the same axis, measured with the extent and
refused for the same reasons. `body_extent` is the high less the low end;
the positions are the magnitudes the binary rounding of that difference
scales with, which the `body-extent` template's rounding allowance reads
(see [Capability templates](./templates.md)).

`triangle_count` is the host's count of the body's mesh. A count of a
tessellation is a point whose evidence is not exact
(`Measurement::Cited` with `exact: false`), as the capability cites it, so at most the
maximum reaches `triangle-count`'s verdicts and evidence.

### Subjects

A measured value is each object's own, unless the registry declares it a
source's or the project's (`MeasuredSubject`, the descriptor's `subject`):

- **A source's** value (`coordinate_shift`, `external_declarations`,
  `any_external`, `host_walls`, `undeclared_hosts`, `possible_hosts`,
  the member list `coordinate_differences`) read on an object is the value of
  the object's source, measured for the source
  (`MeasuredProvider::measure_source`, `members_of_source`) however many
  of its objects read it, and identified by the source's
  source-qualified identity. A template judging the sources themselves
  reads it for every source of the session in identity order, one holding
  no object included ([Capability templates](./templates.md)).
- **The project's** value (`coordinate_reference`, `envelope_size`) is
  one value of the run (`MeasuredProvider::measure_project`): read on an
  object, it is the same for every object, and a template reads it once
  per rule, its refusal leaving the whole rule open.

A provider may cite the sources a value was measured against
(`Citation::sources`: the reference source), which a template's message
names (`{reference:source}`). Since no object is measured, a value of a
source or the project never takes `@anchor`: the registry refuses it when
the name is read (`EngineError::InvalidMeasured` at compilation and in a
draft's diagnostics).

### Coordinate systems

The coordinate system of an object's source is compared with the reference
source's (the one of the discipline `reference` names, or the first in
identity order) as `coordinate-consistency` compares them:
`coordinate_shift` and `coordinate_turn` `of` the `world` frame or the
`site` placement, `coordinate_turn;of=north` for true north, and
`coordinate_shift`, `coordinate_turn` `of` the `map` conversion with
`map_scale_change` and `map_target_change` (1 where the target systems
differ). A statement only one source makes is open as incomplete evidence,
and so is a map offset in an inexact unit; a site or true north neither
states has no value; map comparisons with a source stating no map
conversion are open as not recorded. `map_conversion` is 1 where the source
(or, `of=reference`, the reference) states one. Every comparison within its
tolerance, the map comparisons last (so a missing map conversion is not
recorded only when nothing else is unknown), and with a required map
conversion `map_conversion` 1 and the map comparisons only where the
reference states one, reach the capability's verdicts on its fixtures; the
capability judges a source, the rewrite each object of it. Each is a value
of a source: read on an object it is its source's, and a template judges
the sources themselves.

`coordinate_reference` is the project's: how many sources are compared
with the reference source (the one of the discipline `reference` names,
or the first in identity order), which it cites. Fewer than two sources, a
discipline no source or several declare, or sources declaring none, leave
it not evaluated, as `coordinate-consistency` words them. The member list
`coordinate_differences` (`reference`, `length` in metres, `angle` in
degrees, `scale`, `require_map`) lists of a source each statement in which
its coordinate system departs from the reference's beyond the tolerances
(`found` true, its words `finding`: `map offset moved by 1.0000 m`) or
cannot be compared (`found` undecided, why), as `compare_coordinate_systems`
reports them and in its order, then the georeference: a missing map
conversion `require_map` asks for is a difference (`states no map
conversion`), and otherwise one only the reference or the source states,
or neither, is undecided and not `recorded`. The reference itself lists
only its own missing map conversion `require_map` asks for, and nothing
where its coordinate system cannot be read.

### Building envelopes

`envelope_size`, `on_envelope`, `declared_external` and `bounds_envelope`
read one derivation of the building envelope (`derivation`: `all-spaces`
around the `bounding` objects, `gross-area-groups` around the members the
`groups` reach along `group_path`), asked of the envelope-membership
service once per run for its bounding objects (`EnvelopeMembershipRequest`).
A bounding selection that leaves an object undecided or picks none, groups
reaching no member, and a service refusing the derivation leave each value
not evaluated, as `external-wall-validation` words them.
`envelope_size` (the project's) counts the objects placed on the
envelope; `on_envelope` is 1 where geometry places the object on it
whatever the model declares, `declared_external` 1 where the model
declares it external (not evaluated where it states neither external nor
internal, or its body could not be measured) and `bounds_envelope` 1
where the envelope is derived around it. `external_declarations` (a
source's) counts the source's `objects` (those surely picked) declared
external in any of `derivations` that can be derived: an interval up to
those whose declaration is unknown in one.

### Alignments

Infrastructure limits are stated against an alignment: a pier within a
station range, a mast far enough from the track axis, a soffit high enough
above the gradient line. `station`, `offset` and `height_above_gradient`
locate an object's reference point, its placement origin, along an
alignment through `AlignmentServiceHandle::measure_alignment_position`
(`AlignmentRequest`: the object and the alignment, never the same object).
The alignment is a 3D centreline: a plan measured by its own length from
its start and a gradient line giving the height along it. The point's
**foot** is where the plan perpendicular from it meets the plan curve, the
nearest one where there are several:

- `station` is the label of the plan distance from the start to the foot,
  through the station equations the alignment states (the distance itself
  where it states none);
- `offset` is the plan distance from the foot to the point, positive to
  the left of the direction of travel (the lateral axis of the section
  frame: tangent, left, up); `side=right` counts the right positive;
- `height_above_gradient` is the point's elevation above the gradient line
  at the foot.

`alignment` names the alignment's source kinds (`IfcAlignment`; subtypes
match). The alignment is the one object of those kinds that `path` reaches
from the object or, without a path, the one in the object's source. None
selected is an exact absence; several are refused, never picked from.

The parameters read the alignment at the stretch of plan distances the
foot may lie in (`AlignmentParameterRequest`,
`AlignmentServiceHandle::measure_alignment_parameter`): `alignment_curvature`
is the plan's signed curvature per metre (positive turning left, zero on a
straight), `alignment_radius` its unsigned inverse (a length, refused
where the alignment may run straight there, since the radius is then
unbounded), `alignment_gradient` the gradient line's rise over plan run
(positive rising in the direction of travel), and `alignment_cant` how far
one rail head stands above the other, unsigned. An alignment stating no
cant has none: an exact absence, never a zero. The trait's
`measure_alignment_parameter` refuses by default, so a service that reads
no parameters never answers with a straight, level, uncanted line.

Every value is an interval sure to hold the exact one (`AlignmentInterval`),
and its evidence is exact exactly when every interval is a point; locating
a point on a curve is a numerical search, so a position practically never
is. The handle refuses an answer about another request. A position is not
evaluated with the service's reason when:

- the point's nearest foot lies before the alignment's start or beyond its
  end (`AlignmentError::OffRange`): it is off the range, never clamped to
  an end;
- two feet are equally near within the measurement's bounds, or the
  nearest cannot be decided (`Ambiguous`), such as a point at the centre
  of an arc;
- the object has no reference point the service holds (`UnknownObject`),
  the object named is no alignment (`NotAlignment`), or the alignment, its
  stationing or the point's placement cannot be read (`Unavailable`).

Without an alignment service the values are a missing service, reported
once per rule and source. The CLI's IFC implementation is described under
[Geometry](./cli.md#geometry).

#### Sections and clearance envelopes

Clearance profiles, member thickness at a station and cover in a section
are checked in the plane normal to an alignment. The **section** at a
station is the vertical plane through the centreline point normal to the
alignment's plan there; its coordinates are `lateral`, horizontal and
positive to the left of the direction of travel, and `up`, vertical, from
the gradient line, so a point of the section at `(lateral, up)` has that
`offset` and that `height_above_gradient`. Cant does not turn the frame:
an envelope that tilts with the cant is stated tilted.

`AlignmentServiceHandle::measure_section` answers a `SectionRequest` (the
alignment, a station, the bodies, never the alignment itself) with one
`BodySection` per body, an **interval region**: the oriented segments of
the body's mesh cut by the computed plane (the region to their left), the
band of every piece of the mesh within `radius` of the plane projected onto
it, and that `radius`, which covers the mesh's certified deviation and the
plane's numerical error. A point lies **surely** in the section when it is
in the cut and farther than the radius from the band, **possibly** when it
is in the cut or within the radius of the band. The engine derives the
area (`area`: the cut's area less or plus the band grown by the radius)
and the reach along `lateral` or `up` (`extent`: from the spread of points
proven inside to the spread of everything possibly inside) from that
region, never accepted from an adapter; a plane missing the body surely has
no extent. Evidence is exact exactly when every radius is zero.

- `station_section_area;alignment=<kinds>[;path=<steps>];station=<m>` is
  the area of the object's own body in the section at the station (an
  area; zero where the plane misses it);
- `station_section_thickness;…;station=<m>[;direction=up|lateral]` is how
  far that section reaches vertically (the default) or horizontally; none
  where the plane misses the body.

The station is the alignment's label; the service carries it to a plan
distance through the station equations, and a station no stretch or
several stretches carry is refused, as is one before the start or beyond
the end. A body that is unmeasured, whose mesh is no closed solid, or that
is measured through its parts (whose overlaps a cut would count twice) is
refused with the reason.

`envelope_intrusions;bodies=<kinds>;envelope=<polygon>;from=<m>;to=<m>;step=<m>`
is measured on an alignment. The **clearance envelope** is a simple
polygon in section coordinates (`SectionPolygon`), for example
`envelope=-2:0,2:0,2:5,-2:5`, and swept from station `from` to station
`to` it is the volume of every point at `(lateral, up)` in it in the
section at any distance in between. The bodies are the objects of the
`bodies` kinds in the alignment's source (subtypes match).
`AlignmentServiceHandle::measure_envelope` answers an `EnvelopeRequest`
with one `Intrusion` per body: `Sure` with the distance where a point of
the body was proven inside the envelope, `Clear` when the body was proven
out of the whole swept volume, `Possible` with the stretch and reason
otherwise. The value is the count from the sure to the possible
intrusions, so a rule `envelope_intrusions… = 0` fails on a sure intrusion,
is not evaluated while one is only possible, and passes only when every
body is proven clear. The evidence names the intruding and the undecided
bodies (the first five of each). No body of the kinds is a count of zero; an
unmeasured body or one that is no closed solid refuses the value with the
reason, since it could hide an intrusion.

The sweep is sound between its samples: `step` is the declared plan
distance between sampled sections, but between two of them the service
bounds how far any point of the envelope can move and tests the body's
mesh in 3D against the envelope grown by that bound over the step, so a
body crossing the envelope between two samples, however thin, is found or
left possible, never passed. A step too coarse to decide is halved a
bounded number of times; what stays undecided leaves the body possible with
the reason. The trait's `measure_section` and `measure_envelope` refuse by
default, so a service that cuts nothing never answers with empty sections
or clear envelopes.

### Stated rather than measured

`level_height` is a storey's height to the next storey of the same spatial
parent, which the IFC adapter states: storey elevations are the world
heights of their placements (in metres through the project's length unit),
siblings are the storeys the same `IfcRelAggregates` parent aggregates, and
the highest storey has none (an exact absence). A tilted storey, a sibling
at the same elevation or unreadable, and a storey aggregated twice are
refused. The engine forwards the name to the host's property resolver.

Exactness is stated by the measurement, never inferred from a point: a
tessellation measures points too. A value the engine measures itself is
exact exactly when the geometry service's evidence is (and, for a thickness
square to a face, the face's normals'). Built-in code states it by the
variant it answers: `Measurement::Value` is never exact, a point included;
`Measurement::Rounded` is exact, its interval holding only the rounding of
exact arithmetic; `Measurement::Cited` is exact as its `exact` says, as the
capability measuring it cites its own evidence; and a member is exact as
its `exact` says, unless a field is cited inexact itself. An exact point is
a quantity (or a number) with exact evidence. Anything else is a `measured`
value, an interval sure to hold the exact value, its evidence exact only
where the measurement states it. Sums, differences and shares of measured
values round outward, so the interval holds the exact result; a sum of
float lengths is never a point claimed exact. A count found on inexact
evidence is cited as such:

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

## Source facts

The set `axioval:source` (`axioval_ir::SOURCE_SET`) states what an evidence
session knows about an object's source, as text (several values as a list):
`discipline` (`SOURCE_DISCIPLINE`), the discipline the session declares for
the source, and the metadata the `source` selector reads, `fileName`,
`application`, `schema`, `project` and `timestamp`. A source with no declared
discipline has none (an exact absence); a metadata field the source never
records is not recorded. Outside an evidence session the set is a missing
service. Any other name in the set is refused when the ruleset compiles.
Expressions read it like any property, so a rule over a federation can
judge each discipline on its own:

```json
{"kind": "implies",
 "antecedent": {"kind": "compare", "operator": "equals",
                "left": {"kind": "property", "propertySet": "axioval:source", "property": "discipline"},
                "right": {"kind": "literal", "value": {"type": "string", "value": "architecture"}}},
 "consequent": "…"}
```

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
- A number with a unit is a quantity: exact when it is a point computed from exact reads only, otherwise a `measured` interval (a point computed from an approximation included), whose evidence is never exact. Text, enumeration values, truths, dates and date-times are themselves, and `null` is an exact absence. An interval of plain numbers is read only by expressions; anywhere else it cannot be read. A value that cannot be computed leaves its reader not evaluated with the reason.
- Its evidence is located at `axioval:value/<name>` and cites the locators of every read it was computed from.

A decimal literal states a value as a source does: `30 mm` is the double nearest 0.030 m, the same one a source stating `0.030` holds, so a cover of exactly 30 mm meets a 30 mm bound rather than straddling it.
