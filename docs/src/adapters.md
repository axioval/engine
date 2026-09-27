# Independent adapters

Adapters are peers around the source-neutral engine. No adapter receives special privileges.

## IFC

`axioval-ifc` provides a production IFC2X3 and IFC4 STEP path for exact direct properties:
strict bytes become a SHA-256 fingerprinted `EvidenceSession`; IFC objects become
source-qualified Axioval objects; and `ifc-properties::exact_property` backs the
session's property service with occurrence/type provenance and exact absence.
The property resolver owns and declares the same source/revision/fingerprint/schema
snapshot registered by the session; mismatched service composition is rejected.
Parser diagnostics, unsupported schemas, malformed traversal, conflicts, and
unsupported values fail closed.

A present property reports the IFC type its value was written with (`IFCLABEL`,
`IFCBOOLEAN`, ...) as `Property::data_type`, upper case whatever the file used.
A value is carried when its defined type's base in the file's release maps without
loss: every `STRING`-based type (`IfcLabel`, `IfcDuration`, ...),
`INTEGER`- and `NUMBER`-based integers (`IfcCountMeasure`),
`IfcBoolean`, and `IfcReal` or dimensionless `NUMBER` reals. Other real-valued
measures are converted to SI through their unit (see *Measures*).

Date and time types are read as dates, in property sets and attributes alike, and
keep their declared type and the same evidence locator as any other value:
- `IfcDate` (`YYYY-MM-DD`) is a `Date`;
- `IfcDateTime` is a `DateTime` when it states a UTC offset (`2026-09-27T10:00:00+02:00`
  or `...Z`). Without one its instant is unknown: the property is refused as
  incomplete (`PropertyResolutionError::Incomplete`, naming the value), never read
  in a guessed zone and never downgraded to text;
- `IfcTimeStamp` (seconds since 1970-01-01, UTC; IFC2X3 and IFC4) is a UTC `DateTime`.

Text that is not the type's ISO 8601 form, or a day that does not exist, is an
invalid value (`PropertyResolutionError::InvalidValue`).

Quantity sets (`IfcElementQuantity`) resolve like property sets
(`ifc-properties` 0.4.0, openbimrs/ifc#66): `Qto_SpaceBaseQuantities.NetFloorArea`
is an area quantity in SI. So do the attributes of predefined property sets
(`ifc-properties` 0.4.1, openbimrs/ifc#149) such as `IfcDoorLiningProperties`,
`IfcDoorPanelProperties` and their window counterparts: a set is found by its
`Name`, or by its entity name when it states none, and `LiningDepth` is a length
in SI with its declared type (`IFCPOSITIVELENGTHMEASURE`). An enumeration
attribute (`PanelOperation`) is its constant as text, typed by its
enumeration (`IFCDOORPANELOPERATIONENUM`), and an unset optional attribute is
`null` with its declared type.

The other simple property kinds (`ifc-properties` 0.4.1, openbimrs/ifc#150)
map onto the IR's composite values, every scalar in them read as a single
value of its declared type, with the unit the kind states for it:
- an `IfcPropertyEnumeratedValue` is its selected item, a `List` of them when
  several are selected, and `null` when none is;
- an `IfcPropertyListValue` is a `List`, `null` when it states no element;
- an `IfcPropertyBoundedValue` is a `Bounded` value (lower and upper bound,
  and in IFC4 the set point), `null` when it states none of them;
- an `IfcPropertyTableValue` is a `Table` of its defining and defined values,
  each column in its own unit, `null` without rows.

The declared type is the one every scalar of the value declares; a table
whose columns declare two types reports none. The request is refused, never
answered as absent, when:
- a complex quantity or two quantities carry the requested name;
- a property set and a quantity set share the requested set name;
- the requested attribute of a predefined set is an entity reference;
- the value is an `IfcPropertyReferenceValue`, which names an entity the IR
  cannot carry, or an `IfcComplexProperty`.

Any other absence is exact.

The session also enumerates an object's properties through
`exact_properties_where` (`ifc-properties` 0.4.1, openbimrs/ifc#78): the
traversal, validation and refusals are those of `exact_property`, so an
exact set and name enumerate what they resolve, and an empty answer is a
proof. Quantities and predefined-set attributes are enumerated like
properties, an occurrence value overrides an inherited one of the same set
and name, and ambiguity is refused per source. Only selected members must
be readable: an unselected complex property does not refuse the answer, a
selected reference value does. A set of a reserved name is never selected.

Direct-property completeness does not imply relationship completeness. The IFC
session registers an exact relationship-selection service: a relationship
identity is the entity name, in the source's own release, of an objectified relationship type (for
example `IfcRelContainedInSpatialStructure`, `IfcRelVoidsElement`), and a
supertype such as `IfcRelConnects` covers every concrete subtype present. End
slots are read from the bundled normative schema, not a hand-written table.
Every answer carries a scan locator naming the type and instance count, which is
what makes an empty selection exact. A malformed instance, a dangling reference,
a relationship type whose ends are not object references (such as
`IfcRelDefinesByProperties`), or an object from another source refuses the whole
answer rather than dropping an edge. The adapter does not depend on Axiolid and
does not own geometry policy.

An instance that leaves a schema-required end empty (`$`) is handled
separately, because real exporters do it routinely: second-level virtual space
boundaries often carry no `RelatedBuildingElement`. By default such an instance
also refuses the answer, since its missing edge could touch any object. A
request built with `with_absent_ends(AbsentEndPolicy::Skip)` answers from the
edges that exist instead and cites every skipped instance as
`relationship-absent-end:<instance>:<attribute>` evidence. The capability
parameter `skip_absent_relationship_ends` opts a rule in; the default stays
strict.

The session also registers a `SourceIntegrityServiceHandle`. Its scan uses the
same end reader, so it always lists exactly the instances a strict request
refuses on: an absent required end is a `relationship.absent-required-end`
**warning**, while a dangling or wrongly shaped end is a
`relationship.malformed` **error**. Hosts show these beside the report whether
or not any rule skipped them.

The session binds to the one release the file header declares: IFC2X3 TC1
(`IFC2X3_TYPE_SYSTEM`) or IFC4 ADD2 TC1 (`IFC4_TYPE_SYSTEM`). A header naming
any other release, several releases, or none is refused rather than read with
the wrong tables. The session declares that release's type system on its
snapshot, so package concepts bind to IFC names (see
[Concept binding](./concept-binding.md)), and registers that release's entity
inheritance from the bundled normative schema for `includeSubtypes`.

### Classifications

The session registers a classification service backed by
`ifc-classification`. For each object it returns every assignment, direct or
inherited from the object's type, as the classification system's name and
the chain of codes from the assigned item up to the root. A selector matches
the leaf code, or any code in the chain when `includeDescendants` is set;
a `codePattern` matches codes the same way, and a selector without a code
matches any assignment in its system.

IFC2X3 hierarchies are flat: a reference's `ReferencedSource` may only name
the system itself. A file that chains references anyway is refused rather
than flattened. An assignment whose system the file does not state is
neither a match nor a mismatch, and the object is reported as not evaluated.

### Source metadata

The session states what the file says about itself for `source` selectors
(see [Source selectors](./capabilities.md#source-selectors)): `application`,
the `ApplicationFullName` of the `IfcApplication` every `IfcOwnerHistory`
names as `OwningApplication`, and `project`, every `IfcProject.Name`. Both
are read over the whole file, so a file without owner histories states no
application, exactly. An owner history whose application is not an
`IfcApplication`, or a name that is not text, leaves the field unread, so a
selector over it is not evaluated rather than a non-match. The schema comes
from the snapshot; the file name is the host's to state.

### Attributes

The property service answers the reserved attribute sets from the entity
itself. In `axioval:attributes`, the property name is the attribute's name in
the file's release schema, matched ignoring ASCII case: `Name` is an
`IfcSpace`'s number, `LongName` its name, and `PredefinedType` its
enumeration. `axioval:type-attributes` reads the same attributes from the
type object `IfcRelDefinesByType` assigns, so `Name` there is the
construction type. The evidence locator names the instance
(`attribute:#12:LongName`), or the relationship and the type object
(`type-attribute:#40:#30:Name`).

- Text, enumeration, boolean, integer and unit-free real values are
  answered.
- An unset attribute (`$`), an attribute the entity does not declare, and an
  object without a type are exact absences.
- An object typed by two type objects is a conflict.
- A measure such as `IfcBuildingStorey.Elevation` is converted to SI with
  the project's default unit, as described under *Measures*. References,
  aggregates and derived values are refused.

### Measures

A measure is a number in a unit: the explicit `Unit` of a property,
otherwise the project's default unit of its kind. Property values and
attributes of a measure type go through `ifc_properties::exact_unit`
(0.3.0). It resolves the effective unit to an exact SI scale, with an offset
for degrees Celsius, and follows conversion-based and derived units. The
value becomes a quantity in SI (`240` mm reads as `0.24` m); a ratio or
count becomes a plain decimal. A measure whose unit cannot be resolved is
refused rather than read as a bare number, and so is a plain scalar
(`IFCREAL`, `IFCINTEGER`) that carries a unit. Causes include no
`IfcProject`, no project unit of the needed kind, or an ambiguous
assignment.

### Presentation layers

`axioval:presentation.Layer` is read from `IfcPresentationLayerAssignment`.
The adapter looks for layer assignments on:
- the object's shape representations;
- their items;
- the representations that `IfcMappedItem`s map in, which is how a type's shared geometry reaches its occurrences.

The value lists every distinct layer, sorted by name (`["A-AXIS", "A-WALL"]`). An object without a shape, or with no assigned layer, has none. A model
without any `IfcPresentationLayerAssignment` records no layers: every request is answered `NotRecorded`, never absent, so a layer rule is reported not
applicable to that source. The locator names the object and each assignment, in the order of the layers (`layer:#4:#82,#80`).

### Surface transparency

`axioval:presentation.Transparency` is read from the presentation styles of the object's body through `ifc-style` (0.3.0). Only body representations count (`Body`, `Body-FallBack`, or no identifier), and within them each representation item:

- takes the styles of its own `IfcStyledItem`, or, without one, those of the `IfcPresentationLayerWithStyle`s it is on. This is the cascade of `ifc-style`'s `resolve_item_style`, applied over an index built once per model, and a test pins the two to agree;
- takes, when an `IfcMappedItem` has no surface style of its own, the styles of the items of the representation it maps in, which is how a type's shared geometry reaches its occurrences;
- is drawn with the styles of the object's material when it has no surface style of its own: each `IfcMaterial` of the material that applies (as for `axioval:material`) carries them in its `IfcMaterialDefinitionRepresentation`.

The item's own style is authoritative, as IFC defines it: a material style never overrides a styled item, it only draws the items nothing else styles. Only `IfcSurfaceStyle`s count; a curve, fill-area or text style says nothing about a surface. A surface style's transparency is the `Transparency` of its `IfcSurfaceStyleShading` or `IfcSurfaceStyleRendering`; unset, it is `0.0` (opaque), the value the schema states. IFC2X3 shading has no transparency and is opaque; its rendering has one, and its styles are unwrapped from `IfcPresentationStyleAssignment`.

The value lists every distinct transparency, ascending (`[0.2, 0.7]` for tinted glazing in a tinted frame), so a selector states whether `any` or `all` surfaces must satisfy a comparison; several styles are never averaged or picked from. An object without a body, or whose body has no styled surface and whose material carries no style, has none (an exact absence). The locator names the object and the contributing surface styles, those of the material after `material:` (`transparency:#5:material:#56`).

Refused rather than guessed:
- a surface style without a shading element (only textures, lighting, refraction, or an externally defined style): it states no transparency;
- two `IfcStyledItem`s on one item: a conflict.

An object whose styled glazing sits in an unstyled frame and has no styled material lists only the glazing: an unstyled item states nothing, and is not assumed opaque.

### Materials

`axioval:material` is read from `IfcRelAssociatesMaterial` through
`ifc-material` (0.3.0), in the release the file declares (IFC2X3 or IFC4).
An association on the object wins; without one, the
association of its `IfcRelDefinesByType` type object applies, so a wall
without its own material takes the layer set of its wall type. The adapter
reads:
- an `IfcMaterial`: its name and category;
- an `IfcMaterialLayerSet`, directly or through an
  `IfcMaterialLayerSetUsage`: its name, each layer's material, thickness,
  name and category, and the total thickness (`IfcMlsTotalThickness`);
- an `IfcMaterialConstituentSet`: each constituent's material, name,
  category and fraction;
- an `IfcMaterialProfileSet`, directly or through an
  `IfcMaterialProfileSetUsage`: each profile's material, name and category;
- an `IfcMaterialList`: each material's name and category.

`Names` collects every non-empty name and category among these, and each
member material's name and category, as IDS matches a material value.

Thicknesses and fractions are converted from the project's units as
described under *Measures*, or refused. A unit the file does not resolve
refuses that measure alone: the names and members are exact without it. The locator names the object, where
the association came from, the association and the entity that holds the
value: `material:#1:type:#40:#41:#32/#20` is layer `#32`'s material `#20`,
associated by `#41` with type `#40`; a usage adds `usage:#36`. An object
with no association, directly or through its type, has no material.

- Two associations on the object, or on its type, and two type objects are
  conflicts.
- A malformed material, a lone layer, constituent or profile associated
  directly, and an `IfcMaterialProfileSetUsageTapering` (two profile sets)
  are refused.
- An attribute the file's release does not declare cannot be stated, so its
  property is absent: an IFC2X3 `IfcMaterial` has no category, and an IFC2X3
  `IfcMaterialLayer` no name or category. IFC2X3 has no constituent or
  profile sets.

### Bodies

`axioval:body` is read from the product's body representation (`Body`,
else `Facetation`, else one without an identifier, never an `Axis` or
`FootPrint`) through `ifc-geometry`'s `body_description` (0.4.0,
openbimrs/ifc#147), the same reader the geometry lowering is built from, so
the facts and the meshed body cannot disagree. Each geometric item is one
entry, `IfcMappedItem`s resolved: a column placed from its type's shared
geometry reads as if its extrusion were authored in place, and `Mapped`
says it was not. For every swept-area solid the adapter states its profile
(`IfcProfileDef` families from rectangle to Z shape, with the IFC attribute
names as dimension names), the solid's placement in world coordinates, and
for an `IfcExtrudedAreaSolid` its depth and world direction. An
`IfcArbitraryClosedProfileDef` and an `IfcArbitraryProfileDefWithVoids`
state their outline and voids as vertices through `ifc-geometry`'s
`profile_outline` (0.4.1, openbimrs/ifc#166), from `IfcPolyline` and
line-only `IfcIndexedPolyCurve` boundaries in the exact length unit. An
`IfcArcIndex` segment or any other curve family refuses the outline facts
alone (`Unavailable`), never chorded; the rest of the profile stands.

Lengths go through the project's exact length unit and angles through its
plane-angle unit (`ifc_properties::exact_unit`), never through
`ifc_geometry::units::resolve`, which assumes metres and radians when a
file states none. A length unit that does not resolve refuses the whole
body; a plane-angle unit that does not resolve refuses only the angles
the file states in it (profile slopes, a revolution's angle). `PositionAngle`
and `Extrusion.Inclination` come from direction vectors and need no unit. The locator names the object, the representation, the mapped
items followed and the item (`body:#30:#37:#35>#33`), and for a profile
fact the profile entity (`body:#10:#19:#18:profile:#14`).

- An object that is no `IfcProduct`, or has no body representation, has
  no body: every name is absent.
- An item family or profile family the reader does not describe, an open
  profile swept as a solid, a dangling reference, a mapping that scales or
  mirrors a swept solid, and a placement it cannot compose refuse the whole
  body.
- An optional attribute the file leaves unset is absent even where the
  schema defines a default; the adapter reports what the file states.

### Object frames

The IFC session registers an object-frame service (`ObjectFrameServiceHandle`; see [typed host services](./services.md)) without geometry. An `IfcProduct`'s frame is its `ObjectPlacement`: the `IfcLocalPlacement` chain is composed by `ifc-geometry`'s placement resolver, the same composition that places meshed bodies, so frames and geometry agree. The slot is read from the file's release, so IFC2X3 and IFC4 are both covered.

- Each link's `IfcAxis2Placement3D` or `IfcAxis2Placement2D` gives the axes. An omitted `Axis` or `RefDirection` takes the schema default, and a `RefDirection` not perpendicular to `Axis` is projected onto the plane normal to it (`IfcBuildAxes`). Local X, Y and Z become right, forward and up.
- The composed origin is converted from the project length unit to metres through `ifc_properties::exact_unit`. A project whose length unit cannot be resolved exactly is refused, never read as metres.
- IFC placements cannot mirror: `IfcAxis2Placement3D` derives Y as Z × X, so every frame is right-handed. A component mirrored in plan by placing it with `Axis` (0,0,-1) is reported as that rotation, with up pointing down and forward along -Y. Mirroring stated in a representation (an `IfcCartesianTransformationOperator` of a mapped item) is not part of the placement and does not change the frame.
- IFC does not state a product's front. The placement's Y axis is an authoring convention, not a statement of which side a component is used from, so every IFC frame reports `ObjectFront::NotStated`.
- A product without an `ObjectPlacement`, and an object that is not a product, is `NotPlaced`. `IfcGridPlacement`, at the object or anywhere in its chain, is refused as unsupported. Cyclic, over-deep or otherwise malformed chains and parallel `Axis` and `RefDirection` are refused as unreadable.

The locator names the object and its placement chain from the object upwards: `placement:#33:#32<#24<#12`.

Door leaves come from `openbim-ifc`'s `door_operation` (0.7.0, openbimrs/ifc#148), which joins the door's placement, its `OperationType` (the occurrence's or its type's; both stated must agree) and its `IfcDoorPanelProperties` (the occurrence's own sets, else its type's), and cites the IFC4 documentation for each convention: leaves lie on the placement's x axis, partitioning `OverallWidth` by `PanelWidth`, and a swinging leaf opens towards local +y. The adapter depends on the facade with `geometry-select` and `properties` only, so no codec and no geometry kernel come with it; the derivation lives there because it joins two sibling crates.

- `SINGLE_SWING_*`, `DOUBLE_DOOR_SINGLE_SWING`, `DOUBLE_SWING_*`, `DOUBLE_DOOR_DOUBLE_SWING`, `SWING_FIXED_*`, the sliding operations and `ROLLINGUP` are placed; the hinge side is the operation's side seen from above, and the other side for a placement turned over (`Axis` (0,0,-1)).
- `NOTDEFINED`, `USERDEFINED`, revolving, folding and lifting operations, the two `DOUBLE_DOOR_SINGLE_SWING_OPPOSITE_*` operations (which do not say which leaf opens towards +y), panels contradicting the operation and conflicting operation types are refused (`Refused`); a missing operation type, `OverallWidth` or panel set is `NotStated`.
- The lining thickness is `IfcDoorLiningProperties.LiningThickness` and each leaf's depth its panel set's `PanelDepth`, through the exact project length unit; the occurrence's own lining set governs, else its type's, and several are refused. The panel's offset across the wall (`LiningOffset`, `LiningToPanelOffsetX`) is not applied: IFC defines it only by figures that disagree.
- The locator names the door, the operation and the sets: `door-operation:#20:SINGLE_SWING_LEFT:panels=#30:lining=#40`. Derivations are cached per door.

Window panels are leaves too, from `openbim-ifc`'s `window_operation` (0.7.2, openbimrs/ifc#170), the window counterpart of `door_operation`: it joins the window's placement, its partitioning (the occurrence's `PartitioningType` or its `IfcWindowType`'s, both stated must agree; IFC2X3 `IfcWindowStyle.OperationType`), its `IfcWindowPanelProperties` and, for a split window, the mullion and transom offsets of its `IfcWindowLiningProperties`. Panels tile the placement's XZ plane and open towards local +y. An object `door_operation` calls no door is tried as a window; one that is neither is `NotADoor`.

- Single, double and triple partitionings are placed. A side-hung panel is a `Swing` leaf, a tilt-and-turn panel a `TiltAndTurn` leaf, a top- or bottom-hung one a `Tilt` leaf, a sliding one `Slide` along its width or height, and `Removable` and `Fixed` panels neither swing nor tilt. Each leaf carries the panel's height; its swing sector lies in the plane of its bottom edge, its tilt sector in the vertical plane through the low end of its top or bottom hinge. The hinge side is the operation's hand seen from above, and the other side for a placement turned over.
- `NOTDEFINED` and `USERDEFINED` partitionings, pivot, `OTHEROPERATION` and `NOTDEFINED` panels, panels contradicting the partitioning, conflicting partitionings and a split whose offset lies outside the window are refused (`Refused`); a missing partitioning, `OverallWidth`, `OverallHeight`, panel set, lining set or offset a split needs is `NotStated`.
- The lining thickness is `IfcWindowLiningProperties.LiningThickness`, chosen as for doors, and each leaf's depth its panel set's `FrameDepth`. Lining, mullion and transom thicknesses are not applied to the panels.
- The locator names the window, the partitioning and the sets: `window-operation:#20:DOUBLE_PANEL_VERTICAL:panels=#30,#31:lining=#40`.

### Coordinate system

The IFC session registers a coordinate-system service (`CoordinateSystemServiceHandle`) without geometry. It reads the model context: the one root `IfcGeometricRepresentationContext` whose `ContextType` is `Model`.

- `WorldCoordinateSystem` is resolved by `ifc-geometry`'s axis-placement reader, the reader that places bodies, and its origin is converted through the exact project length unit, never read as metres.
- `TrueNorth` gives the plan direction of north.
- An IFC4 `IfcMapConversion` whose `SourceCRS` is the model context gives the map conversion: `TargetCRS.Name`, `Eastings`, `Northings`, `OrthogonalHeight`, and `XAxisAbscissa`/`XAxisOrdinate` and `Scale`, which take the schema's stated defaults (no rotation, scale 1) when unset. The offset is in the target's `MapUnit`, resolved exactly through `ifc_properties::exact_unit`; an unstated or unresolvable map unit is reported unknown.
- A file without a model context states no coordinate system. Several model contexts, several conversions of the one context, or conversions only from something else are refused. IFC2X3 has no `IfcMapConversion`, so its files state no map conversion; property-set conventions for georeferencing IFC2X3 are not read.

### Integrity warnings

Besides relationship ends, the integrity scan reports two schema cardinality
violations as warnings: an element contained by more than one spatial
structure (`spatial.contained-twice`), and a zone grouping something other
than zones, spaces and spatial zones (`zone.member-not-spatial`). Both come
from `ifc-systems`; rules still see every containment the file states.

### GlobalId aliases

Object identity stays the STEP instance (`#42`), which is unique in a file but
renumbered by every export. Each object also carries its `IfcRoot.GlobalId` as
an external id in the `ifc-globalid` scheme (`IFC_GLOBAL_ID`), the identity
issue exchange and model comparison need.

The alias is attached only when a consumer can trust it. A GlobalId that is
unset, not 22 characters of the IFC alphabet, or has a leading digit above `3`
(which does not fit a 128-bit UUID and would collide with another id) is
reported as `identity.invalid-global-id`. A GlobalId claimed by more than one
`IfcRoot` instance, relationships and type objects included, is reported once
as `identity.duplicate-global-id` and attached to none of its claimants. Both
are warnings: the object stays checkable and only loses its alias.

## Axiolid

`axioval-axiolid` supplies geometry evidence for any source capable of exposing Axiolid-compatible geometry handles. A proprietary CAD adapter can use it directly without importing OpenBIM or IFC.

A host states one of three things about each object: its mesh (`with_mesh`, or
`with_tessellated_mesh` for curved parts), that it has **no body**
(`with_no_body`, e.g. a storey or zone), or that its body is **unmeasured**
(`with_unmeasured`: it exists but could not be meshed). The last two differ on
purpose. A bodiless object is skipped wherever every other object is a
candidate obstacle. An unmeasured one has an unknown extent, so contact
refuses while one of the request's candidates is unmeasured, space
measurements refuse while one exists that could affect them, and
free-space checks refuse it as an obstacle. Treating an unmeasured slab as
absent would make a wall above it look unsupported, exactly.

`AxiolidPlanAreaService` measures plan footprints and footprint overlaps
through the same plan overlay. A planar mesh measures exactly. A tessellated
mesh with chord deviation `d` and footprint perimeter `P` measures within
`2·P·d + π·d²`, the area of the band where the true and meshed boundaries can
differ.

An uncovered area is the footprint less its overlap with the union of the
cover's footprints grown by a radius `r`. A disc has no exact polygon, so each
projected cover triangle is grown by a regular 16-gon inside the disc and by
one around it (the convex hull of the triangle's vertices moved to the
polygon's vertices): the true area lies between the two results. Both
polygons reach exactly `r` along the plan axes, so only the rounded corners
of a grown cover can differ, and they do not where the corner lies outside
the element. A tessellated cover with deviation `d` is bracketed between
growth by `r − d` and `r + d`; when `r < d` the inner growth is the measured
cover widened by its band. The element's own band widens both ends. The
evidence is exact exactly when the two ends coincide. A cover member that is
unmeasured, undescribed or a group that cannot give a footprint refuses the
measurement, never zero. The 0.3.0 overlay's region dilation is not used: it
does not state on which side of the true offset its polygon lies.

Coverage (`measure_coverage`) brackets every source's effect area between an
inner region, inside the true one, and an outer region holding it. A grown
effect is the source's footprint dilated by the range with a stated side
(`Region::dilate_inner` and `dilate_outer`, axiolid-overlay 0.3.2 or later). A visible
effect is the exact visibility polygon of the source's footprint centre in
the free region (`Region::visibility_polygon`), cut to a regular 64-gon
inscribed in the range's disc or circumscribing it. A travel effect cuts the
free region into convex cells and judges each by the shortest-path distance
at its centroid from the kernel's distance map (`axiolid-route`
`distance_map`, `DistanceMap::nearest`): within a convex cell of the free
region the distance is 1-Lipschitz, so a cell lies within range when the
centroid's distance plus the cell's radius does, and out of range when the
distance less the radius exceeds it; other cells are split, up to seven
times and 4096 queries per effect, and the rest count towards the outer
bound only. The free region is the subject's footprint less the blockers'
footprints: all of them for inner bounds, the certain ones for outer bounds.
A request's connections join the free region first: the connected spaces'
and passages' footprints (a bodiless opening's through the void the host
registered with `with_opening_void`), the certain ones for inner bounds and
all for outer bounds. Travel is then walked over the joined region but its
cells are judged only on the subject's part of it. A connection that is
tessellated, unmeasured or has neither body nor void is left out, and the
covered area's upper bound stays at the whole footprint.
A centre outside the free region reaches none of it; one on its boundary is
unmeasured. A travel effect is the union of the cells in range. Effects are
regions, united and clipped to the footprint by region operations; the
overlay takes its own output as an operand from axiolid-overlay 0.3.3 on. Only exact meshes are measured: a tessellated
subject or blocker refuses the request, while a tessellated or unmeasured
source, or a bodiless one that has no centre to travel or look from, leaves
its own effect unmeasured, which keeps the upper bound at the whole
footprint. A bodiless source grows nothing.

A bodiless group, such as a zone that makes up a fire compartment, has no
mesh of its own. Membership is a semantic fact, so the host declares it:
`with_group(group, members)`, or `with_undecided_group(group, reason)` when it
could not decide it. Both also declare the group bodiless. The group's
footprint is the union of its members' footprints, with overlapping members
counted once; a member may itself be a group. A tessellated member makes the
footprint approximate, measured with the largest member deviation. The
footprint is unavailable, never zero, when a member has no body, is
unmeasured or undescribed, when the membership is undecided, when the group
groups nothing, or when it contains itself.

`AxiolidPlanSpanService` measures plan spans over the footprints the
plan-area service gathers, a declared group spanning the union of its
members. Every point of a footprint lies in the convex hull of its projected
vertices, and the distance is convex, so the longest diagonal is the largest
distance between two hull vertices and the farthest span the largest between
a hull vertex of each footprint; no boundary is walked. A centre is the
centroid of the plan overlay, overlapping parts counted once. A tessellated
mesh with chord deviation `d` widens a diagonal by `2d` and a farthest span by
both deviations. Its centroid can move only through the band of area
`b = 2·P·d + π·d²` where the true and meshed footprints differ, whose points
lie within `R + d` of the measured centroid (`R` the farthest hull vertex), so
by at most `b·(R + d) / (A − b)`; a footprint no larger than its band has no
bounded centre and is refused. A located centre lies inside (or outside)
the footprint when it lies inside (or outside) the measured one farther from
every boundary edge than its own radius plus the chord deviation, and is
undecided otherwise, on the boundary of an exact footprint included.

The service's least-area rectangle is the overlay's rotating calipers over
every plan vertex of the footprint (`minimum_area_rectangle`, axiolid-overlay
0.3.2 or later). The choice of orientation is exact, and ties between
orientations are counted; only the output is rounded, by at most the
kernel's stated error for the centre, every corner and every half extent,
which fixes each axis within `asin(e / (L − e))` for a longer half extent
`L`. A rectangle along the coordinate axes turns nothing and is taken from
the extreme coordinates, exact whenever their differences and sums are. A
tessellated footprint widens every half extent and the centre by its chord
deviation and has an unproven orientation. This is the adapter's one
rectangle: sections and the shelf layout below use the same computation.

Recesses are the pockets between a footprint's outer boundary and its convex
hull. On a simple counter-clockwise ring the ring vertices on the hull's
boundary follow the hull in order, and between two consecutive ones any
further vertices bound a pocket; its mouth joins the two, its depth is the
farthest pocket vertex from the mouth's line. Pockets shallower than 1e-6 m
are the overlay's grid rounding (axiolid/kernel#173), not recesses; holes are
not walked, and a tessellated footprint refuses. A section is the overlay
intersection of the footprints, its sides those of its least-area rectangle,
bounded as above and stated only when the orientation is unique: a section
whose least-area orientation is tied may have other sides and is refused. A
tessellated member refuses too, since the short side of a least-area
rectangle does not grow monotonically with the shape.

`AxiolidPlanAreaService` measures the area outside bands with the same
overlay: each band is the convex hull of its two footprints' vertices,
clipped to the positions along the band's direction that both reach. The
clip lines are computed, so each is moved 0.1 µm (more for large
coordinates) inwards for the surely covered part and outwards for the
possibly covered one, which also keeps a cut clear of the overlay's
tolerance around hull vertices; the area outside lies between the two. A
band bounded by a tessellated footprint is refused.

`AxiolidLinearQuantityService` lays shelving bands on a space's footprint
less the clearance of each requested door or opening: its footprint grown by
the rule's clearance plus its plan gap to the space, bracketed between a
circumscribed 16-gon (for the lower bound) and an inscribed one (for the
upper). A bodiless opening is placed through the void the host gives
(`with_opening_void`); without one, or with a door more than 1 m from the
space, the measurement refuses. Bands run along both axes of the footprint's
and every door's least-area rectangle, anchored at either wall, and the
longest layout counts; an axis only proposes a layout, so a tied or unproven
orientation proposes its axes too. Each band is found exactly between
breakpoints (a boundary vertex in its strip, an edge crossing its long
sides); the lower bound tests cross-sections grown by a margin (1e-6 m plus
the chord deviation) and shrinks the runs, the upper bound the reverse, so
the interval holds the layout's length on the true geometry. 1 mm of each
cross-section may touch the boundary: without that construction tolerance a
band against a wall rounded a hair off the layout's axis would be one point
long. The clear height is the mesh's vertical span, widened by twice the
chord deviation.
The same service finds corridor ends with the region skeleton of
`axiolid-route` 0.3.3 (`skeleton`, prune factor 1.5, which drops the spurs
into right-angled and sharper corners). Vertices on the line through their
neighbours are dropped first, so a straight wall is one edge. The boundary
spacing starts at a tenth of the footprint's narrower bounding side and is
refined to a tenth of the narrowest width the skeleton finds until it lies
below an eighth of it; a footprint needing more than 50 000 samples is
refused. Skeleton nodes lie inside the footprint and their clearances are
certified; the axis itself is approximate. An end's wall is decided only
when the path stops at it (the end lies no farther from it than its
clearance and two spacings) and rays from the end within 20° either side
of the path's direction, taken one clearance back along it, all meet the
kernel's wall first; otherwise it is undecided. A tessellated space is
refused: its edges are chords of a curved wall. Each subject's gap and
facing length are measured on its own footprint, exactly for a planar mesh;
a tessellated subject widens the gap by its chord deviation `d` and the
facing by `2d`.

`AxiolidVerticalExtentService` measures the lowest and highest points of a
mesh. A planar mesh measures exactly; a tessellated mesh with chord deviation
`d` reports each elevation as `[z - d, z + d]` with approximate evidence, even
for a declared zero deviation. Bodiless and unmeasured objects are refused.
The same service projects the used positions onto any direction for a
directional extent. Along a coordinate axis a planar mesh measures exactly;
along any other direction each projection is a rounded dot product, widened
by a bound on its rounding and reported as approximate. A tessellation
widens further by its chord deviation.

`AxiolidWalkingSurfaceService` measures stair flights, ramps and headroom. Only a mesh that is closed and faces outward is measured, since whether a face looks up is read from its winding. A flight's treads are the planes plane detection finds (`axiolid_inspect::detect_planes`, axiolid-inspect 0.3.2, axiolid/kernel#131) facing up and level: each plane's certified deviation and its corners' spread in elevation stay within 64 units in the last place of the coordinates (the rounding a placement transform leaves) plus twice a tessellation's chord deviation, and a tread is measured at the interval of its corners' elevations. Planes at one elevation make one tread. A tessellated flight is measured too, every position widened by its chord deviation and never exact. The walking direction comes from the treads, never the placement: in plan from the centre of the lowest tread's bounding rectangle to the highest's. When every tread's centre lies within 1 µm (plus twice the chord deviation) of that line and further along it than the one below, the flight is straight; otherwise it turns (winders, a quarter turn) and is walked along a polyline with a vertex on each tread, midway across it or at the requested distance from the side the winders turn towards, its positions arc lengths along it: floating-point constructions widened by a relative margin over the sine of the angle at which the line crosses a tread's edge, never exact, and a grazing crossing refuses. A tread's nosing is the boundary edge the walking line climbs onto it across; the riser below it is open where the lower tread's back edge meets a face falling away, closed where every part of it meets a face rising, and the first riser closed only where faces in its nosing's vertical plane cover the strip below it to the base, otherwise not measured. A flight must be one piece, since its lowest point is where its first riser starts; one in several pieces (separate treads) is refused, and so is a flight with a sloped face flatter than 45° looking up. A ramp is measured only from an exact planar mesh, a tessellated one refused. A ramp's runs are the connected sets of upward sloped faces flatter than 45°, each planar within 10⁻⁹ of the coordinates' magnitude (two slopes meeting without a landing are refused), climbing along the plane's steepest ascent. A direction along a coordinate axis projects exactly; any other is widened by a bound on the projection's rounding and reported as approximate. Headroom clips each walking face against each face of a requested obstacle in plan and takes the least height difference over the regions they share, widened by a numerical margin, so it is never exact. A bodiless obstacle is skipped; an unmeasured one refuses, and so does a tessellated one within the walking surface's plan box or an obstacle whose faces lie both below and above the walking surface (it crosses it).

A tread's or run's sides are reported only when its faces fill a rectangle along its walking direction: every edge on the boundary of the faces (matched between faces by exact coordinates) lies on a side of the rectangle their positions span, and their plan area is the rectangle's. Along a coordinate axis that is exact; along any other direction positions and sides are widened by the projection's rounding. A tessellated tread's edges may lie within twice its chord deviation of a side, its area fall short by that band, and its sides widen by three times the deviation. A turning flight's tread walks square to its nosing, so a tread with parallel nosing and back edge states its sides across that direction; a winder tapers, fills no rectangle and states none, so a flight with winders has no width. A turning flight's end is placed along its end tread's own direction, square to its nosing, with the tread's rectangle in that frame giving the arrival line and the width; an end on a winder, which fills no rectangle, is refused. Consecutive treads whose nosings are parallel within their uncertainty share one frame. A landing is sought at an end's elevation (within the level tolerance) among the requested candidates' upward level faces, and a ramp's own, never a flight's: a connected set of them must cover part of the plan strip from where the end's walking surface stops to where its body ends, grown by `LANDING_REACH` (1 µm), across the end's width, with positive area. Several such surfaces are refused rather than merged; one is measured when it fills a rectangle along the leaving direction, and otherwise only found. A bodiless candidate is skipped, an unmeasured one refuses, and so does a tessellated one near the end. The clearance below clips each downward-facing level face of a requested space (its floor) against each downward-facing face of the subject in plan, drops the parts where a face of the subject lies flush on the floor, and takes the least height over the rest, widened by a numerical margin and never exact; a subject crossing the floor refuses.

Handrails are measured in the frame of the flight or run: the pitch line is the polyline through the treads' nosings (and the upper floor's edge where the flight ends in a riser) or the line of a run's surface. A requested rail counts when its box, grown by its chord deviation, reaches the plan strip over the pitch line within the reach across, lies no higher than the reach above the pitch line and not wholly below it; a bodiless rail is skipped and an unmeasured one refuses. Its upward-facing faces must fill one rectangle along the walking direction, as for a tread's width, or the request refuses: the rail bends, stands at an angle or carries parts outside its line. Between the positions of its corners and the pitch line's knots, the top of its body is the upper envelope of the lines its edges trace, so its least height above the pitch line lies at an end of such a piece or where two lines cross, and its greatest at an end. Heights carry a numerical margin scaled by the coordinates and the steepest line; a tessellated rail is accepted and widened by its chord deviation in every position and height. The rise over an extension is the spread of the top beyond an end over the requested length, evaluated without rounding where every line is level, so a level extension of an exact rail rises by exactly zero.

A turning flight's handrails are measured in its straight parts, its runs of consecutive treads sharing one frame; it must start and end on such treads. A rail whose box comes within the reach of a part's sides and the flight's extent along it must fill a rectangle along exactly one such part's direction, longer along it than across, on one side of its middle; otherwise the request refuses. A rail near no part but within the reach of the flight's plan box refuses too, since it might be a piece beside a winder zone; only one farther away is skipped. Its pitch line is the polyline through the ends, on its side, of the nosings lying on the part's side line (within the nosing's uncertainty and 10⁻⁹ of the coordinates), from the part's treads outwards as far as they go; ends at one position merge into one point spanning their elevations, and ends running back refuse. Before and after those points, where the side turns away, the pitch line's height lies between the last point's elevation and the next tread's, so the rail's heights there are bounds over both, measured no farther than the flight reaches. Rises are measured only beyond the flight's own ends. A tessellated flight's parts climb along directions its chords turn, so a rail straight along the true side fills no rectangle along them and is refused.

`AxiolidTriangleCountService` counts the triangles of the registered mesh,
which is the host's tessellation: exact evidence for a planar mesh,
approximate for a tessellated one, zero for a bodiless object, refused for an
unmeasured one or a mesh whose indices or positions cannot be read. Like the
directional extent, each count cites the counted object's own source, so one
geometry set can hold several files.

`AxiolidBoundaryCoverageService` measures space-boundary coverage from boundary
surfaces the host registers per space (`with_space`, then `with_boundary`,
`with_tessellated_boundary` or `with_unmeasured_boundary`, keyed by space and
boundary, in the coordinates of the space's body). The body's faces are
grouped by oriented plane; each boundary triangle counts on the nearest face
plane all its corners lie within the request's tolerance of, and a boundary
with a triangle on no face plane is reported off the surface and covers
nothing. Within each plane the faces and each boundary's part are projected
into the plane and unioned as overlay regions: the uncovered area is the
faces less the boundaries' union, measured directly, and the covered area
the rest of the faces; the overlap accumulates what each boundary shares
with the union so far, so a part three boundaries cover counts once. A plane
along a coordinate axis projects by dropping that coordinate, without
rounding; any other plane widens every area by a band of the projection's
rounding and the overlay's grid snapping (axiolid/kernel#173) along the
regions' boundaries, and a tessellated boundary by its chord deviation.
Evidence is exact only when nothing widened and every boundary corner lies
exactly on its plane. A curved (tessellated), unmeasured or bodiless space
and an unmeasured boundary refuse.

`AxiolidFacadeAreaService` measures facade areas. The host declares the
spaces (`with_space`); the CLI declares every `IfcSpace`. A triangle counts
when its normal lies within 45° of horizontal and, from each of four sample
points (the centroids of its midpoint sub-triangles), a probe 1 µm outside it
lies in no other body or space (so a wall end against a neighbour, a frame in
its reveal, or a face flush with a room does not count) and a horizontal ray
meets, within 1 m, neither the object itself (a reveal) nor a declared space
first (an inner face). Meeting nothing, or another body first, is outside: a
facade across a courtyard counts. A triangle whose samples disagree is partly
covered and widens the interval by its area with approximate evidence. A
tessellated subject widens by `2·P·d + π·d²` over the facade triangles'
perimeter `P`. A tessellated body within reach, an unmeasured body anywhere,
or a declared space without a body refuses. Walls from IFC are meshed net of
their openings, so a wall's facade excludes its windows, and a window's
facade is its own outer face.

`AxiolidDerivedRelationshipService` derives the relationships of the
[derived-relationship service](./services.md) from the same meshes. The host
declares which objects are spaces (`with_space`) and which are doors,
windows or openings (`with_opening`); an opening the geometry declares
bodiless is probed through a void shape given with `with_opening_void`
(`with_tessellated_opening_void`, `with_unmeasured_opening_void`). Spaces must
be closed solids, since containment is a winding-number test.

- **contained-in-space.** The reference point is the centre of the element's
  mesh extent. It lies in every space whose winding number there reaches one
  half. Otherwise the nearest space counts when the offset to its nearest
  surface point is within both tolerances; two spaces equally near (within
  1e-9 m) refuse. A reference point within 1e-9 m of a space's surface
  refuses. A bodiless element occupies no place and lies in no space.
- **adjacent-space.** The direction through the opening is the narrowest
  width of its plan footprint's convex hull (rotating calipers); a footprint
  as wide one way as the other refuses. From each face, at mid-height, a probe
  sweeps outward `reach` metres, starting 1 µm past the face so a face flush
  with a room boundary does not start on it. The side's spaces are those
  entered first; a space holding both starting points encloses the opening (a
  gross-area space) and is on neither side. Each edge's evidence records the
  side as `side=+(nx,ny)` or `side=-(nx,ny)` along the canonical normal, and a
  side that enters nothing records `outside`.
- **overlapping-group-space.** A space belongs to every declared space with a
  strictly larger footprint that covers at least the ratio of its own and is
  vertically within the tolerance. Footprints and overlaps come from the plan
  overlay the plan-area service uses.

Every derivation compares exact planar bodies: a tessellated subject refuses,
as does a tessellated space whose enclosing extent comes within the
derivation's reach of the question. A curved space elsewhere blocks nothing.
Answers are cached per derivation and subject, so counting per space measures
each component once.

`AxiolidSightService` answers lines of sight with the kernel's certified
`line_of_sight` (axiolid-inspect 0.3.1). A visible answer is one ray, checked
with exact predicates, that crosses a target triangle before any blocker
triangle. A hidden answer covers every ray to the target by single blocker
pieces: a triangle, two coplanar triangles of one mesh forming a convex
quadrilateral (a wall face), or a whole closed convex blocker (a column). A
target covered only where two separate blocker meshes meet stays undecided.
The distance to the target is the least distance from the eye to its
triangles, widened by a rounding bound (and a tessellated target's chord
deviation), so a target is left unlooked-at only when surely beyond the
range. A blocker whose box lies apart from the box holding the eye and the
target cannot meet any segment between them and is not handed over. A
tessellated target in range or tessellated blocker that may matter refuses,
since its mesh is not its shape; an unmeasured blocker refuses; a bodiless
blocker is skipped and a bodiless target refused.

`AxiolidProximityService` measures pairwise proximity for clash and distance checks. Hosts register curved parts with `with_tessellated_mesh` and a chord deviation, and measurements involving them are approximate. See [Clash, interference and distance](./clash.md).

## ICDD

`axioval-icdd` opens an ICDD package, dispatches member payloads to registered source decoders, maps linksets, and assembles a project. ICDD serialization types do not cross into the engine IR.

## Alternate geometry kernels

An OpenCascade or CGAL adapter may implement the same evidence traits in an external crate. Native/FFI code is never enabled by the default pure-Rust distribution.

## Conformance

Every source adapter must prove source-qualified identity, deterministic enumeration, provenance and strict malformed-data behavior. Every geometry adapter must prove exactness reporting, backend-failure propagation, transform/unit handling and cache isolation.
