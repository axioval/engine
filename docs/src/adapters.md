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
is an area quantity in SI. The request is refused, never answered as absent, when:
- a complex quantity or two quantities carry the requested name;
- a property set and a quantity set share the requested set name;
- the requested name is an attribute of a predefined property set such as
  `IfcDoorLiningProperties`, which is not read.

Any other absence is exact.

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
the leaf code, or any code in the chain when `includeDescendants` is set.

IFC2X3 hierarchies are flat: a reference's `ReferencedSource` may only name
the system itself. A file that chains references anyway is refused rather
than flattened. An assignment whose system the file does not state is
neither a match nor a mismatch, and the object is reported as not evaluated.

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

### Materials

`axioval:material` is read from `IfcRelAssociatesMaterial` through
`ifc-material` (0.2.0). An association on the object wins; without one, the
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

Thicknesses are converted from the project's length unit to metres as
described under *Measures*, or refused. The locator names the object, where
the association came from, the association and the entity that holds the
value: `material:#1:type:#40:#41:#32/#20` is layer `#32`'s material `#20`,
associated by `#41` with type `#40`; a usage adds `usage:#36`. An object
with no association, directly or through its type, has no material.

- Two associations on the object, or on its type, and two type objects are
  conflicts.
- A malformed material, a lone layer, constituent or profile associated
  directly, and an `IfcMaterialProfileSetUsageTapering` (two profile sets)
  are refused.
- IFC2X3 materials are refused. `ifc-material` 0.2 reads IFC4 attribute
  positions whatever release a file declares; IFC2X3 support waits for it to
  bind to the file's release (openbimrs/ifc#77).

### Object frames

The IFC session registers an object-frame service (`ObjectFrameServiceHandle`; see [typed host services](./services.md)) without geometry. An `IfcProduct`'s frame is its `ObjectPlacement`: the `IfcLocalPlacement` chain is composed by `ifc-geometry`'s placement resolver, the same composition that places meshed bodies, so frames and geometry agree. The slot is read from the file's release, so IFC2X3 and IFC4 are both covered.

- Each link's `IfcAxis2Placement3D` or `IfcAxis2Placement2D` gives the axes. An omitted `Axis` or `RefDirection` takes the schema default, and a `RefDirection` not perpendicular to `Axis` is projected onto the plane normal to it (`IfcBuildAxes`). Local X, Y and Z become right, forward and up.
- The composed origin is converted from the project length unit to metres through `ifc_properties::exact_unit`. A project whose length unit cannot be resolved exactly is refused, never read as metres.
- IFC placements cannot mirror: `IfcAxis2Placement3D` derives Y as Z × X, so every frame is right-handed. A component mirrored in plan by placing it with `Axis` (0,0,-1) is reported as that rotation, with up pointing down and forward along -Y. Mirroring stated in a representation (an `IfcCartesianTransformationOperator` of a mapped item) is not part of the placement and does not change the frame.
- IFC does not state a product's front. The placement's Y axis is an authoring convention, not a statement of which side a component is used from, so every IFC frame reports `ObjectFront::NotStated`.
- A product without an `ObjectPlacement`, and an object that is not a product, is `NotPlaced`. `IfcGridPlacement`, at the object or anywhere in its chain, is refused as unsupported. Cyclic, over-deep or otherwise malformed chains and parallel `Axis` and `RefDirection` are refused as unreadable.

The locator names the object and its placement chain from the object upwards: `placement:#33:#32<#24<#12`.

Door and window leaves (hinge side, swing sector from `OperationType` and panel definitions) wait for upstream support (openbimrs/ifc#148).

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
bounded centre and is refused.

`AxiolidVerticalExtentService` measures the lowest and highest points of a
mesh. A planar mesh measures exactly; a tessellated mesh with chord deviation
`d` reports each elevation as `[z - d, z + d]` with approximate evidence, even
for a declared zero deviation. Bodiless and unmeasured objects are refused.
The same service projects the used positions onto any direction for a
directional extent. Along a coordinate axis a planar mesh measures exactly;
along any other direction each projection is a rounded dot product, widened
by a bound on its rounding and reported as approximate. A tessellation
widens further by its chord deviation.

`AxiolidTriangleCountService` counts the triangles of the registered mesh,
which is the host's tessellation: exact evidence for a planar mesh,
approximate for a tessellated one, zero for a bodiless object, refused for an
unmeasured one or a mesh whose indices or positions cannot be read. Like the
directional extent, each count cites the counted object's own source, so one
geometry set can hold several files.

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

`AxiolidProximityService` measures pairwise proximity for clash and distance checks. Hosts register curved parts with `with_tessellated_mesh` and a chord deviation, and measurements involving them are approximate. See [Clash, interference and distance](./clash.md).

## ICDD

`axioval-icdd` opens an ICDD package, dispatches member payloads to registered source decoders, maps linksets, and assembles a project. ICDD serialization types do not cross into the engine IR.

## Alternate geometry kernels

An OpenCascade or CGAL adapter may implement the same evidence traits in an external crate. Native/FFI code is never enabled by the default pure-Rust distribution.

## Conformance

Every source adapter must prove source-qualified identity, deterministic enumeration, provenance and strict malformed-data behavior. Every geometry adapter must prove exactness reporting, backend-failure propagation, transform/unit handling and cache isolation.
