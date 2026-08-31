# Connectivity and routes

`axioval-engine` owns the deterministic, source-neutral topology contract. It does **not** prove geometry and does not infer edges from IFC, CAD, ICDD, or vendor relationships.

## Complete graph contract

A `ConnectivityGraph` is built over an explicit universe of source-qualified `ObjectId` values. Construction requires `CompleteTopologyEvidence`: exact adapter provenance that every node and candidate transition in the requested scope was assessed. Construction fails when:

- a node identity is duplicated;
- a connection references an object outside that universe;
- an undirected connection is duplicated;
- an edge is self-referential;
- its clear width is invalid;
- edge evidence is approximate or lacks a provenance locator;
- graph-coverage evidence is incomplete or lacks a provenance locator.

This distinction is intentional: a known isolated object is unreachable, while an unknown object is a `TopologyError`. Missing input can therefore never become a clean negative result.

## Exact edges

Only trusted host code may create `VerifiedConnection` values. Each edge carries exact adapter evidence and an exact clear width in metres. Source-format relationships can nominate portal candidates, but they are not connection evidence.

A host may independently obtain:

- semantic candidates and identities from `axioval-openbim` or a proprietary CAD adapter;
- aperture, landing, and free-space proofs from `axioval-axiolid` or another geometry adapter;
- container/project assembly from `axioval-icdd`.

The host composes those results before constructing the graph. There is deliberately no `axioval-openbim-axiolid` adapter or dependency direction.

## Determinism

Reachable components are returned in source-qualified identity order. Shortest-hop routing uses deterministic breadth-first traversal; equal-length alternatives select the lexically first path. Every query applies a minimum clear-width threshold without mutating the graph.

Metric geometry paths, portal verification, collision checks, free-space construction, and vertical-connector recognition remain geometry-kernel responsibilities. The engine consumes their typed evidence rather than backend handles.
