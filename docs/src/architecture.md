# Architecture

## Dependency direction

```text
axioval-ir
    ↑
axioval-engine ← axioval-rules
    ↑                 ↑
    ├─ axioval-openbim│
    ├─ axioval-axiolid│
    └─ axioval-icdd   │
          ↑           │
        axioval facade
```

`axioval-ir`, `axioval-engine`, and `axioval-rules` may not import source formats, federation containers, geometry kernels, or vendor types. Adapters depend inward; core never depends outward.

## Execution

1. A normalized package is deserialized under a deny-unknown-fields contract.
2. The compiler binds selectors and parameters to trusted capability descriptors.
3. Compilation produces an immutable ordered execution plan.
4. A run selects a `ProjectView` and negotiates required evidence capabilities.
5. Capabilities evaluate semantic facts and typed evidence.
6. Findings retain rule, source, object, evidence, precision and diagnostic provenance.
7. Stable ordering produces reproducible reports.

## Project model

A project is a collection of source contributions and links. A view is an immutable interpretation of that collection: one source, a raw federation, or a composed/layered result. The engine does not prescribe how the view was serialized.

Identity is always source-qualified. External IDs such as IFC GlobalId are aliases and never universal primary keys.

## Trust boundary

Rule packages contain data, not code. Capability IDs resolve only through an application-created trusted registry. Packages cannot load dynamic libraries, issue network requests, choose file paths, or instantiate arbitrary Rust types.

## Geometry boundary

Rules request semantic evidence such as footprints, bounds, intersections, routes or clearances. The evidence provider owns backend handles and reports exactness/provenance. Axiolid is the default adapter but is not part of the public engine IR.

## Failure model

Unavailable, unsupported, invalid, budget-exceeded and backend-failure are distinct from false. A rule may only pass when its required evidence is available at the declared exactness.
