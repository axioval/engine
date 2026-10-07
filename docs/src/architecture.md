# Architecture

## Workspace layout

Crates are grouped by the role they play, never by the dependency they wrap.
The directory says what a crate may do; the name says what it contains.

```text
crates/
  contracts/ir                 source-neutral vocabulary
  engine/{core,rules}          capability execution
  sources/semantic/ifc         one adapter per format
  sources/geometry/axiolid     one adapter per kernel
  sources/assembly/icdd        one adapter per container
  sinks/{bcf,bcf-api,bcf-snapshot,html,xlsx}
                               one writer per exchange or report format
  packages/export              the export framework every target shares
  packages/ids                 one importer per rule format
  codecs/java-stream           one standalone codec per wire format
  facade/axioval               feature-gated re-exports
  apps/cli                     executables
```

Adding a geometry backend (CGAL, manifold3d) means a new sibling under
`sources/geometry/` and a feature on the facade. Nothing in `contracts/` or
`engine/` changes, because neither may name a source.

## Dependency direction

```text
contracts/ir
    ^
engine/core  <-  engine/rules
    ^                 ^
    |                 |
    +- sources/semantic/ifc
    +- sources/geometry/axiolid
    +- sources/assembly/icdd
              ^
        facade/axioval

contracts/ir  <-  sinks/{bcf, html, xlsx}  <-  sinks/{bcf-api, bcf-snapshot}

contracts/ir  <-  packages/export

contracts/ir, engine/core, engine/rules, packages/export  <-  packages/ids

codecs/java-stream           (depends on nothing in this tree)
```

A host composes adapters. The CLI, for instance, meshes IFC bodies with
`ifc-geometry` and registers them with the Axiolid services under the IFC
session's object identities, which no adapter may do because neither may
depend on the other.

A sink reads finished reports. It depends on the IR alone (the BCF API
client and the snapshot renderer also on the BCF sink), so any host can
write BCF, HTML or a spreadsheet from any source's report without linking
the engine or an adapter.

A package importer writes definition packages and rulesets from another rule
format. It reads the engine's capability descriptors so its definitions match
them, and never a model, so it links no source adapter.

An exporter writes packages back as another format through an
[export profile](./export.md). The profile trait, the loss kinds and the
rule comparator live in `axioval-export`, which depends on the IR alone and
names no format; each target implements the profile, in this workspace
(`axioval-ids`) or in a host's own crate. A difference that would change a
check's result is always a refused loss, never a degraded one.

A codec reads and writes one wire format, byte for byte, and depends on no
crate of this tree. It knows the format's grammar, not the classes or
records an application stores in it; the application that owns those binds
them on top ([Java serialization streams](./java-stream.md)).

`axioval-ir`, `axioval-export`, `axioval-engine`, and `axioval-rules` may not import source formats, federation containers, geometry kernels, or vendor types. Adapters depend inward; core never depends outward.

## Layers of a rule

A rule is data. What it is built from is layered, each layer reading only
the ones below it, and only the bottom layer touches geometry:

```text
rulesets         rule instances: a definition, parameter values, severity,
                 applicability; compiled into one ordered plan
    |
rules            a built-in capability (most of them templates) or an
                 `expression` rule; both are run by the same evaluator
    |
judges,          generic judges (range, comparison, uniqueness, facets,
expressions,     requirements tables, items of a list, ...), expressions
selectors        over intervals with three-valued truth, and selectors
    |            deciding which objects a rule applies to
    |
measured values  registered names (`slope`, `door_clear_width`, `steps`,
and stated       `route_verdicts`, ...) answered by trusted providers, and
properties       the properties a source states; each an interval with
    |            evidence and exactness
    |
host services    typed, source-neutral requests (`VerticalExtentService`,
                 `ProximityService`, `WalkingSurfaceService`, ...),
                 answered by the adapters a host registers
```

- **Measurement in measured values.** A quantity read from geometry is a
  registered measured value or member list
  ([Derived properties](./derived.md#measured-values)), answered by a
  `MeasuredProvider` in the rules crate that reuses the capability's own
  code. The registry types every name and its parameters, so compilation
  checks them.
- **Decision in expressions and generic judges.** Comparing a value with
  a limit, a band or a table row is an [expression](./expressions.md) or a
  shared judge, decided through one comparison module
  (`axioval_engine::comparison`). Values are intervals; a straddling
  comparison is not evaluated, never passed; `null` (stated absent) and
  not evaluated (could not be read) never mix.
- **Searches stay in Rust.** Routes, pairings, free-floor fits, sight lines
  and allocations are algorithms no expression states. Each stays built-in
  code and states its answer as a measured list, one three-valued item per
  searched object, which a template or an expression judges.
- **Built-in capabilities are templates.** A capability keeps its id and
  parameter descriptor, the outside contract packages, the catalogue and
  export profiles address, and is defined inside as a composition of the
  layers below ([Capability templates](./templates.md),
  [ADR 0005](./adr-0005.md)). An author can [compose](./composing-rules.md)
  the same parts into an `expression` rule, or fork a template into one.

The [architecture gate](./contributing.md#layering-a-new-check) holds the
layering call site by call site, and the
[parity harness](./parity.md) and the [benchmark](./performance.md) hold
every template to the implementation it replaced.

## Execution

1. A normalized package is deserialized under a deny-unknown-fields contract.
2. The compiler binds selectors and parameters to trusted capability
   descriptors, resolves measured names against the registry, and type and
   unit checks every expression and derived value.
3. Compilation produces an immutable ordered execution plan. Several rulesets
   compile into one plan with `compile_rulesets`: each on its own terms, its
   rule IDs qualified by its package ID (`package-id/rule-id`).
4. An `EvidenceSession` binds the project to one immutable revision,
   fingerprint, and optional schema per declared source. Every trusted evidence
   service must expose its own source snapshots; registration rejects unbound,
   duplicate, unknown, or non-identical revision/fingerprint/schema bindings.
5. A run selects that session-bound `ProjectView` and negotiates required evidence capabilities.
6. Each rule runs. A template's rule is bound once into a flat plan (its
   parameters folded into constants; the capability keeps the plans it
   bound), its values read in
   batches through the run's `MeasuredValues`, measurements memoized for
   the run in a `MeasuredMemo`, and selections shared between the rules of
   the run; nothing measured in one run answers another
   ([Template performance](./performance.md)). Other capabilities evaluate
   semantic facts and typed evidence directly.
7. Findings retain rule, source, object, evidence, precision and diagnostic
   provenance; an expression's findings carry an explanation of the path
   that decided them.
8. Stable ordering produces reproducible reports.

## Project model

A project is a collection of source contributions and links. A view is an immutable interpretation of that collection: one source, a raw federation, or a composed/layered result. The engine does not prescribe how the view was serialized.

Identity is always source-qualified. External IDs such as IFC GlobalId are aliases and never universal primary keys.

## Trust boundary

Rule packages contain data, not code. Capability IDs resolve only through an application-created trusted registry. Packages cannot load dynamic libraries, issue network requests, choose file paths, or instantiate arbitrary Rust types. Expressions and templates are data the engine evaluates: the expression
language is total (no loops, recursion or user-defined functions), bounded
in size, and every run shares an evaluation budget. Measured values are
answered only by providers the host registered.

## Geometry boundary

Rules request semantic evidence such as footprints, bounds, intersections, routes or clearances. The evidence provider owns backend handles and reports exactness/provenance. Axiolid is the default adapter but is not part of the public engine IR.

## Failure model

Unavailable, unsupported, invalid, budget-exceeded and backend-failure are distinct from false. A rule may only pass when its required evidence is available at the declared exactness.
