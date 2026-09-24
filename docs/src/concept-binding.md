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
   ADD2 TC1. IFC4 and IFC4.3 are different type systems.
3. **Binding.** At run time the engine registers `ConceptBindings` for the
   session. Selection and property resolution translate each concept through
   the one external name whose type system the object's source declared.
4. **Failing closed.** Binding never passes a concept through verbatim.
   Each of these is reported as not evaluated (`InvalidDeclaration`), never
   as a non-match or an absence:
   - the source declares no type system;
   - the concept has no name in any declared type system;
   - the concept has distinct names in two declared type systems.

## Subtypes

`includeSubtypes` is honoured through `TypeHierarchyServiceHandle`. An object
whose kind equals the bound name always matches. Any other kind needs the
source's own hierarchy; without it, membership is unknown and reported as not
evaluated. An unknown entity name is an error, never a proven non-member.

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
