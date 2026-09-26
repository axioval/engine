# Typed host services

The engine passes a `RuleContext` containing the immutable project view and a type-indexed `ServiceRegistry` to trusted capabilities. Applications register semantic or computational services explicitly; duplicate concrete service types are rejected instead of silently replacing an implementation.

Adapter crates are peers:

- OpenBIM implementations can provide semantic/property/relationship services.
- Axiolid implementations can provide geometry services for IFC, proprietary CAD, or any source able to lower geometry into Axiolid.
- ICDD implementations can provide project assembly and cross-document link services.
- Alternate geometry backends register the same source-neutral service interfaces.

Services reach a run through its `EvidenceSession`, bound to the source snapshots they were built from. An adapter's service records its own snapshots (`with_service`). A service the host builds itself, such as geometry meshed from the same file, records only a source identity, so the host states the snapshots it was built from (`with_host_service`). Both are checked the same way: at least one binding, no source twice, and every binding equal to the session's snapshot, so a service built from another revision is refused.

Rule packages cannot register services and cannot supply executable code. Missing required services must produce an explicit not-evaluated/backend-unavailable outcome, never a pass.

Property resolution is exposed through `PropertyResolutionServiceHandle`. A present response is a `ResolvedProperty` bound to the complete `PropertyRequest`, including the source-qualified object identity and property key; absence requires equivalent exact request-bound evidence. The handle rejects cross-object substitution, mismatched property keys, absent provenance, approximate values, non-finite numerics, and non-reviewable absence claims. Built-in property capabilities and property-based selectors require this service rather than treating a missing entry in an object map as proof of absence.

An object's own attributes (an IFC element's `Name`, `Tag`, `PredefinedType`) are exposed through `AttributeServiceHandle`. An answer is unset, a scalar with the source-declared type, or structured (a reference or non-empty aggregate: present, but nothing to compare), always with exact evidence from the object's source. An attribute the object's class does not have is an error, never an absence. The same handle answers an object's predefined type: the designation narrowing its class as the source resolves it, and whether it is user-defined; a source without the notion refuses.

Relationship-based comparison candidates are exposed through `RelationshipSelectionServiceHandle`. A request binds the checked object, the complete selector-derived candidate universe, and either a shared-group or directional traversal query. Successful responses must stay inside that universe, use canonical unique object IDs and evidence, and prove exact completeness. Source adapters interpret native containment, hosting, and relationship structures; the engine does not read `Object.relationships` as authoritative source semantics.

Metric routing is exposed as `MetricRoutingServiceHandle`. The concrete handle wraps a backend-neutral trait object so it remains type-indexable in `ServiceRegistry`. Engine capabilities consume validated metric requests and bounded evidence; adapters keep native mesh and B-rep types behind the service.

Free-space area, fixed directional clearance, and constrained placement search are exposed through `FreeSpaceServiceHandle`. The handle enforces exact request binding and preserves asymmetric proof requirements: one obstruction or placement witness may be sufficient, while clear and no-placement verdicts require complete evidence. Placement domains distinguish unrestricted, support-grounded, and anchor-frame offset searches. Supported found witnesses require exact whole-base support evidence bound to the requested object, found frame, and maximum gap. See [Free space and clearance](./free-space.md).

Walkable-region topology is exposed through `WalkabilityServiceHandle`. A trusted geometry backend supplies a complete exact snapshot; engine routing distinguishes definite-width, impossible, and uncertain-width paths without importing backend cells or geometry. See [Walkability topology](./walkability.md).

Pairwise proximity is exposed through `ProximityServiceHandle`: object extents with their geometry fidelity, and per-pair separation, plan overlap, witnessed penetration and containment. Evidence exactness must match fidelity, so a tessellation of curved faces is always approximate. The engine's `candidate_pairs` broad phase consumes the extents. See [Clash, interference and distance](./clash.md).
