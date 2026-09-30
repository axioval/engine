# Concept binding

A rule package never names a source's own vocabulary. It names **canonical
concepts** (`axioval:fire.ifc4.wall`, `axioval:fire.ifc4.fire-rating`) and
declares, for each concept, the name it has in one or more external **type
systems**. A source adapter, in turn, declares which type system its data is
written in. Evaluation joins the two per source, and only there.

## Why this exists

Before binding existed, the engine compared concept IDs verbatim with source
data. A wall rule selected objects whose kind was literally
`axioval:fire.ifc4.wall`; an IFC model's walls are kinded `IFCWALL`. Nothing
matched, the selector selected nothing, and the report came back empty: no
findings and no not-evaluated outcomes. A caller reads that as a pass.

## The contract

1. **Catalog.** The compiler collects every object-type, property and
   property-set concept from the definition packages the ruleset declares.
   A concept declared twice, or referenced but declared nowhere, is a compile
   error (`EngineError::UnknownConcept`).
2. **Declared type systems.** A source adapter declares the type systems its
   snapshot uses with `SourceSnapshot::with_type_system`. The identity is a
   release-bound semantic URI, for example
   `https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4` for IFC4
   ADD2 TC1, and `https://standards.buildingsmart.org/IFC/RELEASE/IFC2x3/TC1/HTML/`
   for IFC2x3 TC1 (the identity `openbim.ifc` assigns; bSDD publishes no IFC2x3
   dictionary). IFC2X3, IFC4 and IFC4.3 are different type systems, so a package
   needs a name in each release it should run against.
3. **Binding.** At run time the engine registers `ConceptBindings` for the
   session. Selection and property resolution translate each concept through
   the one external name whose type system the object's source declared.
4. **Failing closed.** Binding never passes a concept through verbatim.
   Each of these is reported as not evaluated (`UnboundConcept`), never as a
   non-match or an absence:
   - the source declares no type system;
   - the concept has no name in any declared type system;
   - the concept has distinct names in two declared type systems.

   These depend on the package and the source, never on one object, so the
   runtime reports each once per rule, source and cause. The outcome names no
   object; its message gives the source, how many objects it affected and a
   few of them. A model of 2,040 objects checked against a package with no
   name for its release yields one outcome, not 2,040.

## Attribute sets

The reserved sets `axioval:attributes`, `axioval:type-attributes`,
`axioval:presentation`, `axioval:material` and `axioval:body` (see
[IR](./ir.md#attribute-sets)) are not package concepts and bind to
themselves in every source. The property inside them is an ordinary
property concept: a package declares `space-number` with the IFC4 name
`Name` and references it in `axioval:attributes`, `layer-thickness` with
the IFC4 name `TotalThickness` and references it in `axioval:material`, or
`section-depth` with the IFC4 name `Profile.OverallDepth` and references it
in `axioval:body`. Capabilities that read these sets themselves (the
presentation layer in `clash`, the body in `allowed-profile`,
`opening-zone`, `opening-area` and `empty-host`) ask by the engine's own names, never
through binding.

The derived sets (`axioval_ir::is_derived_set`) go further: the property
inside them is no concept either. In `axioval:classification` it is the id
of a classification the ruleset declares, or `<id>;level=<n>` for a level
of its class tree ([Derived properties](./derived.md)); compilation checks
it against those ids and the tree's depth, and
evaluation passes it through unchanged. In `axioval:measured` it is one of
the engine's measured names (`extent_z`, `bottom`, …).

## Name patterns

A property set or property named by a pattern (a `propertyPattern`
selector, `property-value`'s `property_pattern`, the wildcard and pattern
cells of `property-requirements`) is not a concept either: the pattern
matches the names the source states and is never bound. An exact name
beside it in the same rule binds as usual, so a concept set with a pattern
property searches the bound set only. A package whose patterns spell one
release's names (`Pset_.*Common`) should say which releases it targets, as
it does for concepts.

## Subtypes

`includeSubtypes` is honoured through `TypeHierarchyServiceHandle`. An object
whose kind equals the bound name always matches. Any other kind needs the
source's own hierarchy; without it, membership is unknown and reported as not
evaluated. An unknown entity name is an error, never a proven non-member.

An `entityType` selector of a rule's applicability is bound per source before
any rule runs, and the bound class is asked of the source's resource service
(see [Entity types and resource objects](./capabilities.md#entity-types-and-resource-objects)).
A concept a source does not bind reaches no resource object there; its objects
already report the unbound concept.

## Hosts evaluating capabilities directly

A trusted host that calls a capability without a compiled plan has no
`ConceptBindings` registered. Its rules are written in its own source
vocabulary and are used verbatim. `Runtime` always registers bindings, and
overrides any a host supplies, so package rules cannot take that path.

## Target groups

MCS rich applicability names populations by ID. A rule with one group
compiles as that group's selector. A rule with several is carried in
`ExecutionPlan::deferred` and reported as not evaluated: a one-selector
capability would otherwise have to evaluate a population the author never
named.
