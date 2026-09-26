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
loss: every `STRING`-based type (`IfcLabel`, `IfcDate`, `IfcDuration`, ...),
`INTEGER`- and `NUMBER`-based integers (`IfcTimeStamp`, `IfcCountMeasure`),
`IfcBoolean`, and `IfcReal` or dimensionless `NUMBER` reals. Other real-valued
measures stay refused until unit handling lands.

Exact absence covers what the resolver reads: `IfcPropertySet` members.
Quantity sets (`IfcElementQuantity`) and predefined property sets
(`IfcDoorLiningProperties` and its kin) are not read, so an absence is refused
as incomplete when the requested set is one of them, or, for a request that
names no set, when one of them has a member of the requested name (a
quantity, a nested quantity, or a predefined set's attribute). The index is
built once per session over the whole file; it can make an answer not
evaluated, never change a present value.

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

Several distinct layers are a conflict. An object without a shape, or with no assigned layer, has none. The locator names the object and the assignment
(`layer:#1:#80`).

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

`AxiolidPlanAreaService` measures plan footprints and footprint overlaps
through the same plan overlay. A planar mesh measures exactly. A tessellated
mesh with chord deviation `d` and footprint perimeter `P` measures within
`2·P·d + π·d²`, the area of the band where the true and meshed boundaries can
differ.

`AxiolidProximityService` measures pairwise proximity for clash and distance checks. Hosts register curved parts with `with_tessellated_mesh` and a chord deviation, and measurements involving them are approximate. See [Clash, interference and distance](./clash.md).

## ICDD

`axioval-icdd` opens an ICDD package, dispatches member payloads to registered source decoders, maps linksets, and assembles a project. ICDD serialization types do not cross into the engine IR.

## Alternate geometry kernels

An OpenCascade or CGAL adapter may implement the same evidence traits in an external crate. Native/FFI code is never enabled by the default pure-Rust distribution.

## Conformance

Every source adapter must prove source-qualified identity, deterministic enumeration, provenance and strict malformed-data behavior. Every geometry adapter must prove exactness reporting, backend-failure propagation, transform/unit handling and cache isolation.
