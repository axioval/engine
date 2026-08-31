# `axioval-engine`

Trusted capability registration, strict package binding, typed host services, execution plans, and deterministic runtime orchestration.

Packages are untrusted data. Unknown definitions, capability/signature drift, unknown parameters, missing required values, and type mismatches fail compilation.

`topology.rs` owns deterministic source-neutral connectivity and route queries over exact typed evidence. It must not infer edges from source relationships or import geometry, IFC, ICDD, Axiolid, or vendor types.

`metric_routing.rs` owns canonical-metre requests, conservative distance bounds, three-valued threshold comparison, and the backend-neutral service handle. Algorithms and native geometry types stay in Axiolid or another geometry provider. A blocked result requires complete exact evidence; a known route through incomplete topology proves existence only.

`free_space.rs` owns source-neutral metric frames, clearance shapes, area bounds, placement searches, and their typed service handle. Keep proof asymmetry explicit: obstruction and placement may use exact witnesses; clear and no-placement require complete exact coverage. Rasterization, collision, CSG, and search algorithms stay outside the engine.

`walkability.rs` owns complete source-neutral walkable-region snapshots and deterministic three-valued width-constrained routes. Derived region IDs are evidence-local, not model objects. Reject unrequested object mappings, relation-only portals, duplicate passages, incomplete coverage, and backend geometry types.
