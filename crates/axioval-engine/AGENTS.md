# `axioval-engine`

Trusted capability registration, strict package binding, typed host services, execution plans, and deterministic runtime orchestration.

Packages are untrusted data. Unknown definitions, capability/signature drift, unknown parameters, missing required values, and type mismatches fail compilation. Capability execution returns `CapabilityEvaluation`; missing services or unusable evidence belong in typed not-evaluated outcomes, never an empty-findings false pass. Runtime attaches the compiled rule identity and sorts report outcomes deterministically.

`properties.rs` owns exact source-neutral property requests, request-bound present values and absence proofs, response validation, and the typed property-resolution service handle. Every conclusive response must bind the complete request, including the source-qualified object identity; matching property names alone are insufficient. Missing object-map entries are not evidence of absence. Source interpretation and completeness proof stay in providers.

`topology.rs` owns deterministic source-neutral connectivity and route queries over exact typed evidence. It must not infer edges from source relationships or import geometry, IFC, ICDD, Axiolid, or vendor types.

`metric_routing.rs` owns canonical-metre requests, conservative distance bounds, three-valued threshold comparison, and the backend-neutral service handle. Algorithms and native geometry types stay in Axiolid or another geometry provider. A blocked result requires complete exact evidence; a known route through incomplete topology proves existence only.

`free_space.rs` owns source-neutral metric frames, clearance shapes, area bounds, constrained placement searches, and their typed service handle. Keep proof asymmetry explicit: obstruction and placement may use exact witnesses; clear and no-placement require complete exact coverage. Grounded support and frame-offset search predicates belong in requests; every supported found witness also needs exact frame-bound whole-base support evidence. Rasterization, collision, CSG, and search algorithms stay outside the engine.

`walkability.rs` owns complete source-neutral walkable-region snapshots and deterministic three-valued width-constrained routes. Derived region IDs are evidence-local, not model objects. Reject unrequested object mappings, relation-only portals, duplicate passages, incomplete coverage, and backend geometry types.
