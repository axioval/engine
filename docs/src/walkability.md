# Walkability topology

`WalkabilityService` converts a complete geometry scene into a source-neutral graph of derived walkable regions. Axiolid or another trusted geometry backend owns free-space extraction, obstacle subtraction, portal verification, and region decomposition.

The request supplies deterministic source-qualified sets of:

- walkable surface objects;
- entrance or portal objects;
- obstacle objects;
- the required clear width and optional elevation band;
- verified-portal and moving-envelope policy.

Semantic selectors run before this service. IFC placements, meshes, B-reps, and native kernel types never enter the request.

## Snapshot guarantees

A `WalkabilitySnapshot` is accepted only when it carries complete exact provenance. Every passage must have exact reviewable evidence, declared endpoints, and a conservative clear-width interval. Duplicate passages, unrequested object mappings, and portals outside the request policy are rejected.

Object-to-region membership lets reusable capabilities ask whether selected spaces, entrances, or components share traversable free space without exposing backend cells as model objects.

## Three-valued routes

For the request minimum width, the engine evaluates two deterministic graphs:

1. **definite graph:** passage lower bound meets the width;
2. **possible graph:** passage upper bound meets the width.

A route in the definite graph is `Reachable`. No route in the possible graph is `Unreachable`. A route only in the possible graph is `Indeterminate`. Approximate width evidence therefore cannot become a pass or a false negative.

Corridor metric lengths remain owned by the separate metric-routing service. End-clearance placement remains owned by the free-space placement service.
