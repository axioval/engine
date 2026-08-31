# Metric routing

Metric routing is a typed host service, not an algorithm embedded in the rule engine. `MetricRoutingService` may be implemented by Axiolid or another trusted geometry backend. OpenBIM may supply semantic route candidates, but it does not own geometry and no combined OpenBIM–Axiolid adapter is required.

## Canonical request

A `MetricRouteRequest` carries:

- source-qualified, object-grounded origin and destination points;
- coordinates in canonical metres;
- a validated mobility profile: radius, clear height, maximum step, and maximum slope.

NaN, infinity, and negative dimensions are rejected before backend execution. `MetricRoutingServiceHandle` validates that a backend response starts and ends at the requested points. The handle is a concrete type that can be registered in `ServiceRegistry` and consumed through `RuleContext`.

## Bounded shortest-distance evidence

`LengthInterval` stores inclusive lower and upper bounds. Exact evidence has equal bounds. Policy comparison is intentionally three-valued:

- `Satisfied`: the upper bound meets the maximum;
- `Violated`: the lower bound exceeds the maximum;
- `Indeterminate`: the interval straddles the threshold.

A known longer route plus an unavailable shortcut can therefore prove route existence, but cannot produce an exact shortest-distance verdict. Raster or tolerance-based backends may return conservative bounds rather than silently upgrading an approximation.

## Negative results

`MetricRouteOutcome::Blocked` requires `CompleteMetricEvidence`: exact, reviewable provenance that topology and relevant obstacles were complete for the query. `BlockedMetricRouteEvidence` binds that proof to the complete request, including endpoints and mobility profile; the service handle rejects evidence returned for another request. Missing geometry, partial obstacle composition, unsupported connectors, resource limits, and backend failures return `MetricRoutingError`; they never become `Blocked` or a passing rule result.

The engine contract contains no mesh, B-rep, IFC entity, Axiolid kernel, OpenCascade, or vendor type.
