# `axioval-engine`

Trusted capability registration, strict package binding, typed host services, execution plans, and deterministic runtime orchestration.

Packages are untrusted data. Unknown definitions, capability/signature drift, unknown parameters, missing required values, and type mismatches fail compilation. A `table` parameter's columns are part of the signature: the definition must declare the descriptor's columns by ID, kind and requirement, and every bound or defaulted row must fit them (no unknown column, no cell of another kind, no missing required cell), or compilation fails. Capability execution returns `CapabilityEvaluation`; missing services or unusable evidence belong in typed not-evaluated outcomes, never an empty-findings false pass. Runtime attaches the compiled rule identity and sorts report outcomes deterministically.

`properties.rs` owns exact source-neutral property requests, request-bound present values and absence proofs, response validation, and the typed property-resolution service handle. Every conclusive response must bind the complete request, including the source-qualified object identity; matching property names alone are insufficient. Missing object-map entries are not evidence of absence. Source interpretation and completeness proof stay in providers.

`relationships.rs` owns exact comparison-candidate requests and request-bound complete selections. The request binds the checked object, canonical candidate universe, and semantic traversal or shared-group query. Providers interpret native relationship structures and must return exact reviewable completeness evidence. Capabilities must not treat the legacy `Object.relationships` map as authoritative.

`derived_relationships.rs` owns relationships derived from geometry: the `axioval:derived.*` identities with their tolerances, the derived-relationship handle, and the routing the session installs over the relationship handle. The handle refuses evidence that does not name its derivation. Derivation algorithms stay in geometry adapters; an unknown derivation or parameter is an invalid request, never an empty answer.

`topology.rs` owns deterministic source-neutral connectivity and route queries over exact typed evidence. It must not infer edges from source relationships or import geometry, IFC, ICDD, Axiolid, or vendor types.

`metric_routing.rs` owns canonical-metre requests, conservative distance bounds, three-valued threshold comparison, and the backend-neutral service handle. Algorithms and native geometry types stay in Axiolid or another geometry provider. A blocked result requires complete exact evidence; a known route through incomplete topology proves existence only.

`free_space.rs` owns source-neutral metric frames, clearance shapes, area bounds, constrained placement searches, and their typed service handle. Keep proof asymmetry explicit: obstruction and placement may use exact witnesses; clear and no-placement require complete exact coverage. Grounded support and frame-offset search predicates belong in requests; every supported found witness also needs exact frame-bound whole-base support evidence. Rasterization, collision, CSG, and search algorithms stay outside the engine.

`session.rs` owns immutable project/source snapshots and the session-authoritative service registry. Every session service must implement `SnapshotBoundService`; reject unbound, duplicate, unknown, or non-identical source/revision/fingerprint/schema bindings before registration.

`walkability.rs` owns complete source-neutral walkable-region snapshots and deterministic three-valued width-constrained routes. Derived region IDs are evidence-local, not model objects. Reject unrequested object mappings, relation-only portals, duplicate passages, incomplete coverage, and backend geometry types.

`proximity.rs` owns pairwise proximity contracts: extents with geometry fidelity, and separation, plan overlap, witnessed penetration and containment per pair. Evidence exactness must equal fidelity; a tessellation is never exact. Penetration is a lower bound and `None` only when neither body is a closed solid, never zero by default. No clash verdict crosses this seam.

`vertical_extent.rs` owns bottom and top elevation intervals per object. Evidence is exact exactly when both are points, and the handle refuses an extent naming another object. No stacking or spacing verdict crosses this seam.

`pairwise.rs` owns the broad-phase candidate search. It must stay complete: discard only by the gap between enclosing boxes (mesh extent grown by chord deviation), and keep it proven against the exhaustive search in its tests.
