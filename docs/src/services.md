# Typed host services

The engine passes a `RuleContext` containing the immutable project view and a type-indexed `ServiceRegistry` to trusted capabilities. Applications register semantic or computational services explicitly; duplicate concrete service types are rejected instead of silently replacing an implementation.

Adapter crates are peers:

- OpenBIM implementations can provide semantic/property/relationship services.
- Axiolid implementations can provide geometry services for IFC, proprietary CAD, or any source able to lower geometry into Axiolid.
- ICDD implementations can provide project assembly and cross-document link services.
- Alternate geometry backends register the same source-neutral service interfaces.

Rule packages cannot register services and cannot supply executable code. Missing required services must produce an explicit not-evaluated/backend-unavailable outcome, never a pass.

Metric routing is exposed as `MetricRoutingServiceHandle`. The concrete handle wraps a backend-neutral trait object so it remains type-indexable in `ServiceRegistry`. Engine capabilities consume validated metric requests and bounded evidence; adapters keep native mesh and B-rep types behind the service.

Free-space area, fixed directional clearance, and constrained placement search are exposed through `FreeSpaceServiceHandle`. The handle enforces exact request binding and preserves asymmetric proof requirements: one obstruction or placement witness may be sufficient, while clear and no-placement verdicts require complete evidence. Placement domains distinguish unrestricted, support-grounded, and anchor-frame offset searches. Supported found witnesses require exact whole-base support evidence bound to the requested object, found frame, and maximum gap. See [Free space and clearance](./free-space.md).

Walkable-region topology is exposed through `WalkabilityServiceHandle`. A trusted geometry backend supplies a complete exact snapshot; engine routing distinguishes definite-width, impossible, and uncertain-width paths without importing backend cells or geometry. See [Walkability topology](./walkability.md).
